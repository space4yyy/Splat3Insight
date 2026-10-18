//! How often the background sync runs.
//!
//! While OBS is open the history is synced often, so the cards are current the
//! moment they are shown; a hidden browser source may be shut down and stop
//! polling, so the OBS process itself decides. Card pages polling
//! `/api/overlay` count too, for cards shown outside OBS. Otherwise the history
//! is synced every few minutes, which keeps the archive complete with far fewer
//! SplatNet requests.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Sync interval while OBS is open or a card page reads the data.
pub(crate) const LIVE_SYNC_INTERVAL: Duration = Duration::from_secs(30);
/// Sync interval otherwise.
pub(crate) const IDLE_SYNC_INTERVAL: Duration = Duration::from_secs(300);
/// While waiting for the next sync, how often to look whether OBS opened or closed.
pub(crate) const OBS_CHECK_INTERVAL: Duration = Duration::from_secs(10);
/// A card page counts as reading the data when it requested it this recently.
const LIVE_WINDOW_MS: i64 = 30_000;

/// When a card page last requested data, shared between the OBS service and
/// the background sync, plus how to tell whether OBS is running.
#[derive(Clone)]
pub(crate) struct ObsActivity {
    last_request_ms: Arc<AtomicI64>,
    on_start: Arc<dyn Fn() + Send + Sync>,
    obs_running: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl Default for ObsActivity {
    fn default() -> Self {
        Self::new(|| {})
    }
}

impl ObsActivity {
    /// `on_start` runs when a card page starts reading the data after a quiet
    /// period, so the first card is up to date without waiting for the interval.
    pub(crate) fn new(on_start: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_obs_check(on_start, obs_is_running)
    }

    fn with_obs_check(
        on_start: impl Fn() + Send + Sync + 'static,
        obs_running: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            last_request_ms: Arc::new(AtomicI64::new(0)),
            on_start: Arc::new(on_start),
            obs_running: Arc::new(obs_running),
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

    /// Looks at the running processes, so call it off the async runtime.
    pub(crate) fn sync_interval(&self, now_ms: i64) -> Duration {
        if self.is_live(now_ms) || (self.obs_running)() {
            LIVE_SYNC_INTERVAL
        } else {
            IDLE_SYNC_INTERVAL
        }
    }
}

fn within_live_window(last_request_ms: i64, now_ms: i64) -> bool {
    last_request_ms > 0 && now_ms.saturating_sub(last_request_ms) <= LIVE_WINDOW_MS
}

/// OBS Studio's main process: `OBS` on macOS, `obs64.exe` on Windows, `obs` on Linux.
fn is_obs_process(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "obs" | "obs64.exe" | "obs32.exe" | "obs.exe"
    )
}

#[cfg(target_os = "macos")]
fn obs_is_running() -> bool {
    // proc_listallpids reports how many process ids it wrote; ask for the count first.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return false;
    }
    let mut pids: Vec<libc::c_int> = vec![0; count as usize + 64];
    let buffer_size = (pids.len() * std::mem::size_of::<libc::c_int>()) as libc::c_int;
    let written = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), buffer_size) };
    pids.truncate(written.max(0) as usize);
    pids.into_iter().filter(|pid| *pid > 0).any(|pid| {
        let mut name = [0_u8; 64];
        let length = unsafe { libc::proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32) };
        length > 0 && is_obs_process(&String::from_utf8_lossy(&name[..length as usize]))
    })
}

#[cfg(windows)]
fn obs_is_running() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout).lines().any(|line| {
                line.split(',')
                    .next()
                    .is_some_and(|name| is_obs_process(name.trim_matches('"')))
            })
        })
        .unwrap_or(false)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn obs_is_running() -> bool {
    std::fs::read_dir("/proc")
        .map(|entries| {
            entries.flatten().any(|entry| {
                std::fs::read_to_string(entry.path().join("comm"))
                    .is_ok_and(|name| is_obs_process(&name))
            })
        })
        .unwrap_or(false)
}

#[cfg(not(any(unix, windows)))]
fn obs_is_running() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    fn counting(obs_open: Arc<AtomicBool>) -> (ObsActivity, Arc<AtomicUsize>) {
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&starts);
        let activity = ObsActivity::with_obs_check(
            move || {
                counter.fetch_add(1, Ordering::Relaxed);
            },
            move || obs_open.load(Ordering::Relaxed),
        );
        (activity, starts)
    }

    #[test]
    fn syncs_slowly_until_obs_requests_the_cards() {
        let (activity, starts) = counting(Arc::new(AtomicBool::new(false)));
        assert!(!activity.is_live(1_000_000));
        assert_eq!(activity.sync_interval(1_000_000), IDLE_SYNC_INTERVAL);

        activity.record_request(1_000_000);
        assert_eq!(starts.load(Ordering::Relaxed), 1);
        assert!(activity.is_live(1_000_000));
        assert_eq!(activity.sync_interval(1_020_000), LIVE_SYNC_INTERVAL);
    }

    #[test]
    fn only_the_first_request_after_a_quiet_period_triggers_a_sync() {
        let (activity, starts) = counting(Arc::new(AtomicBool::new(false)));
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

    #[test]
    fn syncs_often_while_obs_is_open_even_without_card_requests() {
        let obs_open = Arc::new(AtomicBool::new(true));
        let (activity, starts) = counting(Arc::clone(&obs_open));
        assert!(!activity.is_live(1_000_000));
        assert_eq!(activity.sync_interval(1_000_000), LIVE_SYNC_INTERVAL);
        assert_eq!(starts.load(Ordering::Relaxed), 0);

        obs_open.store(false, Ordering::Relaxed);
        assert_eq!(activity.sync_interval(1_000_000), IDLE_SYNC_INTERVAL);
    }

    #[test]
    fn recognizes_obs_by_its_process_name_on_each_platform() {
        for name in ["OBS", "obs", "obs64.exe", "OBS64.EXE", "obs32.exe", "obs\n"] {
            assert!(is_obs_process(name), "{name:?}");
        }
        for name in [
            "OBS Helper",
            "OBS Helper (GPU)",
            "obs-ffmpeg-mux",
            "Splat3Insight",
            "",
        ] {
            assert!(!is_obs_process(name), "{name:?}");
        }
        // The real check only needs to run without failing; whether OBS is open varies.
        let _ = obs_is_running();
    }
}
