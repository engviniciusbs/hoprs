//! What the agent service may do with the rest of hoprs: a narrow interface over the
//! backend's vault, sessions, prompt broker and transfer engine. The real one is
//! `backend::agent::RealHost`; the service's tests use the in-memory `FakeHost`
//! (`agent::testing`).
//!
//! Nothing here opens a tab, takes focus or answers a dialog: a connection an agent asks
//! for goes through the same prompt broker as the user's, its dialogs flagged as asked by
//! an agent, and the agent only waits.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use hoprs_core::{Error, Protocol, Result, Site, SiteId};
use hoprs_remote::RemoteFs;
use hoprs_ssh::{ExecLimits, ExecOutput};
use hoprs_transfer::{ConflictPolicy, Event, TransferId, TransferItem, TransferSpec};
use tokio::sync::broadcast;

use super::paths::Protected;
use crate::backend::SessionId;

/// A connection an agent asked for did not come up.
///
/// `Debug` prints only the error's kind and code: an authentication error's message or
/// detail can name key files. The service answers the agent with a fixed sentence per
/// kind, never with this error's text.
pub struct ConnectFailure {
    pub error: Error,
}

impl fmt::Debug for ConnectFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectFailure")
            .field("kind", &self.error.kind)
            .field("code", &self.error.code)
            .finish_non_exhaustive()
    }
}

/// Where a session connects: the site's protocol, host, port and user name as they were
/// when the session started. A site edited since may name another server; the service
/// compares the two before it uses a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl Endpoint {
    pub fn of(site: &Site) -> Endpoint {
        Endpoint {
            protocol: site.protocol,
            host: site.host.clone(),
            port: site.port,
            user: site.user.clone(),
        }
    }
}

/// The site's session as the agent service checks it before use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEndpoint {
    /// Where the session connects.
    pub endpoint: Endpoint,
    /// Only an agent uses it (no tab).
    pub agent_only: bool,
}

/// The operations the agent service needs (see the module docs).
#[async_trait]
pub trait AgentHost: Send + Sync + 'static {
    /// The master switch of Settings › Agents (MCP), now.
    fn enabled(&self) -> bool;
    /// The saved sites, read now (their access levels included).
    async fn sites(&self) -> Result<Vec<Site>>;
    /// The site's session while it is connected and ready.
    fn connected(&self, site: &SiteId) -> Option<SessionId>;
    /// Connects the site for an agent: a new session is the agent's and its dialogs say an
    /// agent asked; a session the site already has is reused as it is.
    async fn connect(&self, site: &SiteId) -> std::result::Result<SessionId, ConnectFailure>;
    /// Where the site's session (in any state) connects, and whether only an agent uses
    /// it; `None` without a session.
    fn session_endpoint(&self, site: &SiteId) -> Option<SessionEndpoint>;
    /// Whether the site's session is waiting on the user (a dialog is open).
    fn prompt_open(&self, site: &SiteId) -> bool;
    /// Closes the site's session if only an agent uses it; whether it did.
    async fn close_agent_session(&self, site: &SiteId) -> bool;
    /// The sites whose session only an agent uses, sorted.
    fn agent_sites(&self) -> Vec<SiteId>;
    /// The remote filesystem of a connected session.
    fn remote(&self, session: SessionId) -> Result<Arc<dyn RemoteFs>>;
    /// Runs one command on an SSH session.
    async fn exec(
        &self,
        session: SessionId,
        command: String,
        limits: ExecLimits,
    ) -> Result<ExecOutput>;
    /// Enqueues an agent's transfer under `policy` (one that never asks); the engine
    /// checks each of its items' local path against the protected ones.
    async fn enqueue(&self, spec: TransferSpec, policy: ConflictPolicy) -> Result<TransferId>;
    /// The transfer queue, every origin.
    async fn transfers(&self) -> Vec<TransferItem>;
    /// A new receiver of the engine's events.
    fn transfer_events(&self) -> broadcast::Receiver<Event>;
    /// The local paths an agent may not touch, as they are now.
    async fn protected(&self) -> Protected;
}

#[cfg(test)]
mod tests {
    use hoprs_core::ErrorKind;

    use super::*;

    #[test]
    fn connect_failure_debug_names_no_path() {
        let failure = ConnectFailure {
            error: Error::new(ErrorKind::Auth, "No key in /home/me/.ssh/id_ed25519")
                .with_detail("tried /home/me/.ssh/deploy"),
        };
        let text = format!("{failure:?}");
        assert!(!text.contains("/home"), "{text}");
        assert!(text.contains("Auth"), "{text}");
    }
}
