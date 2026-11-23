use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::SiteId;

/// Connection protocol of a site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    Ssh,
    Ftp,
    FtpsExplicit,
    FtpsImplicit,
}

impl Protocol {
    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Ssh => 22,
            Protocol::Ftp | Protocol::FtpsExplicit => 21,
            Protocol::FtpsImplicit => 990,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Protocol::Ssh => "ssh",
            Protocol::Ftp => "ftp (unencrypted)",
            Protocol::FtpsExplicit => "ftps (explicit)",
            Protocol::FtpsImplicit => "ftps (implicit)",
        }
    }
}

/// What an SSH site opens. Only meaningful for `Protocol::Ssh`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenAs {
    Terminal,
    Files,
    #[default]
    Both,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SshOptions {
    #[serde(default = "default_true")]
    pub use_agent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_file: Option<PathBuf>,
    #[serde(default)]
    pub open_as: OpenAs,
}

impl Default for SshOptions {
    fn default() -> Self {
        Self {
            use_agent: true,
            key_file: None,
            open_as: OpenAs::Both,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FtpMode {
    #[default]
    Passive,
    Active,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FtpOptions {
    #[serde(default)]
    pub mode: FtpMode,
    #[serde(default = "default_true")]
    pub nat_workaround: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_fingerprint: Option<String>,
}

impl Default for FtpOptions {
    fn default() -> Self {
        Self {
            mode: FtpMode::Passive,
            nat_workaround: true,
            pinned_fingerprint: None,
        }
    }
}

/// What a coding agent (over MCP) may do on a site. The user sets it by hand
/// in the site's form; it is never exported, restored or imported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentAccess {
    /// The site does not exist for agents.
    #[default]
    None,
    /// List, stat, read and download.
    Read,
    /// Everything, including writes and (SSH) commands.
    All,
}

impl AgentAccess {
    pub fn is_none(&self) -> bool {
        *self == Self::None
    }

    /// Whether this level covers `needed`.
    pub fn allows(self, needed: AgentAccess) -> bool {
        self >= needed
    }
}

/// Lenient on purpose: `"read"` and `"all"` are those levels; anything else
/// (a level a newer hoprs wrote, another type) is "No access", so the site
/// stays readable and never gains access it was not given.
impl<'de> Deserialize<'de> for AgentAccess {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};

        struct Level;
        impl<'de> Visitor<'de> for Level {
            type Value = AgentAccess;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an agent access level")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(match v {
                    "read" => AgentAccess::Read,
                    "all" => AgentAccess::All,
                    _ => AgentAccess::None,
                })
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(AgentAccess::None)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(AgentAccess::None)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(AgentAccess::None)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(AgentAccess::None)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(AgentAccess::None)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(AgentAccess::None)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(AgentAccess::None)
            }
        }
        deserializer.deserialize_any(Level)
    }
}

/// A saved site. Secrets never live here; `has_*` only flag keyring entries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Site {
    pub id: SiteId,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub protocol: Protocol,
    #[serde(default)]
    pub ssh: SshOptions,
    #[serde(default)]
    pub ftp: FtpOptions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_dir: Option<PathBuf>,
    #[serde(default)]
    pub has_password: bool,
    #[serde(default)]
    pub has_key_passphrase: bool,
    /// What agents may do here; absent from `sites.toml` means no access.
    #[serde(default, skip_serializing_if = "AgentAccess::is_none")]
    pub agent_access: AgentAccess,
}

impl Site {
    pub fn new(
        name: impl Into<String>,
        host: impl Into<String>,
        user: impl Into<String>,
        protocol: Protocol,
    ) -> Self {
        Self {
            id: SiteId::new(),
            name: name.into(),
            host: host.into(),
            port: protocol.default_port(),
            user: user.into(),
            protocol,
            ssh: SshOptions::default(),
            ftp: FtpOptions::default(),
            remote_dir: None,
            local_dir: None,
            has_password: false,
            has_key_passphrase: false,
            agent_access: AgentAccess::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Wrap {
        protocol: Protocol,
    }

    #[test]
    fn protocol_serializes_kebab_case() {
        let s = toml::to_string(&Wrap {
            protocol: Protocol::FtpsExplicit,
        })
        .unwrap();
        assert!(s.contains("\"ftps-explicit\""), "{s}");
        let back: Wrap = toml::from_str(&s).unwrap();
        assert_eq!(back.protocol, Protocol::FtpsExplicit);
    }

    #[test]
    fn default_ports_and_labels() {
        let cases = [
            (Protocol::Ssh, 22, "ssh"),
            (Protocol::Ftp, 21, "ftp (unencrypted)"),
            (Protocol::FtpsExplicit, 21, "ftps (explicit)"),
            (Protocol::FtpsImplicit, 990, "ftps (implicit)"),
        ];
        for (p, port, label) in cases {
            assert_eq!(p.default_port(), port);
            assert_eq!(p.label(), label);
        }
    }

    #[test]
    fn site_new_uses_default_port_and_options() {
        let s = Site::new("Prod", "example.com", "deploy", Protocol::FtpsImplicit);
        assert_eq!(s.port, 990);
        assert_eq!(s.name, "Prod");
        assert_eq!(s.host, "example.com");
        assert_eq!(s.user, "deploy");
        assert_eq!(s.ssh, SshOptions::default());
        assert_eq!(s.ftp, FtpOptions::default());
        assert!(s.ssh.use_agent);
        assert_eq!(s.ssh.open_as, OpenAs::Both);
        assert!(!s.has_password && !s.has_key_passphrase);
        assert!(s.remote_dir.is_none() && s.local_dir.is_none());
    }

    #[test]
    fn site_ids_are_unique() {
        let ids: HashSet<_> = (0..1000).map(|_| SiteId::new()).collect();
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn missing_optional_fields_use_defaults() {
        let s: Site = toml::from_str(
            r#"
id = "abc"
name = "n"
host = "h"
port = 22
user = "u"
protocol = "ssh"
"#,
        )
        .unwrap();
        assert_eq!(s.ssh, SshOptions::default());
        assert_eq!(s.ftp, FtpOptions::default());
        assert!(!s.has_password);
        assert!(!s.has_key_passphrase);
    }

    #[test]
    fn site_roundtrip_with_paths() {
        let mut s = Site::new("n", "h", "u", Protocol::Ssh);
        s.ssh.key_file = Some(PathBuf::from("/home/u/.ssh/id_ed25519"));
        s.local_dir = Some(PathBuf::from("work"));
        s.remote_dir = Some("/srv".into());
        s.has_password = true;
        let text = toml::to_string(&s).unwrap();
        assert_eq!(toml::from_str::<Site>(&text).unwrap(), s);
    }

    fn site_toml(extra: &str) -> String {
        format!(
            "id = \"s1\"\nname = \"One\"\nhost = \"h\"\nport = 22\nuser = \"u\"\nprotocol = \"ssh\"\n{extra}"
        )
    }

    #[test]
    fn agent_access_serde() {
        let mut site = Site::new("One", "h", "u", Protocol::Ssh);
        assert!(!toml::to_string(&site).unwrap().contains("agent_access"));
        site.agent_access = AgentAccess::Read;
        let text = toml::to_string(&site).unwrap();
        assert!(text.contains("agent_access = \"read\""), "{text}");
        assert_eq!(
            toml::from_str::<Site>(&text).unwrap().agent_access,
            AgentAccess::Read
        );
        site.agent_access = AgentAccess::All;
        let text = toml::to_string(&site).unwrap();
        assert!(text.contains("agent_access = \"all\""), "{text}");
        assert_eq!(
            toml::from_str::<Site>(&text).unwrap().agent_access,
            AgentAccess::All
        );

        for extra in [
            "",
            "agent_access = \"none\"\n",
            "agent_access = \"write\"\n",
            "agent_access = \"ALL\"\n",
            "agent_access = 3\n",
            "agent_access = true\n",
            "agent_access = [\"all\"]\n",
            "agent_access = { level = \"all\" }\n",
        ] {
            let site: Site = toml::from_str(&site_toml(extra)).unwrap();
            assert_eq!(site.agent_access, AgentAccess::None, "{extra}");
        }
        let site: Site = toml::from_str(&site_toml("agent_access = \"all\"\n")).unwrap();
        assert_eq!(site.agent_access, AgentAccess::All);
    }

    #[test]
    fn agent_access_order() {
        use AgentAccess::*;
        assert!(All.allows(Read));
        assert!(Read.allows(Read));
        assert!(!Read.allows(All));
        assert!(!None.allows(Read));
        assert!(None.is_none() && !Read.is_none());
        assert_eq!(Site::new("a", "h", "u", Protocol::Ftp).agent_access, None);
    }
}
