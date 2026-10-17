//! A sliding-window attempt counter, keyed by client address.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

pub struct RateLimiter {
    max: usize,
    window_s: u64,
    hits: Mutex<HashMap<String, VecDeque<u64>>>,
}

impl RateLimiter {
    pub fn new(max: usize, window_s: u64) -> RateLimiter {
        RateLimiter {
            max,
            window_s,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Records an attempt by `key` at `now_s` and says whether it is within the limit. A refused
    /// attempt is not recorded, so the window ends `window_s` after the last allowed attempt.
    pub fn allow(&self, key: &str, now_s: u64) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if hits.len() > 4096 {
            hits.retain(|_, q| q.back().is_some_and(|t| now_s < t + self.window_s));
        }
        let q = hits.entry(key.to_string()).or_default();
        while q.front().is_some_and(|t| now_s >= t + self.window_s) {
            q.pop_front();
        }
        if q.len() >= self.max {
            return false;
        }
        q.push_back(now_s);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sixth_attempt_in_a_minute_is_refused() {
        let r = RateLimiter::new(5, 60);
        assert!((0..5).all(|i| r.allow("a", 100 + i)));
        assert!(!r.allow("a", 110));
    }

    #[test]
    fn the_window_slides_and_keys_are_independent() {
        let r = RateLimiter::new(5, 60);
        for _ in 0..5 {
            assert!(r.allow("a", 100));
        }
        assert!(r.allow("b", 100));
        assert!(!r.allow("a", 159));
        assert!(r.allow("a", 160));
    }
}
