//! In-process linked MCP transport pair.
//!
//! Mirrors TS `services/mcp/InProcessTransport.ts`: two endpoints are linked;
//! `send()` on one endpoint asynchronously delivers the JSON-RPC message to
//! the peer, and `close()` on either endpoint closes both sides.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug)]
pub struct InProcessTransport {
    peer_tx: mpsc::UnboundedSender<Value>,
    rx: Mutex<mpsc::UnboundedReceiver<Value>>,
    closed: Arc<AtomicBool>,
    peer_closed: Arc<AtomicBool>,
}

impl InProcessTransport {
    pub async fn start(&self) -> Result<()> {
        Ok(())
    }

    pub async fn send(&self, message: Value) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(anyhow!("Transport is closed"));
        }
        let tx = self.peer_tx.clone();
        tokio::task::spawn(async move {
            let _ = tx.send(message);
        });
        Ok(())
    }

    pub async fn recv(&self) -> Option<Value> {
        self.rx.lock().await.recv().await
    }

    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.peer_closed.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

pub fn create_linked_transport_pair() -> (InProcessTransport, InProcessTransport) {
    let (a_tx, a_rx) = mpsc::unbounded_channel();
    let (b_tx, b_rx) = mpsc::unbounded_channel();
    let a_closed = Arc::new(AtomicBool::new(false));
    let b_closed = Arc::new(AtomicBool::new(false));

    (
        InProcessTransport {
            peer_tx: b_tx,
            rx: Mutex::new(a_rx),
            closed: a_closed.clone(),
            peer_closed: b_closed.clone(),
        },
        InProcessTransport {
            peer_tx: a_tx,
            rx: Mutex::new(b_rx),
            closed: b_closed,
            peer_closed: a_closed,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn send_delivers_to_peer_asynchronously() {
        let (a, b) = create_linked_transport_pair();
        a.start().await.unwrap();
        b.start().await.unwrap();
        a.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
            .await
            .unwrap();
        assert_eq!(
            b.recv().await,
            Some(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
        );
    }

    #[tokio::test]
    async fn close_closes_both_sides_and_is_idempotent() {
        let (a, b) = create_linked_transport_pair();
        a.close().await.unwrap();
        a.close().await.unwrap();
        assert!(a.is_closed());
        assert!(b.is_closed());
        assert_eq!(
            a.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
                .await
                .unwrap_err()
                .to_string(),
            "Transport is closed"
        );
    }
}
