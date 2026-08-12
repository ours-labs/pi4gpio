//! クライアント識別。
//!
//! ローカルソケット接続では`SO_PEERCRED`（UID/PID）とdaemon内で一意な
//! セッション番号を識別子として使う。同じプロセスが再接続した場合も古い接続と
//! 新しい接続を区別し、古い接続の切断cleanupが新しい接続のロックを解放しない。
//! リモート経路（Tailscale限定bind＋APIキー）が有効な場合はAPIキーを識別子にする。

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::net::UnixStream;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClientId {
    Local {
        uid: u32,
        pid: u32,
        session_id: u64,
    },
    // NETWORK_POLICY.mdのTailscale限定bind＋APIキー実装まで未使用。
    #[allow(dead_code)]
    Remote {
        api_key_id: String,
        session_id: u64,
    },
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
