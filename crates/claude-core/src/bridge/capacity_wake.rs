//! Capacity wake primitive for bridge poll loops.
//!
//! Port of TS `src/bridge/capacityWake.ts`. A poll loop may sleep while all
//! bridge capacity is consumed, but it should wake early when either shutdown
//! is requested or capacity frees up.

use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct CapacityWake {
    outer: CancellationToken,
    notify: Arc<Notify>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityWakeReason {
    OuterCancelled,
    CapacityChanged,
}

impl CapacityWake {
    pub fn new(outer: CancellationToken) -> Self {
        Self {
            outer,
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn wake(&self) {
        self.notify.notify_waiters();
    }

    pub async fn wait(&self) -> CapacityWakeReason {
        tokio::select! {
            _ = self.outer.cancelled() => CapacityWakeReason::OuterCancelled,
            _ = self.notify.notified() => CapacityWakeReason::CapacityChanged,
        }
    }

    pub async fn sleep_or_wake(&self, duration: std::time::Duration) -> Option<CapacityWakeReason> {
        tokio::select! {
            reason = self.wait() => Some(reason),
            _ = tokio::time::sleep(duration) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wakes_on_capacity_change() {
        let token = CancellationToken::new();
        let wake = CapacityWake::new(token);
        let waiter = {
            let wake = wake.clone();
            tokio::spawn(async move { wake.wait().await })
        };
        tokio::task::yield_now().await;
        wake.wake();
        assert_eq!(waiter.await.unwrap(), CapacityWakeReason::CapacityChanged);
    }

    #[tokio::test]
    async fn wakes_on_outer_cancel() {
        let token = CancellationToken::new();
        let wake = CapacityWake::new(token.clone());
        let waiter = {
            let wake = wake.clone();
            tokio::spawn(async move { wake.wait().await })
        };
        tokio::task::yield_now().await;
        token.cancel();
        assert_eq!(waiter.await.unwrap(), CapacityWakeReason::OuterCancelled);
    }

    #[tokio::test(start_paused = true)]
    async fn sleep_returns_none_on_timeout() {
        let token = CancellationToken::new();
        let wake = CapacityWake::new(token);
        let waiter =
            tokio::spawn(
                async move { wake.sleep_or_wake(std::time::Duration::from_secs(10)).await },
            );
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_secs(10)).await;
        assert_eq!(waiter.await.unwrap(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn sleep_returns_reason_on_wake() {
        let token = CancellationToken::new();
        let wake = CapacityWake::new(token);
        let waiter = {
            let wake = wake.clone();
            tokio::spawn(
                async move { wake.sleep_or_wake(std::time::Duration::from_secs(10)).await },
            )
        };
        tokio::task::yield_now().await;
        wake.wake();
        assert_eq!(
            waiter.await.unwrap(),
            Some(CapacityWakeReason::CapacityChanged)
        );
    }
}
