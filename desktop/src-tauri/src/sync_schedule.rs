//! How often the background sync runs.
//!
//! OBS card pages request `/api/overlay` every 5 seconds. While such requests
//! keep arriving, OBS is showing the cards and the history is synced often so
//! they stay current; otherwise it is synced every few minutes, which keeps the
//! archive complete with far fewer SplatNet requests.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Sync interval while OBS is showing the cards.
pub(crate) const LIVE_SYNC_INTERVAL: Duration = Duration::from_secs(30);
/// Sync interval otherwise.
pub(crate) const IDLE_SYNC_INTERVAL: Duration = Duration::from_secs(300);
/// OBS counts as showing the cards when it requested them this recently.
const LIVE_WINDOW_MS: i64 = 30_000;

/// When OBS last requested card data, shared between the OBS service and the
/// background sync.
#[derive(Clone)]
pub(crate) struct ObsActivity {
    last_request_ms: Arc<AtomicI64>,
    on_start: Arc<dyn Fn() + Send + Sync>,
}

impl Default for ObsActivity {
    fn default() -> Self {
        Self::new(|| {})
    }
}

impl ObsActivity {
    /// `on_start` runs when OBS starts showing the cards after a quiet period,
    /// so the first card is up to date without waiting for the slow interval.
    pub(crate) fn new(on_start: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            last_request_ms: Arc::new(AtomicI64::new(0)),
            on_start: Arc::new(on_start),
        }
    }

    pub(crate) fn record_request(&self, now_ms: i64) {
        let previous = self.last_request_ms.swap(now_ms, Ordering::Relaxed);
        if !within_live_window(previous, now_ms) {
            (self.on_start)();
        }
    }

    pub(crate) fn is_live(&self, now_ms: i64) -> bool {
        within_live_window(self.last_request_ms.load(Ordering::Relaxed), now_ms)
    }

    pub(crate) fn sync_interval(&self, now_ms: i64) -> Duration {
        if self.is_live(now_ms) {
            LIVE_SYNC_INTERVAL
        } else {
            IDLE_SYNC_INTERVAL
        }
    }
}

fn within_live_window(last_request_ms: i64, now_ms: i64) -> bool {
    last_request_ms > 0 && now_ms.saturating_sub(last_request_ms) <= LIVE_WINDOW_MS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn counting() -> (ObsActivity, Arc<AtomicUsize>) {
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&starts);
        let activity = ObsActivity::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        (activity, starts)
    }

    #[test]
    fn syncs_slowly_until_obs_requests_the_cards() {
        let (activity, starts) = counting();
        assert!(!activity.is_live(1_000_000));
        assert_eq!(activity.sync_interval(1_000_000), IDLE_SYNC_INTERVAL);

        activity.record_request(1_000_000);
        assert_eq!(starts.load(Ordering::Relaxed), 1);
        assert!(activity.is_live(1_000_000));
        assert_eq!(activity.sync_interval(1_020_000), LIVE_SYNC_INTERVAL);
    }

    #[test]
    fn only_the_first_request_after_a_quiet_period_triggers_a_sync() {
        let (activity, starts) = counting();
        for second in 0..12 {
            activity.record_request(1_000_000 + second * 5_000);
        }
        assert_eq!(starts.load(Ordering::Relaxed), 1);

        // OBS stopped requesting for longer than the live window.
        assert!(activity.is_live(1_055_000 + LIVE_WINDOW_MS));
        assert!(!activity.is_live(1_055_000 + LIVE_WINDOW_MS + 1));
        assert_eq!(
            activity.sync_interval(1_055_000 + LIVE_WINDOW_MS + 1),
            IDLE_SYNC_INTERVAL
        );
        activity.record_request(1_200_000);
        assert_eq!(starts.load(Ordering::Relaxed), 2);
    }
}
