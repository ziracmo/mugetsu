//! When to fetch: every 10 s for the repository the user is looking at,
//! every 5 min for the others (and for everything while the app is in the background).

use std::time::{Duration, Instant};

pub const FOCUSED_INTERVAL: Duration = Duration::from_secs(10);
pub const BACKGROUND_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub fn interval(focused: bool) -> Duration {
    if focused {
        FOCUSED_INTERVAL
    } else {
        BACKGROUND_INTERVAL
    }
}

/// `last` is when the previous fetch *started*; `None` means never.
pub fn is_due(last: Option<Instant>, now: Instant, focused: bool) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= interval(focused))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_repos_fetch_every_ten_seconds() {
        let t0 = Instant::now();
        assert!(is_due(None, t0, true));
        assert!(!is_due(Some(t0), t0 + Duration::from_secs(9), true));
        assert!(is_due(Some(t0), t0 + Duration::from_secs(10), true));
    }

    #[test]
    fn background_repos_fetch_every_five_minutes() {
        let t0 = Instant::now();
        assert!(!is_due(Some(t0), t0 + Duration::from_secs(60), false));
        assert!(is_due(Some(t0), t0 + Duration::from_secs(300), false));
    }
}
