//! The channel on Windows: a named pipe only this user can open.
//!
//! The name carries the SID of the process token's user (`\\.\pipe\hoprs-agent-<SID>`;
//! a directory other than the standard one adds a hash of it, see [`pipe_name`]). The
//! first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`: if any process
//! already holds the name, `bind` fails instead of joining it. Every instance carries
//! the security descriptor `O:<SID>D:P(A;;GA;;;<SID>)`: owned by the user, a protected
//! DACL with one entry that allows that user and nobody else, so no other user can open
//! the pipe nor add an instance to it. Remote clients are refused
//! (`PIPE_REJECT_REMOTE_CLIENTS`). The relay's `connect` checks that the pipe it opened
//! is owned by its own user's SID before it sends anything.
//!
//! The `unsafe` blocks are the Win32 calls (token, SID and descriptor conversions, the
//! pipe's creation with that descriptor); each says why it is sound.

use std::ffi::c_void;
use std::io::{Error, ErrorKind, Result};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::pin::Pin;
use std::ptr;
use std::sync::OnceLock;
use std::task::{Context, Poll};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_NO_DATA, ERROR_PIPE_BUSY, HANDLE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW, GetSecurityInfo, SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Every hoprs pipe name starts so.
const PREFIX: &str = r"\\.\pipe\hoprs-agent-";
/// How often, and how far apart, `connect` retries a pipe whose instances are all busy.
const BUSY_TRIES: u32 = 50;
const BUSY_PAUSE: Duration = Duration::from_millis(20);

/// Memory the system allocated for us (`LocalAlloc`), freed on drop.
struct Local(*mut c_void);

// SAFETY: the memory is owned by this value alone and is plain data (a string or a
// self-relative security descriptor); `LocalFree` may run on any thread.
unsafe impl Send for Local {}

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a system call that allocates with LocalAlloc,
            // and nothing uses it after this.
            unsafe { LocalFree(self.0) };
        }
    }
}

/// A handle we opened, closed on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by us and is closed once, here.
        unsafe { CloseHandle(self.0) };
    }
}

/// A NUL-terminated UTF-16 string the system gave us, as a `String`.
fn wide_text(text: &Local) -> String {
    let start = text.0.cast::<u16>().cast_const();
    let mut units = 0usize;
    // SAFETY: the system returned a NUL-terminated UTF-16 string at `start`, which lives
    // as long as `text`; reading up to and excluding its NUL stays inside it.
    let slice = unsafe {
        while *start.add(units) != 0 {
            units += 1;
        }
        std::slice::from_raw_parts(start, units)
    };
    String::from_utf16_lossy(slice)
}

/// `sid` as text (`S-1-5-21-…`).
fn sid_text(sid: PSID) -> Result<String> {
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: `sid` points to a valid SID (callers get it from the system and keep its
    // memory alive meanwhile); `text` is a valid out pointer.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(Error::last_os_error());
    }
    Ok(wide_text(&Local(text.cast())))
}

/// The SID of this process token's user, read from the system once.
fn user_sid() -> Result<String> {
    static SID: OnceLock<String> = OnceLock::new();
    if let Some(sid) = SID.get() {
        return Ok(sid.clone());
    }
    let sid = query_user_sid()?;
    Ok(SID.get_or_init(|| sid).clone())
}

fn query_user_sid() -> Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing; `token` is
    // a valid out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(Error::last_os_error());
    }
    let token = Handle(token);
    let mut len = 0u32;
    // SAFETY: a null buffer of length 0 only asks for the length needed.
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(Error::last_os_error());
    }
    // u64s: aligned for TOKEN_USER and its SID.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` has at least `len` writable bytes, aligned to 8.
    let ok =
        unsafe { GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) };
    if ok == 0 {
        return Err(Error::last_os_error());
    }
    // SAFETY: the call filled `buf` with a TOKEN_USER (aligned, within its length), whose
    // SID points into `buf`, alive until the end of this function.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    sid_text(sid)
}

/// The pipe of the standard directory: `\\.\pipe\hoprs-agent-<SID>`. Without a SID the
/// name still says hoprs, and `bind` fails (it needs the SID for the descriptor).
pub(super) fn standard_pipe_name() -> String {
    let sid = user_sid().unwrap_or_else(|_| "unknown".to_owned());
    format!("{PREFIX}{sid}")
}

/// The pipe of a run whose directory is `dir`: the standard name for the directories
/// of the instance lock's locations, else that name, `-` and the first 16 hex digits of
/// the SHA-256 of `dir`, so runs (and tests) in other directories never share a pipe.
pub(super) fn pipe_name(dir: &Path) -> String {
    let standard = crate::instance_lock_paths()
        .iter()
        .any(|lock| lock.parent() == Some(dir));
    if standard {
        return standard_pipe_name();
    }
    let digest = Sha256::digest(dir.as_os_str().as_encoded_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{hex}", standard_pipe_name())
}

/// The pipe's security descriptor: owned by the user, one allow entry for the user.
struct Descriptor(Local);

impl Descriptor {
    fn for_user(sid: &str) -> Result<Descriptor> {
        let sddl: Vec<u16> = format!("O:{sid}D:P(A;;GA;;;{sid})")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated and alive for the call; `descriptor` is a valid
        // out pointer; the size out pointer may be null.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(Error::last_os_error());
        }
        Ok(Descriptor(Local(descriptor)))
    }
}

/// The pipe's next instance, waiting for a relay, and what makes more.
pub(super) struct Listener {
    name: String,
    descriptor: Descriptor,
    next: NamedPipeServer,
}

impl Listener {
    pub(super) fn bind(name: &str) -> Result<Listener> {
        let descriptor = Descriptor::for_user(&user_sid()?)?;
        let next = create(name, &descriptor, true)?;
        Ok(Listener {
            name: name.to_owned(),
            descriptor,
            next,
        })
    }

    pub(super) async fn accept(&mut self) -> Result<Stream> {
        while let Err(err) = self.next.connect().await {
            // That instance is spoilt: a fresh one takes its place, made before the old
            // one closes so the name is never free.
            self.next = create(&self.name, &self.descriptor, false)?;
            // A client that left before its connection was taken: a quiet retry.
            if err.raw_os_error() != Some(ERROR_NO_DATA as i32) {
                return Err(err);
            }
        }
        // The next instance before this one is handed out, so a relay always finds one.
        let fresh = create(&self.name, &self.descriptor, false)?;
        Ok(Stream::Server(std::mem::replace(&mut self.next, fresh)))
    }
}

/// One instance of the pipe `name` with `descriptor`; `first` for the one that claims
/// the name.
fn create(name: &str, descriptor: &Descriptor, first: bool) -> Result<NamedPipeServer> {
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0.0,
        bInheritHandle: 0,
    };
    // SAFETY: `attributes` is a valid SECURITY_ATTRIBUTES whose descriptor lives as long
    // as `descriptor`, beyond this call (CreateNamedPipeW reads it during the call).
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
    }
}

/// A SID in binary form, made from its text (`S-1-…`), freed on drop.
struct BinarySid(Local);

impl BinarySid {
    fn from_text(text: &str) -> Result<BinarySid> {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sid: PSID = ptr::null_mut();
        // SAFETY: `wide` is NUL-terminated and alive for the call; `sid` is a valid out
        // pointer (the SID is allocated with LocalAlloc and freed by `Local`).
        if unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut sid) } == 0 {
            return Err(Error::last_os_error());
        }
        Ok(BinarySid(Local(sid)))
    }

    /// Whether `other` is this SID: compared as SIDs, never by their spelling (Windows
    /// writes some SIDs by an alias, such as `LA` for the built-in Administrator).
    fn is(&self, other: PSID) -> bool {
        // SAFETY: both are valid SIDs: ours lives as long as `self`, `other` is the
        // caller's (alive meanwhile).
        unsafe { EqualSid(self.0.0, other) != 0 }
    }
}

/// The user's SID in binary form.
fn user_binary_sid() -> Result<BinarySid> {
    BinarySid::from_text(&user_sid()?)
}

/// Whether the object behind `handle` is owned by this process's user.
fn owned_by_user(handle: HANDLE) -> Result<bool> {
    let mut owner: PSID = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: `handle` is an open pipe handle (opened for reading, so with READ_CONTROL);
    // the out pointers are valid; the descriptor is freed below and `owner` points into
    // it.
    let code = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if code != 0 {
        return Err(Error::from_raw_os_error(code as i32));
    }
    let _descriptor = Local(descriptor);
    Ok(user_binary_sid()?.is(owner))
}

/// The relay's side: opens the pipe and checks that this user owns it.
pub(super) async fn connect(name: &str) -> Result<Stream> {
    let mut tries = 0;
    let client = loop {
        match ClientOptions::new().open(name) {
            Ok(client) => break client,
            Err(err)
                if err.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) && tries < BUSY_TRIES =>
            {
                tries += 1;
                tokio::time::sleep(BUSY_PAUSE).await;
            }
            Err(err) => return Err(err),
        }
    };
    if !owned_by_user(client.as_raw_handle())? {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "the agent channel's pipe belongs to another user",
        ));
    }
    Ok(Stream::Client(client))
}

/// Either end of a pipe connection.
pub(super) enum Stream {
    Server(NamedPipeServer),
    Client(NamedPipeClient),
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        match self.get_mut() {
            Stream::Server(pipe) => Pin::new(pipe).poll_read(cx, buf),
            Stream::Client(pipe) => Pin::new(pipe).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        match self.get_mut() {
            Stream::Server(pipe) => Pin::new(pipe).poll_write(cx, buf),
            Stream::Client(pipe) => Pin::new(pipe).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        match self.get_mut() {
            Stream::Server(pipe) => Pin::new(pipe).poll_flush(cx),
            Stream::Client(pipe) => Pin::new(pipe).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        match self.get_mut() {
            Stream::Server(pipe) => Pin::new(pipe).poll_shutdown(cx),
            Stream::Client(pipe) => Pin::new(pipe).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, GetAce,
        GetSecurityDescriptorControl, SE_DACL_PROTECTED,
    };

    use super::*;

    const WAIT: Duration = Duration::from_secs(10);
    /// `ACCESS_ALLOWED_ACE_TYPE` (in `Win32_System_SystemServices`, a feature this crate
    /// does not need otherwise).
    const ALLOW: u8 = 0;

    /// The DACL of the object behind `handle`, read as a structure: whether it is
    /// protected, and each entry's type and SID (checked against the user's SID as a
    /// SID, not as text).
    fn dacl_of(handle: HANDLE) -> (bool, Vec<(u8, bool)>) {
        let mut dacl: *mut ACL = ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `handle` is an open pipe handle; the out pointers are valid; the
        // descriptor is freed below and `dacl` points into it.
        let code = unsafe {
            GetSecurityInfo(
                handle,
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(code, 0, "GetSecurityInfo");
        let descriptor = Local(descriptor);
        assert!(!dacl.is_null(), "a DACL");
        let (mut control, mut revision) = (0u16, 0u32);
        // SAFETY: `descriptor` is the valid descriptor read above; the out pointers are
        // valid.
        let ok = unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) };
        assert_ne!(ok, 0, "GetSecurityDescriptorControl");
        let user = user_binary_sid().unwrap();
        // SAFETY: `dacl` points to the valid ACL inside `descriptor`.
        let count = unsafe { (*dacl).AceCount };
        let mut entries = Vec::new();
        for index in 0..u32::from(count) {
            let mut ace: *mut c_void = ptr::null_mut();
            // SAFETY: `index` is below the ACL's entry count; `ace` is a valid out pointer
            // and points into the ACL afterwards.
            assert_ne!(unsafe { GetAce(dacl, index, &mut ace) }, 0, "GetAce");
            // SAFETY: every entry starts with an ACE_HEADER; an allow entry is an
            // ACCESS_ALLOWED_ACE whose SID starts at `SidStart`, inside the entry.
            let entry = unsafe {
                let kind = (*ace.cast::<ACE_HEADER>()).AceType;
                let is_user = kind == ALLOW
                    && user
                        .is(ptr::addr_of_mut!((*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart).cast());
                (kind, is_user)
            };
            entries.push(entry);
        }
        (control & SE_DACL_PROTECTED != 0, entries)
    }

    fn temp_name() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let name = pipe_name(dir.path());
        (dir, name)
    }

    #[test]
    fn pipe_name_carries_the_sid() {
        let sid = user_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "{sid}");
        let standard = standard_pipe_name();
        assert_eq!(standard, format!(r"\\.\pipe\hoprs-agent-{sid}"));
        assert!(standard.starts_with(r"\\.\pipe\hoprs-agent-S-1-"));
        let (dir, name) = temp_name();
        let suffix = name
            .strip_prefix(&format!("{standard}-"))
            .unwrap_or_else(|| panic!("{name}"));
        assert_eq!(suffix.len(), 16);
        assert!(suffix.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            pipe_name(dir.path()),
            name,
            "the same directory, the same pipe"
        );
        let lock = &crate::instance_lock_paths()[0];
        assert_eq!(pipe_name(lock.parent().unwrap()), standard);
    }

    #[tokio::test]
    async fn pipe_admits_its_owner() {
        let (_dir, name) = temp_name();
        let mut listener = Listener::bind(&name).unwrap();
        let (client, server) = tokio::join!(
            tokio::time::timeout(WAIT, connect(&name)),
            tokio::time::timeout(WAIT, listener.accept())
        );
        let (client, server) = (client.unwrap().unwrap(), server.unwrap().unwrap());
        let (mut client, mut server) = (BufReader::new(client), BufReader::new(server));
        client.get_mut().write_all(b"ping\n").await.unwrap();
        let mut line = String::new();
        tokio::time::timeout(WAIT, server.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(line, "ping\n");
        server.get_mut().write_all(b"pong\n").await.unwrap();
        line.clear();
        tokio::time::timeout(WAIT, client.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(line, "pong\n");
    }

    #[tokio::test]
    async fn second_first_instance_is_refused() {
        let (_dir, name) = temp_name();
        let _first = Listener::bind(&name).unwrap();
        assert!(Listener::bind(&name).is_err());
    }

    #[tokio::test]
    async fn descriptor_names_only_the_user() {
        let (_dir, name) = temp_name();
        let mut listener = Listener::bind(&name).unwrap();
        let (client, server) = tokio::join!(
            tokio::time::timeout(WAIT, connect(&name)),
            tokio::time::timeout(WAIT, listener.accept())
        );
        let (client, _server) = (client.unwrap().unwrap(), server.unwrap().unwrap());
        let Stream::Client(client) = &client else {
            panic!("connect gives a client");
        };
        let (protected, entries) = dacl_of(client.as_raw_handle());
        assert!(protected, "the DACL is protected (nothing inherited)");
        assert_eq!(
            entries,
            [(ALLOW, true)],
            "one entry: an allow entry for the user's SID"
        );
        assert!(
            owned_by_user(client.as_raw_handle()).unwrap(),
            "owned by the user"
        );
    }
}
