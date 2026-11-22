//! Encrypted backup of the vault, or of one site or workspace: profiles,
//! workspaces and secrets.
//!
//! File layout (integers big-endian):
//!
//! | bytes | content                                   |
//! |-------|-------------------------------------------|
//! | 8     | magic `HOPRSVLT`                          |
//! | 2     | format version: `1` full, `2` partial     |
//! | 4 x 3 | Argon2id `m_cost` (KiB), `t_cost`, `p_cost` |
//! | 16    | random salt                               |
//! | 24    | random nonce                              |
//! | rest  | XChaCha20-Poly1305 ciphertext + 16-byte tag |
//!
//! The whole header (magic to nonce) is the AEAD's associated data, so
//! changing any byte of it fails authentication. The 32-byte key is derived
//! from the password with Argon2id (v0x13) and the header's parameters; the
//! parameters are checked before Argon2 runs, so a crafted file cannot make
//! it allocate more than 1 GiB or spin for long.
//!
//! The plaintext is UTF-8 JSON: `{ "sites": [...], "workspaces": [...],
//! "secrets": [{ "site": id, "kind": "password" | "key-passphrase",
//! "value": "..." }] }`. It only ever lives in zeroized buffers.
//!
//! A backup of one site or one workspace is *partial*: its header version is
//! `2` and its JSON carries `"partial": true`. A full backup has version `1`
//! and no `partial` key, laid out exactly as older releases wrote it. A
//! partial file restores with [`ImportMode::Merge`] only: either marker alone
//! refuses [`ImportMode::Replace`] (the header is authenticated, so the
//! version cannot be flipped without the password).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, AeadInOut, Generate, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hoprs_core::{Error, ErrorKind, Result, Site, SiteId, Workspace, WorkspaceId};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::atomic::write_atomic;
use crate::profiles::{self, Profiles};
use crate::{
    SecretKind, UnreadableEntry, Vault, append_detail, describe, flag, flag_mut, unknown_site,
    unknown_workspace,
};

const MAGIC: &[u8; 8] = b"HOPRSVLT";
/// Format version of a full backup.
const FORMAT_FULL: u16 = 1;
/// Format version of a partial backup (one site or one workspace).
const FORMAT_PARTIAL: u16 = 2;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = MAGIC.len() + 2 + 3 * 4 + SALT_LEN + NONCE_LEN;
const TAG_LEN: usize = 16;

/// Largest backup file accepted for import.
/// Reason given for secrets not attempted after the keyring proved unavailable.
const KEYRING_SKIPPED: &str = "keyring unavailable";

const MAX_BACKUP_BYTES: u64 = 64 * 1024 * 1024;
const KEY_LEN: usize = 32;

/// Largest accepted Argon2 memory cost: 1 GiB, in KiB.
const MAX_M_COST: u32 = 1024 * 1024;
const MAX_T_COST: u32 = 10;
const MAX_P_COST: u32 = 16;

/// Argon2id cost parameters of a backup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportParams {
    /// Memory, in KiB.
    pub m_cost: u32,
    /// Iterations.
    pub t_cost: u32,
    /// Parallelism.
    pub p_cost: u32,
}

impl Default for ExportParams {
    fn default() -> Self {
        Self {
            m_cost: 65536,
            t_cost: 3,
            p_cost: 1,
        }
    }
}

/// What an export covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportScope {
    /// Every readable site and workspace.
    All,
    /// One workspace and the readable sites it holds.
    Workspace(WorkspaceId),
    /// One site, without any workspace.
    Site(SiteId),
}

/// What an export wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportReport {
    pub sites: usize,
    pub workspaces: usize,
    pub secrets: usize,
    /// Entries this version of hoprs cannot read; they are not exported.
    pub skipped_unreadable: Vec<UnreadableEntry>,
    /// Names of sites flagged as having a secret that the store no longer
    /// holds; the backup has the site without that secret.
    pub missing_secrets: Vec<String>,
}

/// How a backup is applied to the vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportMode {
    /// Adds what is new; an id that already exists is kept and skipped.
    Merge,
    /// The vault becomes exactly the backup.
    Replace,
}

/// What an import changed. Never holds a secret value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackupImportReport {
    pub added_sites: usize,
    /// Names of the backup's sites whose id already existed (merge).
    pub skipped_sites: Vec<String>,
    pub added_workspaces: usize,
    /// Names of the backup's workspaces whose id already existed (merge).
    pub skipped_workspaces: Vec<String>,
    pub secrets_restored: usize,
    /// One line per keyring failure: site name and reason, never the value.
    pub secret_errors: Vec<String>,
}

fn unsupported_params() -> Error {
    Error::coded(
        ErrorKind::Protocol,
        "backup-unsupported-parameters",
        "Unsupported backup parameters",
    )
}

/// Checks the cost parameters against our limits and Argon2's own.
fn argon_params(p: ExportParams) -> Result<Params> {
    if p.m_cost > MAX_M_COST || p.t_cost > MAX_T_COST || p.p_cost > MAX_P_COST {
        return Err(unsupported_params().with_detail(format!(
            "m_cost {} KiB, t_cost {}, p_cost {} (limits: {MAX_M_COST}, {MAX_T_COST}, {MAX_P_COST})",
            p.m_cost, p.t_cost, p.p_cost
        )));
    }
    Params::new(p.m_cost, p.t_cost, p.p_cost, Some(KEY_LEN))
        .map_err(|e| unsupported_params().with_detail(e.to_string()))
}

struct Header {
    version: u16,
    params: ExportParams,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
}

impl Header {
    fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        let fields = [
            &MAGIC[..],
            &self.version.to_be_bytes(),
            &self.params.m_cost.to_be_bytes(),
            &self.params.t_cost.to_be_bytes(),
            &self.params.p_cost.to_be_bytes(),
            &self.salt,
            &self.nonce,
        ];
        let mut at = 0;
        for field in fields {
            out[at..at + field.len()].copy_from_slice(field);
            at += field.len();
        }
        out
    }

    /// Parses and validates a header: `prefix` is the start of a file of
    /// `file_len` bytes (up to [`HEADER_LEN`] of it). No expensive work:
    /// Argon2 only runs once this succeeds.
    fn decode(prefix: &[u8], file_len: u64) -> Result<Header> {
        if prefix.len() < MAGIC.len() || &prefix[..MAGIC.len()] != MAGIC {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "backup-not-a-backup",
                "Not a hoprs backup",
            ));
        }
        if prefix.len() < HEADER_LEN || file_len < (HEADER_LEN + TAG_LEN) as u64 {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "backup-truncated",
                "Backup file is truncated",
            )
            .with_detail(format!("{file_len} bytes")));
        }
        let header = &prefix[..HEADER_LEN];
        let u16_at = |at: usize| u16::from_be_bytes([header[at], header[at + 1]]);
        let u32_at = |at: usize| {
            u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let version = u16_at(8);
        if version != FORMAT_FULL && version != FORMAT_PARTIAL {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "backup-unsupported-version",
                "Unsupported backup version",
            )
            .with_detail(format!("version {version}")));
        }
        let params = ExportParams {
            m_cost: u32_at(10),
            t_cost: u32_at(14),
            p_cost: u32_at(18),
        };
        argon_params(params)?;
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&header[22..22 + SALT_LEN]);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&header[22 + SALT_LEN..HEADER_LEN]);
        Ok(Header {
            version,
            params,
            salt,
            nonce,
        })
    }
}

/// The AEAD keyed with Argon2id(password, salt, params).
fn cipher(password: &SecretString, header: &Header) -> Result<XChaCha20Poly1305> {
    let params = argon_params(header.params)?;
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(
            password.expose_secret().as_bytes(),
            &header.salt,
            &mut key[..],
        )
        .map_err(|e| unsupported_params().with_detail(e.to_string()))?;
    XChaCha20Poly1305::new_from_slice(&key[..]).map_err(|_| {
        Error::coded(
            ErrorKind::Protocol,
            "backup-invalid-key-length",
            "Invalid backup key length",
        )
    })
}

fn random<const N: usize>() -> Result<[u8; N]> {
    <[u8; N]>::try_generate().map_err(|e| {
        Error::coded(
            ErrorKind::Io,
            "backup-random-failed",
            "System random generator failed",
        )
        .with_detail(e.to_string())
    })
}

/// Encrypts `plaintext` into a complete backup file.
fn seal(
    plaintext: &[u8],
    password: &SecretString,
    params: ExportParams,
    version: u16,
) -> Result<Vec<u8>> {
    argon_params(params)?;
    let header = Header {
        version,
        params,
        salt: random()?,
        nonce: random()?,
    };
    let aad = header.encode();
    let ciphertext = cipher(password, &header)?
        .encrypt(
            &XNonce::from(header.nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| {
            Error::coded(
                ErrorKind::Protocol,
                "backup-cannot-encrypt",
                "Cannot encrypt the backup",
            )
        })?;
    let mut out = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    out.extend_from_slice(&aad);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Authenticates and decrypts `body` (the file minus its header) in place.
fn decrypt(
    header: &Header,
    aad: &[u8],
    mut body: Zeroizing<Vec<u8>>,
    password: &SecretString,
) -> Result<Zeroizing<Vec<u8>>> {
    cipher(password, header)?
        .decrypt_in_place(&XNonce::from(header.nonce), aad, &mut *body)
        .map_err(|_| {
            Error::coded(
                ErrorKind::Auth,
                "backup-wrong-password",
                "Wrong password or damaged file",
            )
        })?;
    Ok(body)
}

/// Authenticates and decrypts a backup held in memory.
#[cfg(test)]
fn unseal(file: &[u8], password: &SecretString) -> Result<Zeroizing<Vec<u8>>> {
    let header = Header::decode(&file[..file.len().min(HEADER_LEN)], file.len() as u64)?;
    let (aad, body) = file.split_at(HEADER_LEN);
    decrypt(&header, aad, Zeroizing::new(body.to_vec()), password)
}

/// Reads and decrypts the backup at `path`, returning its (authenticated)
/// format version too. The header is read and checked
/// first, then the size against [`MAX_BACKUP_BYTES`]; only then is the rest
/// read, into one exactly-sized buffer that is decrypted in place.
fn read_backup(path: &Path, password: &SecretString) -> Result<(u16, Zeroizing<Vec<u8>>)> {
    let mut file = fs::File::open(path)?;
    let mut prefix = [0u8; HEADER_LEN];
    let mut filled = 0;
    while filled < HEADER_LEN {
        match file.read(&mut prefix[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err.into()),
        }
    }
    let file_len = file.metadata()?.len();
    let header = Header::decode(&prefix[..filled], file_len)?;
    if file_len > MAX_BACKUP_BYTES {
        return Err(Error::coded(
            ErrorKind::Protocol,
            "backup-too-large",
            "Backup file is too large",
        )
        .with_detail(format!("{file_len} bytes (limit {MAX_BACKUP_BYTES})")));
    }
    // At most MAX_BACKUP_BYTES, so it fits in usize on every target.
    let body_len = usize::try_from(file_len).unwrap_or(usize::MAX) - HEADER_LEN;
    let mut body = Zeroizing::new(Vec::with_capacity(body_len));
    // Bounded even if the file grows meanwhile.
    file.take((MAX_BACKUP_BYTES as usize - HEADER_LEN) as u64)
        .read_to_end(&mut body)?;
    let version = header.version;
    Ok((version, decrypt(&header, &prefix, body, password)?))
}

/// The encrypted JSON document. No `Debug`: it holds secret values.
#[derive(Serialize, Deserialize)]
struct Backup {
    sites: Vec<Site>,
    workspaces: Vec<Workspace>,
    secrets: Vec<BackupSecret>,
    /// Set by an export of one site or workspace; absent from a full one.
    #[serde(default, skip_serializing_if = "is_false")]
    partial: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Serialize, Deserialize)]
struct BackupSecret {
    site: SiteId,
    kind: BackupKind,
    value: String,
}

impl Drop for BackupSecret {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum BackupKind {
    Password,
    KeyPassphrase,
}

impl From<SecretKind> for BackupKind {
    fn from(kind: SecretKind) -> Self {
        match kind {
            SecretKind::Password => Self::Password,
            SecretKind::KeyPassphrase => Self::KeyPassphrase,
        }
    }
}

impl From<BackupKind> for SecretKind {
    fn from(kind: BackupKind) -> Self {
        match kind {
            BackupKind::Password => Self::Password,
            BackupKind::KeyPassphrase => Self::KeyPassphrase,
        }
    }
}

/// Counts bytes, to size the plaintext buffer before writing into it (a
/// growing `Vec` would leave copies of the secrets behind in freed memory).
struct ByteCounter(usize);

impl Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn to_json(backup: &Backup) -> Result<Zeroizing<Vec<u8>>> {
    let cannot = |e: serde_json::Error| {
        Error::coded(
            ErrorKind::Protocol,
            "backup-cannot-build",
            "Cannot build the backup",
        )
        .with_detail(format!(
            "{:?} error at line {} column {}",
            e.classify(),
            e.line(),
            e.column()
        ))
    };
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, backup).map_err(cannot)?;
    let mut json = Zeroizing::new(Vec::with_capacity(counter.0));
    serde_json::to_writer(&mut *json, backup).map_err(cannot)?;
    Ok(json)
}

fn invalid_content(detail: impl Into<String>) -> Error {
    Error::coded(
        ErrorKind::Protocol,
        "backup-invalid-content",
        "Invalid backup content",
    )
    .with_detail(detail)
}

/// Parses the plaintext. Error details carry positions only: serde_json's
/// messages can quote the offending value, which may be a secret.
///
/// Known gap: string values containing escapes (`\"`, `\\`, `\uXXXX`...)
/// are unescaped through serde_json's internal scratch buffer, which is not
/// zeroized; a secret with such characters may leave a copy in freed memory.
/// Values without escapes are copied once, straight into `BackupSecret`.
fn from_json(json: &[u8]) -> Result<Backup> {
    serde_json::from_slice(json).map_err(|e| {
        invalid_content(format!(
            "{:?} error at line {} column {}",
            e.classify(),
            e.line(),
            e.column()
        ))
    })
}

/// Unique ids, workspaces and secrets referencing the backup's own sites.
fn validate(backup: &Backup) -> Result<()> {
    let mut sites = HashSet::new();
    for site in &backup.sites {
        if !sites.insert(&site.id) {
            return Err(invalid_content(format!("duplicate site id {}", site.id)));
        }
    }
    let mut workspaces = HashSet::new();
    for ws in &backup.workspaces {
        if !workspaces.insert(&ws.id) {
            return Err(invalid_content(format!("duplicate workspace id {}", ws.id)));
        }
        if let Some(missing) = ws.sites.iter().find(|s| !sites.contains(&s.site)) {
            return Err(invalid_content(format!(
                "workspace {} references unknown site {}",
                ws.id, missing.site
            )));
        }
    }
    let mut secrets = HashSet::new();
    for secret in &backup.secrets {
        let kind = SecretKind::from(secret.kind);
        if !sites.contains(&secret.site) {
            return Err(invalid_content(format!(
                "secret for unknown site {}",
                secret.site
            )));
        }
        if !secrets.insert((&secret.site, kind)) {
            return Err(invalid_content(format!(
                "duplicate {} for site {}",
                kind.account(),
                secret.site
            )));
        }
    }
    Ok(())
}

const KINDS: [SecretKind; 2] = [SecretKind::Password, SecretKind::KeyPassphrase];

impl Vault {
    /// Writes an encrypted backup of `scope` to `path` (atomically, `0o600`
    /// on Unix): [`Vault::export_bytes`] plus the write. If the sealing
    /// fails, nothing is written.
    ///
    /// Blocking: see [`Vault::export_bytes`].
    pub fn export(
        &self,
        path: &Path,
        scope: &ExportScope,
        password: &SecretString,
        params: ExportParams,
    ) -> Result<ExportReport> {
        let (report, file) = self.export_bytes(scope, password, params)?;
        write_atomic(path, &file)?;
        Ok(report)
    }

    /// Builds an encrypted backup of `scope` and returns it with its report.
    ///
    /// - [`ExportScope::All`]: every readable site and workspace, with all
    ///   their secrets; a full file (version 1).
    /// - [`ExportScope::Workspace`]: the workspace's members that are
    ///   readable sites (vault order) with their secrets, and that workspace
    ///   with its `sites` trimmed to them; a partial file (version 2).
    ///   Members that are unreadable sites are listed in
    ///   `skipped_unreadable`.
    /// - [`ExportScope::Site`]: that site and its secrets, no workspace; a
    ///   partial file.
    ///
    /// An unknown id is `NotFound`. Secrets are read from the store; if one
    /// cannot be read, nothing is produced. Entries this version cannot read
    /// ([`Vault::unreadable_entries`]) are not exported, and workspace
    /// references to such sites are dropped. A site flagged as having a
    /// secret the store no longer holds is exported without it and listed in
    /// `missing_secrets`.
    ///
    /// An empty `password` is refused with `Protocol`.
    ///
    /// Blocking: runs Argon2id (64 MiB of memory with the default
    /// parameters) and keyring calls; call it from `spawn_blocking`.
    pub fn export_bytes(
        &self,
        scope: &ExportScope,
        password: &SecretString,
        params: ExportParams,
    ) -> Result<(ExportReport, Vec<u8>)> {
        if password.expose_secret().is_empty() {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "backup-password-empty",
                "Backup password is empty",
            ));
        }
        argon_params(params)?;

        let exported: HashSet<&SiteId> = self.sites().iter().map(|s| &s.id).collect();
        let trimmed = |ws: &Workspace| {
            let mut ws = ws.clone();
            ws.sites.retain(|s| exported.contains(&s.site));
            ws
        };
        let (sites, workspaces, skipped_unreadable, partial): (
            Vec<&Site>,
            Vec<Workspace>,
            Vec<UnreadableEntry>,
            bool,
        ) = match scope {
            ExportScope::All => (
                self.sites().iter().collect(),
                self.workspaces().iter().map(trimmed).collect(),
                self.unreadable_entries().to_vec(),
                false,
            ),
            ExportScope::Workspace(id) => {
                let ws = self
                    .workspaces()
                    .iter()
                    .find(|w| &w.id == id)
                    .ok_or_else(|| unknown_workspace(id))?;
                let members: HashSet<&SiteId> = ws.sites.iter().map(|s| &s.site).collect();
                let skipped = ws
                    .sites
                    .iter()
                    .filter_map(|s| self.profiles.unreadable_site(&s.site.0))
                    .cloned()
                    .collect();
                (
                    self.sites()
                        .iter()
                        .filter(|s| members.contains(&s.id))
                        .collect(),
                    vec![trimmed(ws)],
                    skipped,
                    true,
                )
            }
            ExportScope::Site(id) => (
                vec![self.site(id).ok_or_else(|| unknown_site(id))?],
                Vec::new(),
                Vec::new(),
                true,
            ),
        };

        let mut missing_secrets = Vec::new();
        let mut backup = Backup {
            sites: Vec::with_capacity(sites.len()),
            workspaces,
            secrets: Vec::new(),
            partial,
        };
        for original in sites {
            let mut site = original.clone();
            for kind in KINDS {
                let value = if flag(&site, kind) {
                    self.secrets.get(&site.id, kind).map_err(|err| {
                        let mut out = Error::coded(
                            err.kind,
                            "backup-cannot-read-secret",
                            "Cannot read a secret for the backup",
                        )
                        .with_detail(format!("site {:?} ({})", site.name, site.id));
                        append_detail(&mut out, &describe(&err));
                        out
                    })?
                } else {
                    None
                };
                *flag_mut(&mut site, kind) = value.is_some();
                if let Some(value) = value {
                    backup.secrets.push(BackupSecret {
                        site: site.id.clone(),
                        kind: kind.into(),
                        value: value.expose_secret().to_owned(),
                    });
                }
            }
            let lost = KINDS.iter().any(|&k| flag(original, k) && !flag(&site, k));
            if lost {
                missing_secrets.push(site.name.clone());
            }
            backup.sites.push(site);
        }

        let report = ExportReport {
            sites: backup.sites.len(),
            workspaces: backup.workspaces.len(),
            secrets: backup.secrets.len(),
            skipped_unreadable,
            missing_secrets,
        };
        let version = if partial { FORMAT_PARTIAL } else { FORMAT_FULL };
        let json = to_json(&backup)?;
        drop(backup);
        let file = seal(&json, password, params, version)?;
        Ok((report, file))
    }

    /// Restores a backup written by [`Vault::export`].
    ///
    /// A partial file (one site or workspace) is refused with `Protocol`
    /// (`backup-partial-no-replace`) in [`ImportMode::Replace`], before
    /// anything changes.
    ///
    /// The file is authenticated, decrypted, parsed and validated before
    /// anything changes; a file over 64 MiB is refused (`Protocol`) after
    /// its header is checked and before the rest is read. Then the profiles
    /// are applied (one save per file, imported sites without secret flags;
    /// if `workspaces.toml` cannot be saved, `sites.toml` is put back), and
    /// then the secrets. A secret the store refuses does not fail the
    /// import: its site stays without the flag and the failure is listed in
    /// `secret_errors`. `sites.toml` is therefore written twice: once with
    /// the profiles, once more with the flags of the restored secrets.
    ///
    /// Blocking: runs Argon2id with the file's parameters (up to 1 GiB) and
    /// keyring calls; call it from `spawn_blocking`.
    ///
    /// - [`ImportMode::Merge`] adds the sites and workspaces whose id is new
    ///   (with their secrets) and lists the others as skipped.
    /// - [`ImportMode::Replace`] makes the vault exactly the backup: sites,
    ///   workspaces, unknown fields and unreadable entries not in it are
    ///   dropped, and every secret the backup does not carry is deleted from
    ///   the store (for sites that disappear, unreadable ones included, and
    ///   for kept ids whose backup lacks that secret). If a backup secret of
    ///   a kept id cannot be stored, the old local value of that kind is
    ///   deleted too (best effort), so it is not mistaken for the backup's.
    pub fn import_backup(
        &mut self,
        path: &Path,
        password: &SecretString,
        mode: ImportMode,
    ) -> Result<BackupImportReport> {
        let (version, json) = read_backup(path, password)?;
        let backup = from_json(&json)?;
        drop(json);
        validate(&backup)?;
        if (version == FORMAT_PARTIAL || backup.partial) && mode == ImportMode::Replace {
            return Err(Error::coded(
                ErrorKind::Protocol,
                "backup-partial-no-replace",
                "This file holds only part of a backup. Restore it with Merge.",
            ));
        }

        let names: HashMap<&SiteId, &str> = backup
            .sites
            .iter()
            .map(|s| (&s.id, s.name.as_str()))
            .collect();
        let fresh = |site: &Site| {
            let mut site = site.clone();
            site.has_password = false;
            site.has_key_passphrase = false;
            site
        };
        let mut report = BackupImportReport::default();
        let old = self.profiles.clone();
        let mut imported: HashSet<SiteId> = HashSet::new();

        let next = match mode {
            ImportMode::Merge => {
                let mut next = old.clone();
                for site in &backup.sites {
                    if self.site(&site.id).is_some() || old.is_unreadable_site_id(&site.id.0) {
                        report.skipped_sites.push(site.name.clone());
                    } else {
                        imported.insert(site.id.clone());
                        next.sites.push(fresh(site));
                    }
                }
                for ws in &backup.workspaces {
                    if self.workspaces().iter().any(|w| w.id == ws.id)
                        || old.is_unreadable_workspace_id(&ws.id.0)
                    {
                        report.skipped_workspaces.push(ws.name.clone());
                    } else {
                        next.workspaces.push(ws.clone());
                    }
                }
                next
            }
            ImportMode::Replace => {
                imported.extend(backup.sites.iter().map(|s| s.id.clone()));
                Profiles::from_items(
                    backup.sites.iter().map(fresh).collect(),
                    backup.workspaces.clone(),
                )
            }
        };
        report.added_sites = imported.len();
        report.added_workspaces = backup.workspaces.len() - report.skipped_workspaces.len();
        let replace = mode == ImportMode::Replace;
        self.commit_profiles(
            next,
            replace || report.added_sites > 0,
            replace || report.added_workspaces > 0,
        )?;

        // After the first "keyring unavailable" no more keyring calls are made:
        // each one could raise another OS unlock prompt.
        let mut keyring_down = false;
        // Deletes are tried until one of them fails as unavailable; a failed
        // write still gets its compensating delete (stale values must go).
        let mut delete_down = false;
        if replace {
            let wanted: HashSet<(&SiteId, SecretKind)> = backup
                .secrets
                .iter()
                .map(|s| (&s.site, s.kind.into()))
                .collect();
            let mut previous: Vec<(SiteId, String)> = old
                .sites
                .iter()
                .map(|s| (s.id.clone(), s.name.clone()))
                .collect();
            previous.extend(
                old.unreadable_sites()
                    .into_iter()
                    .map(|(id, name)| (SiteId(id.clone()), name.unwrap_or(id))),
            );
            for (id, name) in &previous {
                for kind in KINDS {
                    if wanted.contains(&(id, kind)) {
                        continue;
                    }
                    if keyring_down {
                        report.secret_errors.push(format!(
                            "{name}: old {} not deleted: {KEYRING_SKIPPED}",
                            kind.account()
                        ));
                        continue;
                    }
                    if let Err(err) = self.secrets.delete(id, kind) {
                        delete_down = crate::secrets::is_keyring_unavailable(&err);
                        keyring_down |= delete_down;
                        report.secret_errors.push(format!(
                            "{name}: old {} not deleted: {}",
                            kind.account(),
                            describe(&err)
                        ));
                    }
                }
            }
        }

        // Replace: ids that already had local secrets (readable or not).
        let kept: HashSet<SiteId> = if replace {
            old.sites
                .iter()
                .map(|s| s.id.clone())
                .chain(old.unreadable_sites().into_iter().map(|(id, _)| SiteId(id)))
                .collect()
        } else {
            HashSet::new()
        };
        let mut restored = Vec::new();
        for secret in &backup.secrets {
            if !imported.contains(&secret.site) {
                continue;
            }
            let kind = SecretKind::from(secret.kind);
            let name = names.get(&secret.site).copied().unwrap_or_default();
            if keyring_down {
                report.secret_errors.push(format!(
                    "{name}: {} not restored: {KEYRING_SKIPPED}",
                    kind.account()
                ));
                continue;
            }
            let value = SecretString::from(secret.value.as_str());
            match self.secrets.set(&secret.site, kind, &value) {
                Ok(()) => restored.push((secret.site.clone(), kind)),
                Err(err) => {
                    keyring_down = crate::secrets::is_keyring_unavailable(&err);
                    report.secret_errors.push(format!(
                        "{name}: {} not restored: {}",
                        kind.account(),
                        describe(&err)
                    ));
                    if kept.contains(&secret.site)
                        && !delete_down
                        && let Err(err) = self.secrets.delete(&secret.site, kind)
                    {
                        delete_down = crate::secrets::is_keyring_unavailable(&err);
                        report.secret_errors.push(format!(
                            "{name}: old {} not deleted: {}",
                            kind.account(),
                            describe(&err)
                        ));
                    }
                }
            }
        }
        if !restored.is_empty() {
            let mut next = self.profiles.clone();
            for (id, kind) in &restored {
                if let Some(site) = next.sites.iter_mut().find(|s| &s.id == id) {
                    *flag_mut(site, *kind) = true;
                }
            }
            match profiles::save_sites(&self.dir, &next) {
                Ok(()) => {
                    self.profiles = next;
                    report.secrets_restored = restored.len();
                }
                Err(err) => report.secret_errors.push(format!(
                    "{} secrets were stored but sites.toml could not record them: {}",
                    restored.len(),
                    describe(&err)
                )),
            }
        }
        Ok(report)
    }

    /// Saves the changed files of `next` (sites first) and commits it. If
    /// `workspaces.toml` fails after `sites.toml` was written, the previous
    /// `sites.toml` is written back so the vault stays as it was.
    fn commit_profiles(&mut self, next: Profiles, sites: bool, workspaces: bool) -> Result<()> {
        if sites {
            profiles::save_sites(&self.dir, &next)?;
        }
        if workspaces && let Err(mut err) = profiles::save_workspaces(&self.dir, &next) {
            if sites && let Err(rollback) = profiles::save_sites(&self.dir, &self.profiles) {
                append_detail(
                    &mut err,
                    &format!("rollback failed: {}", describe(&rollback)),
                );
                // Memory follows whatever the disk now holds.
                if let Ok(current) = profiles::load(&self.dir) {
                    self.profiles = current;
                }
            }
            return Err(err);
        }
        self.profiles = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use hoprs_core::{
        Error, ErrorKind, Protocol, SavedTab, Site, SiteId, TabKind, Workspace, WorkspaceId,
        WorkspaceSite,
    };
    use secrecy::ExposeSecret;

    use super::*;
    use crate::{MemoryStore, SecretKind, SecretStore};

    const LOW: ExportParams = ExportParams {
        m_cost: 1024,
        t_cost: 1,
        p_cost: 1,
    };

    /// Offset of the ciphertext: magic, version, 3 params, salt, nonce.
    const HEADER: usize = 8 + 2 + 12 + 16 + 24;

    fn pw(s: &str) -> SecretString {
        SecretString::from(s)
    }

    fn ssh_site(name: &str) -> Site {
        Site::new(
            name,
            format!("{}.example.com", name.to_lowercase()),
            "deploy",
            Protocol::Ssh,
        )
    }

    fn workspace(name: &str, sites: &[&SiteId]) -> Workspace {
        Workspace {
            id: WorkspaceId::new(),
            name: name.into(),
            sites: sites
                .iter()
                .map(|&id| WorkspaceSite {
                    site: id.clone(),
                    tabs: vec![],
                })
                .collect(),
        }
    }

    fn secret_of(vault: &Vault, id: &SiteId, kind: SecretKind) -> Option<String> {
        vault
            .secret(id, kind)
            .unwrap()
            .map(|s| s.expose_secret().to_owned())
    }

    fn stored(store: &dyn SecretStore, id: &SiteId, kind: SecretKind) -> Option<String> {
        store
            .get(id, kind)
            .unwrap()
            .map(|s| s.expose_secret().to_owned())
    }

    struct Source {
        _tmp: tempfile::TempDir,
        vault: Vault,
        a: Site,
        b: Site,
        ws: Workspace,
    }

    /// 2 sites, 1 workspace, 2 secrets (A's password, B's key passphrase).
    fn source() -> Source {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        let a = ssh_site("Alpha");
        let mut b = Site::new("Bravo", "ftp.bravo.example", "web", Protocol::FtpsExplicit);
        b.remote_dir = Some("/var/www".into());
        vault.upsert_site(a.clone()).unwrap();
        vault.upsert_site(b.clone()).unwrap();
        vault
            .set_secret(&a.id, SecretKind::Password, &pw("alpha-secret-1"))
            .unwrap();
        vault
            .set_secret(&b.id, SecretKind::KeyPassphrase, &pw("bravo-phrase-2"))
            .unwrap();
        let ws = workspace("Prod", &[&a.id, &b.id]);
        vault.upsert_workspace(ws.clone()).unwrap();
        let a = vault.site(&a.id).unwrap().clone();
        let b = vault.site(&b.id).unwrap().clone();
        Source {
            _tmp: tmp,
            vault,
            a,
            b,
            ws,
        }
    }

    fn export_to(vault: &Vault, dir: &Path) -> std::path::PathBuf {
        let path = dir.join("backup.hoprs");
        vault
            .export(&path, &ExportScope::All, &pw("correct horse"), LOW)
            .unwrap();
        path
    }

    /// Bytes of both profile files, to prove nothing changed.
    fn snapshot(dir: &Path) -> Vec<Option<Vec<u8>>> {
        ["sites.toml", "workspaces.toml"]
            .iter()
            .map(|f| fs::read(dir.join(f)).ok())
            .collect()
    }

    struct FailingStore;

    fn unavailable() -> Error {
        Error::new(ErrorKind::Io, "System keyring unavailable").with_detail("no D-Bus session")
    }

    impl SecretStore for FailingStore {
        fn get(&self, _: &SiteId, _: SecretKind) -> Result<Option<SecretString>> {
            Err(unavailable())
        }
        fn set(&self, _: &SiteId, _: SecretKind, _: &SecretString) -> Result<()> {
            Err(unavailable())
        }
        fn delete(&self, _: &SiteId, _: SecretKind) -> Result<()> {
            Err(unavailable())
        }
    }

    #[test]
    fn export_import_roundtrip_with_secrets() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("backup.hoprs");
        let report = src
            .vault
            .export(&path, &ExportScope::All, &pw("correct horse"), LOW)
            .unwrap();
        assert_eq!(
            report,
            ExportReport {
                sites: 2,
                workspaces: 1,
                secrets: 2,
                skipped_unreadable: vec![],
                missing_secrets: vec![],
            }
        );

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let report = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
            .unwrap();
        assert_eq!(
            report,
            BackupImportReport {
                added_sites: 2,
                added_workspaces: 1,
                secrets_restored: 2,
                ..Default::default()
            }
        );
        assert_eq!(vault.sites(), [src.a.clone(), src.b.clone()]);
        assert_eq!(vault.workspaces(), std::slice::from_ref(&src.ws));
        assert_eq!(
            secret_of(&vault, &src.a.id, SecretKind::Password).as_deref(),
            Some("alpha-secret-1")
        );
        assert_eq!(
            secret_of(&vault, &src.a.id, SecretKind::KeyPassphrase),
            None
        );
        assert_eq!(
            secret_of(&vault, &src.b.id, SecretKind::KeyPassphrase).as_deref(),
            Some("bravo-phrase-2")
        );

        let reopened = Vault::open(tmp.path(), store).unwrap();
        assert_eq!(reopened.sites(), vault.sites());
        assert_eq!(reopened.workspaces(), vault.workspaces());
    }

    #[test]
    fn wrong_password_is_auth_and_changes_nothing() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let local = ssh_site("Local");
        vault.upsert_site(local.clone()).unwrap();
        vault
            .set_secret(&local.id, SecretKind::Password, &pw("local-pw"))
            .unwrap();
        let before = snapshot(tmp.path());

        for mode in [ImportMode::Merge, ImportMode::Replace] {
            let err = vault
                .import_backup(&path, &pw("wrong horse"), mode)
                .unwrap_err();
            assert_eq!(err.kind, ErrorKind::Auth);
            assert_eq!(err.message, "Wrong password or damaged file");
            assert_eq!(snapshot(tmp.path()), before);
            assert_eq!(vault.sites().len(), 1);
            assert_eq!(
                stored(&*store, &local.id, SecretKind::Password).as_deref(),
                Some("local-pw")
            );
            assert_eq!(stored(&*store, &src.a.id, SecretKind::Password), None);
        }
    }

    #[test]
    fn tampered_file_is_auth() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());
        let original = fs::read(&path).unwrap();
        assert!(original.len() > HEADER + 16);

        // Ciphertext, tag, and every header field that keeps the file
        // parseable: params (still in range), salt, nonce.
        let offsets = [
            HEADER,
            original.len() - 1,
            8 + 2 + 3, // low byte of m_cost: 1024 -> 1025
            8 + 2 + 12,
            8 + 2 + 12 + 16 + 5,
        ];
        for offset in offsets {
            let mut bytes = original.clone();
            bytes[offset] ^= 0x01;
            fs::write(&path, &bytes).unwrap();
            let tmp = tempfile::tempdir().unwrap();
            let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
            let err = vault
                .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
                .unwrap_err();
            assert_eq!(err.kind, ErrorKind::Auth, "offset {offset}");
            assert!(vault.sites().is_empty());
            assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn unknown_magic_or_version_is_protocol() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());
        let original = fs::read(&path).unwrap();
        assert_eq!(&original[..8], b"HOPRSVLT");
        assert_eq!(&original[8..10], &[0, 1]);

        let mut magic = original.clone();
        magic[0] = b'X';
        let mut version = original.clone();
        version[9] = 3;
        let truncated = original[..HEADER - 1].to_vec();
        let no_tag = original[..HEADER + 15].to_vec();
        for (what, bytes) in [
            ("magic", magic),
            ("version", version),
            ("truncated", truncated),
            ("no tag", no_tag),
            ("empty", vec![]),
        ] {
            fs::write(&path, &bytes).unwrap();
            let tmp = tempfile::tempdir().unwrap();
            let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
            let err = vault
                .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
                .unwrap_err();
            assert_eq!(err.kind, ErrorKind::Protocol, "{what}: {err:?}");
            assert!(vault.sites().is_empty());
        }
    }

    #[test]
    fn oversized_argon_params_are_refused() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());
        let original = fs::read(&path).unwrap();

        let set = |bytes: &mut Vec<u8>, field: usize, value: u32| {
            let at = 10 + 4 * field;
            bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
        };
        let cases = [
            (0, 1024 * 1024 + 1), // just over 1 GiB
            (0, u32::MAX),
            (0, 0), // below Argon2's minimum
            (1, 11),
            (1, 0),
            (2, 17),
            (2, 0),
        ];
        for (field, value) in cases {
            let mut bytes = original.clone();
            set(&mut bytes, field, value);
            fs::write(&path, &bytes).unwrap();
            let tmp = tempfile::tempdir().unwrap();
            let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
            let err = vault
                .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
                .unwrap_err();
            assert_eq!(err.kind, ErrorKind::Protocol, "field {field} = {value}");
            assert_eq!(err.message, "Unsupported backup parameters");
        }

        // Export refuses to write a file it could not import back.
        let target = out.path().join("big.hoprs");
        let err = src
            .vault
            .export(
                &target,
                &ExportScope::All,
                &pw("correct horse"),
                ExportParams {
                    m_cost: 1024 * 1024 + 1,
                    ..LOW
                },
            )
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert!(!target.exists());
    }

    #[test]
    fn default_params_match_the_spec() {
        assert_eq!(
            ExportParams::default(),
            ExportParams {
                m_cost: 65536,
                t_cost: 3,
                p_cost: 1,
            }
        );
    }

    #[test]
    fn merge_keeps_existing_ids() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let mut mine = src.a.clone();
        mine.name = "My Alpha".into();
        vault.upsert_site(mine.clone()).unwrap();
        vault
            .set_secret(&mine.id, SecretKind::Password, &pw("mine-pw"))
            .unwrap();
        let mut my_ws = src.ws.clone();
        my_ws.name = "My Prod".into();
        my_ws.sites.truncate(1);
        vault.upsert_workspace(my_ws.clone()).unwrap();
        let other = ssh_site("Other");
        vault.upsert_site(other.clone()).unwrap();

        let report = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
            .unwrap();
        assert_eq!(report.added_sites, 1);
        assert_eq!(report.skipped_sites, ["Alpha"]);
        assert_eq!(report.added_workspaces, 0);
        assert_eq!(report.skipped_workspaces, ["Prod"]);
        assert_eq!(report.secrets_restored, 1);
        assert!(report.secret_errors.is_empty());

        let names: Vec<_> = vault.sites().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["My Alpha", "Other", "Bravo"]);
        assert_eq!(vault.workspaces(), [my_ws]);
        assert_eq!(
            secret_of(&vault, &mine.id, SecretKind::Password).as_deref(),
            Some("mine-pw")
        );
        assert_eq!(
            secret_of(&vault, &src.b.id, SecretKind::KeyPassphrase).as_deref(),
            Some("bravo-phrase-2")
        );
        let reopened = Vault::open(tmp.path(), store).unwrap();
        assert_eq!(reopened.sites(), vault.sites());
    }

    #[test]
    fn replace_overwrites() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());

        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("sites.toml"),
            "version = 1\nfuture_top = 1\n\n[[site]]\nid = \"odd\"\nname = \"Odd\"\nprotocol = \"s3\"\n",
        )
        .unwrap();
        let store = Arc::new(MemoryStore::new());
        store
            .set(&SiteId("odd".into()), SecretKind::Password, &pw("odd-pw"))
            .unwrap();
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        assert_eq!(vault.unreadable_entries().len(), 1);
        // Same id as Alpha, with a secret the backup does not have.
        let mut mine = src.a.clone();
        mine.name = "My Alpha".into();
        vault.upsert_site(mine.clone()).unwrap();
        vault
            .set_secret(&mine.id, SecretKind::KeyPassphrase, &pw("stale-phrase"))
            .unwrap();
        let gone = ssh_site("Gone");
        vault.upsert_site(gone.clone()).unwrap();
        vault
            .set_secret(&gone.id, SecretKind::Password, &pw("gone-pw"))
            .unwrap();
        vault
            .upsert_workspace(workspace("Mine", &[&gone.id]))
            .unwrap();

        let report = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Replace)
            .unwrap();
        assert_eq!(report.added_sites, 2);
        assert_eq!(report.added_workspaces, 1);
        assert!(report.skipped_sites.is_empty());
        assert_eq!(report.secrets_restored, 2);
        assert!(
            report.secret_errors.is_empty(),
            "{:?}",
            report.secret_errors
        );

        assert_eq!(vault.sites(), [src.a.clone(), src.b.clone()]);
        assert_eq!(vault.workspaces(), std::slice::from_ref(&src.ws));
        assert!(vault.unreadable_entries().is_empty());
        assert_eq!(stored(&*store, &gone.id, SecretKind::Password), None);
        assert_eq!(
            stored(&*store, &SiteId("odd".into()), SecretKind::Password),
            None
        );
        assert_eq!(stored(&*store, &mine.id, SecretKind::KeyPassphrase), None);
        assert_eq!(
            stored(&*store, &mine.id, SecretKind::Password).as_deref(),
            Some("alpha-secret-1")
        );

        let reopened = Vault::open(tmp.path(), store).unwrap();
        assert_eq!(reopened.sites(), vault.sites());
        assert_eq!(reopened.workspaces(), vault.workspaces());
        assert!(reopened.unreadable_entries().is_empty());
        let text = fs::read_to_string(tmp.path().join("sites.toml")).unwrap();
        assert!(!text.contains("future_top"), "{text}");
    }

    #[test]
    fn replace_with_keyring_down_reports_every_failure() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());

        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(FailingStore)).unwrap();
        vault.upsert_site(ssh_site("Gone")).unwrap();
        let report = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Replace)
            .unwrap();
        assert_eq!(report.added_sites, 2);
        assert_eq!(report.secrets_restored, 0);
        // Gone's two secrets could not be deleted, and the two backup
        // secrets could not be stored.
        assert_eq!(report.secret_errors.len(), 4, "{:?}", report.secret_errors);
        assert!(report.secret_errors.iter().any(|e| e.contains("Gone")));
        assert!(
            vault
                .sites()
                .iter()
                .all(|s| !s.has_password && !s.has_key_passphrase)
        );
    }

    #[test]
    fn export_has_no_plaintext() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());
        let bytes = fs::read(&path).unwrap();
        let contains = |needle: &str| bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
        for needle in [
            "Alpha",
            "alpha.example.com",
            "Bravo",
            "alpha-secret-1",
            "bravo-phrase-2",
            "Prod",
            &src.a.id.0,
            "sites",
        ] {
            assert!(!contains(needle), "{needle} found in the backup");
        }
        // Header: magic, version, params, all big-endian.
        assert_eq!(&bytes[..8], b"HOPRSVLT");
        assert_eq!(&bytes[8..10], &1u16.to_be_bytes());
        assert_eq!(&bytes[10..14], &1024u32.to_be_bytes());
        assert_eq!(&bytes[14..18], &1u32.to_be_bytes());
        assert_eq!(&bytes[18..22], &1u32.to_be_bytes());

        // Fresh salt and nonce every time.
        let again = export_to(&src.vault, &out.path().join("again"));
        let again = fs::read(again).unwrap();
        assert_ne!(&bytes[22..HEADER], &again[22..HEADER]);
    }

    #[cfg(unix)]
    #[test]
    fn export_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn keyring_failure_during_import_is_reported_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let mut source = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        let a = ssh_site("Alpha");
        source.upsert_site(a.clone()).unwrap();
        source
            .set_secret(&a.id, SecretKind::Password, &pw("alpha-secret-1"))
            .unwrap();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&source, out.path());

        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(FailingStore)).unwrap();
        let report = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
            .unwrap();
        assert_eq!(report.added_sites, 1);
        assert_eq!(report.secrets_restored, 0);
        assert_eq!(report.secret_errors.len(), 1);
        let line = &report.secret_errors[0];
        assert!(line.contains("Alpha"), "{line}");
        assert!(line.contains("System keyring unavailable"), "{line}");
        assert!(!format!("{report:?}").contains("alpha-secret-1"));
        assert!(!vault.site(&a.id).unwrap().has_password);
        let reopened = Vault::open(tmp.path(), Arc::new(FailingStore)).unwrap();
        assert!(!reopened.site(&a.id).unwrap().has_password);
    }

    /// Counts `set` calls; the backend is never reachable.
    #[derive(Default)]
    struct CountingFailingStore {
        sets: std::sync::atomic::AtomicUsize,
    }

    impl SecretStore for CountingFailingStore {
        fn get(&self, _: &SiteId, _: SecretKind) -> Result<Option<SecretString>> {
            Err(unavailable())
        }
        fn set(&self, _: &SiteId, _: SecretKind, _: &SecretString) -> Result<()> {
            self.sets.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(unavailable())
        }
        fn delete(&self, _: &SiteId, _: SecretKind) -> Result<()> {
            Err(unavailable())
        }
    }

    #[test]
    fn restore_stops_writing_secrets_once_the_keyring_is_unavailable() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(CountingFailingStore::default());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let report = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
            .unwrap();
        assert_eq!(report.added_sites, 2);
        assert_eq!(report.secrets_restored, 0);
        // One real attempt; the second secret is not tried.
        assert_eq!(store.sets.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(report.secret_errors.len(), 2, "{:?}", report.secret_errors);
        assert!(
            report.secret_errors[0].contains("System keyring unavailable"),
            "{:?}",
            report.secret_errors
        );
        assert!(
            report.secret_errors[1].contains("keyring unavailable"),
            "{:?}",
            report.secret_errors
        );
    }

    #[test]
    fn export_with_keyring_down_fails_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        let a = ssh_site("Alpha");
        vault.upsert_site(a.clone()).unwrap();
        vault
            .set_secret(&a.id, SecretKind::Password, &pw("alpha-secret-1"))
            .unwrap();
        let down = Vault::open(tmp.path(), Arc::new(FailingStore)).unwrap();
        let path = tmp.path().join("backup.hoprs");
        let err = down
            .export(&path, &ExportScope::All, &pw("correct horse"), LOW)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
        assert!(!path.exists());
    }

    #[test]
    fn unreadable_entries_are_not_exported() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("sites.toml"),
            "version = 1\n\n[[site]]\nid = \"odd\"\nname = \"Odd\"\nprotocol = \"s3\"\n",
        )
        .unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        let a = ssh_site("Alpha");
        vault.upsert_site(a.clone()).unwrap();
        let odd = SiteId("odd".into());
        let ws = workspace("Mixed", &[&odd, &a.id]);
        vault.upsert_workspace(ws.clone()).unwrap();

        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("backup.hoprs");
        let report = vault
            .export(&path, &ExportScope::All, &pw("correct horse"), LOW)
            .unwrap();
        assert_eq!(report.sites, 1);
        assert_eq!(report.workspaces, 1);
        assert_eq!(report.skipped_unreadable, vault.unreadable_entries());

        let tmp = tempfile::tempdir().unwrap();
        let mut target = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        target
            .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
            .unwrap();
        // The reference to the unexported site is dropped.
        assert_eq!(target.workspaces()[0].sites.len(), 1);
        assert_eq!(target.workspaces()[0].sites[0].site, a.id);
    }

    fn import_plaintext(json: &str) -> (tempfile::TempDir, Vault, Result<BackupImportReport>) {
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("backup.hoprs");
        let sealed = seal(json.as_bytes(), &pw("pw"), LOW, FORMAT_FULL).unwrap();
        fs::write(&path, sealed).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        let result = vault.import_backup(&path, &pw("pw"), ImportMode::Replace);
        (tmp, vault, result)
    }

    #[test]
    fn seal_and_unseal_roundtrip() {
        let sealed = seal(b"payload", &pw("pw"), LOW, FORMAT_FULL).unwrap();
        assert_eq!(&unseal(&sealed, &pw("pw")).unwrap()[..], b"payload");
        assert_eq!(
            unseal(&sealed, &pw("other")).unwrap_err().kind,
            ErrorKind::Auth
        );
    }

    #[test]
    fn invalid_content_is_protocol_without_values() {
        let site = r#"{"id":"s1","name":"S","host":"h","port":22,"user":"u","protocol":"ssh"}"#;
        let cases = [
            "not json".to_owned(),
            r#"{"sites":[],"workspaces":[],"secrets":[{"site":"s1","kind":"pin","value":"leak-me-1"}]}"#.to_owned(),
            format!(r#"{{"sites":[{site}],"workspaces":[],"secrets":[{{"site":"s1","kind":"password","value":7}}]}}"#),
            format!(r#"{{"sites":[{site},{site}],"workspaces":[],"secrets":[]}}"#),
            format!(r#"{{"sites":[{site}],"workspaces":[{{"id":"w","name":"W","sites":[{{"site":"ghost"}}]}}],"secrets":[]}}"#),
            r#"{"sites":[],"workspaces":[],"secrets":[{"site":"ghost","kind":"password","value":"leak-me-2"}]}"#.to_owned(),
            format!(r#"{{"sites":[{site}],"workspaces":[],"secrets":[{{"site":"s1","kind":"password","value":"a"}},{{"site":"s1","kind":"password","value":"b"}}]}}"#),
        ];
        for json in cases {
            let (tmp, vault, result) = import_plaintext(&json);
            let err = result.unwrap_err();
            assert_eq!(err.kind, ErrorKind::Protocol, "{json}");
            let shown = format!("{err:?}");
            assert!(!shown.contains("leak-me"), "{shown}");
            assert!(vault.sites().is_empty());
            assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn oversized_file_is_refused_before_reading_it() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());
        // Valid header, then a sparse tail past the cap: nothing is allocated
        // for it, and the import must not try to.
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_BACKUP_BYTES + 1).unwrap();
        drop(file);
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        let err = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.message, "Backup file is too large");

        // A huge file that is not a backup at all is refused by its magic.
        let other = out.path().join("huge.bin");
        let file = fs::File::create(&other).unwrap();
        file.set_len(MAX_BACKUP_BYTES + 1).unwrap();
        drop(file);
        let err = vault
            .import_backup(&other, &pw("correct horse"), ImportMode::Merge)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.message, "Not a hoprs backup");
        assert!(vault.sites().is_empty());
    }

    #[test]
    fn header_is_bound_as_associated_data() {
        let header = Header {
            version: FORMAT_FULL,
            params: LOW,
            salt: [7; SALT_LEN],
            nonce: [9; NONCE_LEN],
        };
        let aad = header.encode();
        let cipher = cipher(&pw("pw"), &header).unwrap();
        let nonce = XNonce::from(header.nonce);
        let sealed = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: b"payload",
                    aad: &aad,
                },
            )
            .unwrap();

        // Same key, same nonce: only the associated data differs.
        let mut other = aad;
        other[8 + 1] ^= 0x01;
        for wrong in [&[][..], &other[..], &aad[..HEADER_LEN - 1]] {
            let mut buffer = sealed.clone();
            assert!(cipher.decrypt_in_place(&nonce, wrong, &mut buffer).is_err());
        }
        let mut buffer = sealed.clone();
        cipher.decrypt_in_place(&nonce, &aad, &mut buffer).unwrap();
        assert_eq!(buffer, b"payload");
    }

    #[test]
    fn export_reports_missing_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let a = ssh_site("Alpha");
        let b = ssh_site("Bravo");
        vault.upsert_site(a.clone()).unwrap();
        vault.upsert_site(b.clone()).unwrap();
        vault
            .set_secret(&a.id, SecretKind::Password, &pw("alpha-secret-1"))
            .unwrap();
        vault
            .set_secret(&b.id, SecretKind::Password, &pw("bravo-secret-2"))
            .unwrap();
        // Lost behind the vault's back (e.g. removed in the keyring UI).
        store.delete(&a.id, SecretKind::Password).unwrap();

        let out = tempfile::tempdir().unwrap();
        let report = vault
            .export(
                &out.path().join("b.hoprs"),
                &ExportScope::All,
                &pw("correct horse"),
                LOW,
            )
            .unwrap();
        assert_eq!(report.secrets, 1);
        assert_eq!(report.missing_secrets, ["Alpha"]);
    }

    #[test]
    fn empty_export_password_is_refused() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("b.hoprs");
        let err = src
            .vault
            .export(&path, &ExportScope::All, &pw(""), LOW)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.message, "Backup password is empty");
        assert!(!path.exists());
    }

    /// Never stores anything; reads and deletes work.
    struct NoSetStore(MemoryStore);

    impl SecretStore for NoSetStore {
        fn get(&self, site: &SiteId, kind: SecretKind) -> Result<Option<SecretString>> {
            self.0.get(site, kind)
        }
        fn set(&self, _: &SiteId, _: SecretKind, _: &SecretString) -> Result<()> {
            Err(unavailable())
        }
        fn delete(&self, site: &SiteId, kind: SecretKind) -> Result<()> {
            self.0.delete(site, kind)
        }
    }

    #[test]
    fn replace_drops_old_secret_when_backup_one_cannot_be_stored() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = export_to(&src.vault, out.path());

        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(NoSetStore(MemoryStore::new()));
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let mut mine = src.a.clone();
        mine.name = "My Alpha".into();
        vault.upsert_site(mine.clone()).unwrap();
        store
            .0
            .set(&mine.id, SecretKind::Password, &pw("old-local-pw"))
            .unwrap();

        let report = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Replace)
            .unwrap();
        // Alpha's password and Bravo's passphrase could not be stored.
        assert_eq!(report.secret_errors.len(), 2, "{:?}", report.secret_errors);
        assert_eq!(report.secrets_restored, 0);
        // The old local value does not survive under the backup's site.
        assert_eq!(stored(&*store, &mine.id, SecretKind::Password), None);
        assert!(!vault.site(&mine.id).unwrap().has_password);
    }

    /// An empty vault holding site X with a password.
    fn target_with_x() -> (tempfile::TempDir, Arc<MemoryStore>, Vault, Site) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new());
        let mut vault = Vault::open(tmp.path(), store.clone()).unwrap();
        let x = ssh_site("Xray");
        vault.upsert_site(x.clone()).unwrap();
        vault
            .set_secret(&x.id, SecretKind::Password, &pw("x-secret"))
            .unwrap();
        (tmp, store, vault, x)
    }

    fn plaintext_of(file: &[u8]) -> String {
        String::from_utf8(unseal(file, &pw("correct horse")).unwrap().to_vec()).unwrap()
    }

    #[test]
    fn scope_all_writes_version_1_without_the_marker() {
        let src = source();
        let (report, bytes) = src
            .vault
            .export_bytes(&ExportScope::All, &pw("correct horse"), LOW)
            .unwrap();
        assert_eq!(&bytes[8..10], &[0, 1]);
        assert!(!plaintext_of(&bytes).contains("partial"));
        assert_eq!((report.sites, report.workspaces, report.secrets), (2, 1, 2));
    }

    #[test]
    fn scope_site_exports_that_site_only() {
        let src = source();
        let (report, bytes) = src
            .vault
            .export_bytes(
                &ExportScope::Site(src.a.id.clone()),
                &pw("correct horse"),
                LOW,
            )
            .unwrap();
        assert_eq!((report.sites, report.workspaces, report.secrets), (1, 0, 1));
        assert!(report.skipped_unreadable.is_empty());
        assert_eq!(&bytes[8..10], &[0, 2]);
        let json = plaintext_of(&bytes);
        assert!(json.contains("\"partial\":true"));
        assert!(json.contains("alpha-secret-1"));
        assert!(!json.contains("Bravo"));
        assert!(!json.contains("bravo-phrase-2"));

        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("one.hoprs");
        fs::write(&path, &bytes).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
            .unwrap();
        assert_eq!(vault.sites().len(), 1);
        assert!(vault.workspaces().is_empty());
        assert_eq!(
            secret_of(&vault, &src.a.id, SecretKind::Password).as_deref(),
            Some("alpha-secret-1")
        );
    }

    #[test]
    fn scope_workspace_trims_missing_and_unreadable_members() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("sites.toml"),
            "version = 1\n\n[[site]]\nid = \"odd\"\nname = \"Odd\"\nprotocol = \"s3\"\n",
        )
        .unwrap();
        let mut vault = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        let a = ssh_site("Alpha");
        let charlie = ssh_site("Charlie");
        vault.upsert_site(a.clone()).unwrap();
        vault.upsert_site(charlie.clone()).unwrap();
        let odd = SiteId("odd".into());
        let mut ws = workspace("Mixed", &[&odd, &a.id]);
        ws.sites[1].tabs.push(SavedTab {
            kind: TabKind::Files,
            remote_dir: Some("/srv".into()),
            local_dir: None,
        });
        vault.upsert_workspace(ws.clone()).unwrap();

        let (report, bytes) = vault
            .export_bytes(
                &ExportScope::Workspace(ws.id.clone()),
                &pw("correct horse"),
                LOW,
            )
            .unwrap();
        assert_eq!((report.sites, report.workspaces), (1, 1));
        assert_eq!(report.skipped_unreadable, vault.unreadable_entries());
        assert_eq!(report.skipped_unreadable.len(), 1);
        assert_eq!(&bytes[8..10], &[0, 2]);

        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("ws.hoprs");
        fs::write(&path, &bytes).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mut target = Vault::open(tmp.path(), Arc::new(MemoryStore::new())).unwrap();
        target
            .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
            .unwrap();
        let names: Vec<_> = target.sites().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Alpha"]);
        assert_eq!(target.workspaces().len(), 1);
        let restored = &target.workspaces()[0];
        assert_eq!(restored.sites.len(), 1);
        assert_eq!(restored.sites[0].site, a.id);
        assert_eq!(restored.sites[0].tabs, ws.sites[1].tabs);
    }

    #[test]
    fn scope_of_unknown_site_or_workspace_is_not_found() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = out.path().join("none.hoprs");
        let cases = [
            (
                ExportScope::Site(SiteId("nope".into())),
                "vault-unknown-site",
            ),
            (
                ExportScope::Workspace(WorkspaceId("nope".into())),
                "vault-unknown-workspace",
            ),
        ];
        for (scope, code) in cases {
            let err = src
                .vault
                .export(&path, &scope, &pw("correct horse"), LOW)
                .unwrap_err();
            assert_eq!(err.kind, ErrorKind::NotFound);
            assert_eq!(err.code, Some(code), "{scope:?}");
            assert!(!path.exists());
        }
    }

    fn partial_site_file(src: &Source, dir: &Path) -> std::path::PathBuf {
        let path = dir.join("partial.hoprs");
        src.vault
            .export(
                &path,
                &ExportScope::Site(src.a.id.clone()),
                &pw("correct horse"),
                LOW,
            )
            .unwrap();
        path
    }

    #[test]
    fn partial_file_merges() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = partial_site_file(&src, out.path());
        let (tmp, _store, mut vault, x) = target_with_x();
        let _ = &tmp;
        let report = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Merge)
            .unwrap();
        assert_eq!(report.added_sites, 1);
        let names: Vec<_> = vault.sites().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Xray", "Alpha"]);
        assert_eq!(
            secret_of(&vault, &x.id, SecretKind::Password).as_deref(),
            Some("x-secret")
        );
        assert_eq!(
            secret_of(&vault, &src.a.id, SecretKind::Password).as_deref(),
            Some("alpha-secret-1")
        );
    }

    #[test]
    fn partial_file_refuses_replace_and_changes_nothing() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let path = partial_site_file(&src, out.path());
        let (tmp, store, mut vault, x) = target_with_x();
        let before = snapshot(tmp.path());
        let err = vault
            .import_backup(&path, &pw("correct horse"), ImportMode::Replace)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert_eq!(err.code, Some("backup-partial-no-replace"));
        assert_eq!(snapshot(tmp.path()), before);
        assert_eq!(vault.sites().len(), 1);
        assert_eq!(
            stored(&*store, &x.id, SecretKind::Password).as_deref(),
            Some("x-secret")
        );
    }

    #[test]
    fn either_marker_alone_refuses_replace() {
        let with_marker = r#"{"sites":[],"workspaces":[],"secrets":[],"partial":true}"#;
        let without = r#"{"sites":[],"workspaces":[],"secrets":[]}"#;
        let out = tempfile::tempdir().unwrap();
        for (json, version) in [(with_marker, FORMAT_FULL), (without, FORMAT_PARTIAL)] {
            let path = out.path().join("marker.hoprs");
            fs::write(
                &path,
                seal(json.as_bytes(), &pw("pw"), LOW, version).unwrap(),
            )
            .unwrap();
            let (tmp, _store, mut vault, _x) = target_with_x();
            let before = snapshot(tmp.path());
            let err = vault
                .import_backup(&path, &pw("pw"), ImportMode::Replace)
                .unwrap_err();
            assert_eq!(err.code, Some("backup-partial-no-replace"), "v{version}");
            assert_eq!(snapshot(tmp.path()), before);
            vault
                .import_backup(&path, &pw("pw"), ImportMode::Merge)
                .unwrap();
        }
    }

    #[test]
    fn flipped_version_byte_fails_authentication() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        let partial = partial_site_file(&src, out.path());
        let full = export_to(&src.vault, out.path());
        for (path, flipped) in [(partial, 1u8), (full, 2u8)] {
            let mut bytes = fs::read(&path).unwrap();
            bytes[9] = flipped;
            fs::write(&path, &bytes).unwrap();
            for mode in [ImportMode::Merge, ImportMode::Replace] {
                let (tmp, store, mut vault, x) = target_with_x();
                let before = snapshot(tmp.path());
                let err = vault
                    .import_backup(&path, &pw("correct horse"), mode)
                    .unwrap_err();
                assert_eq!(
                    err.code,
                    Some("backup-wrong-password"),
                    "{flipped} {mode:?}"
                );
                assert_eq!(snapshot(tmp.path()), before);
                assert_eq!(
                    stored(&*store, &x.id, SecretKind::Password).as_deref(),
                    Some("x-secret")
                );
            }
        }
    }

    #[test]
    fn bytes_entry_point_equals_the_file_one() {
        let src = source();
        let out = tempfile::tempdir().unwrap();
        for scope in [ExportScope::All, ExportScope::Site(src.a.id.clone())] {
            let path = out.path().join("eq.hoprs");
            let report = src
                .vault
                .export(&path, &scope, &pw("correct horse"), LOW)
                .unwrap();
            let (bytes_report, bytes) = src
                .vault
                .export_bytes(&scope, &pw("correct horse"), LOW)
                .unwrap();
            assert_eq!(report, bytes_report);
            let on_disk = fs::read(&path).unwrap();
            assert_eq!(plaintext_of(&on_disk), plaintext_of(&bytes), "{scope:?}");
        }
    }
}
