//! Local client identity.
//!
//! Each connection uses `SO_PEERCRED` (UID/PID) plus a daemon-local session
//! number. The session number prevents cleanup from an older connection from
//! releasing resources acquired after the same process reconnects.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::net::UnixStream;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientId {
    Local { uid: u32, pid: u32, session_id: u64 },
}

impl ClientId {
    pub fn from_unix_stream(stream: &UnixStream) -> io::Result<Self> {
        let cred = stream.peer_cred()?;
        Ok(ClientId::Local {
            uid: cred.uid(),
            pid: cred.pid().unwrap_or(0) as u32,
            session_id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
        })
    }
}
