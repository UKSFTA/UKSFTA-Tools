//! The scaling decision for the adaptive connection limit.
//!
//! The decision is pure. The monitor feeds one sample window at a time and
//! the tracker returns the next action, so the policy can be tested without
//! a server or a clock.

use crate::error::UksftaError;

/// A rate must improve by this factor to raise the limit.
const RISE: f64 = 1.10;
/// A rate must fall by this factor to lower the limit.
const DROP: f64 = 0.80;
/// Windows to wait after a change before another change is allowed.
const COOLDOWN: u32 = 2;
/// The limit a run starts at when the maximum is larger.
pub const START: u8 = 4;

/// What the monitor should do after one sample window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Keep the current limit.
    Hold,
    /// The limit rose to this value.
    Raise(u8),
    /// The limit fell to this value.
    Lower(u8),
}

/// The adaptive limit for one segmented run.
#[derive(Debug, Clone)]
pub struct Tracker {
    max: u8,
    limit: u8,
    reference: Option<f64>,
    window: u32,
    next_change: u32,
    no_raise: bool,
}

impl Tracker {
    /// Start at the smaller of [`START`] and the maximum.
    pub fn new(max: u8) -> Self {
        let max = max.max(1);
        Self {
            max,
            limit: START.min(max),
            reference: None,
            window: 0,
            next_change: 0,
            no_raise: false,
        }
    }

    /// The server refused one extra session. Keep the limit and never raise
    /// it again in this run.
    pub fn lock_out_raise(&mut self) {
        self.no_raise = true;
    }

    /// Feed one sample window. `rate` is the acknowledged bytes per second,
    /// `retries` counts range retries in the window, and
    /// `connection_failure` reports a session that would not open.
    pub fn sample(&mut self, rate: f64, retries: u32, connection_failure: bool) -> Action {
        self.window = self.window.saturating_add(1);
        let Some(reference) = self.reference else {
            self.reference = Some(rate);
            self.next_change = self.window + COOLDOWN;
            return Action::Hold;
        };
        if self.window < self.next_change {
            return Action::Hold;
        }
        if retries > 0 || connection_failure {
            return self.change(rate, false);
        }
        if rate <= reference * DROP {
            return self.change(rate, false);
        }
        if !self.no_raise && self.limit < self.max && rate >= reference * RISE {
            return self.change(rate, true);
        }
        Action::Hold
    }

    fn change(&mut self, rate: f64, up: bool) -> Action {
        let next = if up {
            (self.limit + 1).min(self.max)
        } else {
            self.limit.saturating_sub(1).max(1)
        };
        if next == self.limit {
            return Action::Hold;
        }
        self.limit = next;
        self.reference = Some(rate);
        self.next_change = self.window + COOLDOWN;
        if up {
            Action::Raise(next)
        } else {
            Action::Lower(next)
        }
    }
}

/// True when the error says the server refused another channel or ran out of
/// sessions.
pub fn is_session_limit(error: &UksftaError) -> bool {
    let text = error.to_string().to_lowercase();
    [
        "channel",
        "resource",
        "administratively prohibited",
        "too many",
        "max sessions",
        "session limit",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(tracker: &mut Tracker, rate: f64) -> Action {
        tracker.sample(rate, 0, false)
    }

    #[test]
    fn a_run_starts_at_four_or_the_maximum() {
        assert_eq!(Tracker::new(1).limit, 1);
        assert_eq!(Tracker::new(4).limit, 4);
        assert_eq!(Tracker::new(8).limit, 4);
        assert_eq!(Tracker::new(16).limit, 4);
    }

    #[test]
    fn a_gain_after_the_cooldown_raises_the_limit() {
        let mut tracker = Tracker::new(8);
        assert_eq!(gain(&mut tracker, 100.0), Action::Hold);
        assert_eq!(gain(&mut tracker, 200.0), Action::Hold, "cooldown");
        assert_eq!(gain(&mut tracker, 112.0), Action::Raise(5));
        assert_eq!(tracker.limit, 5);
    }

    #[test]
    fn a_small_gain_holds() {
        let mut tracker = Tracker::new(8);
        gain(&mut tracker, 100.0);
        gain(&mut tracker, 100.0);
        assert_eq!(gain(&mut tracker, 105.0), Action::Hold);
        assert_eq!(tracker.limit, 4);
    }

    #[test]
    fn a_retry_lowers_the_limit() {
        let mut tracker = Tracker::new(8);
        gain(&mut tracker, 100.0);
        gain(&mut tracker, 100.0);
        assert_eq!(tracker.sample(100.0, 1, false), Action::Lower(3));
    }

    #[test]
    fn a_connection_failure_lowers_the_limit() {
        let mut tracker = Tracker::new(8);
        gain(&mut tracker, 100.0);
        gain(&mut tracker, 100.0);
        assert_eq!(tracker.sample(100.0, 0, true), Action::Lower(3));
    }

    #[test]
    fn a_rate_drop_of_a_fifth_lowers_the_limit() {
        let mut tracker = Tracker::new(8);
        gain(&mut tracker, 100.0);
        gain(&mut tracker, 100.0);
        assert_eq!(gain(&mut tracker, 81.0), Action::Hold);
        assert_eq!(gain(&mut tracker, 79.0), Action::Lower(3));
    }

    #[test]
    fn a_change_waits_for_the_cooldown() {
        let mut tracker = Tracker::new(8);
        gain(&mut tracker, 100.0);
        gain(&mut tracker, 100.0);
        assert_eq!(gain(&mut tracker, 200.0), Action::Raise(5));
        assert_eq!(gain(&mut tracker, 400.0), Action::Hold, "cooldown");
        assert_eq!(gain(&mut tracker, 400.0), Action::Raise(6));
    }

    #[test]
    fn a_refused_session_blocks_further_raises() {
        let mut tracker = Tracker::new(8);
        gain(&mut tracker, 100.0);
        gain(&mut tracker, 100.0);
        tracker.lock_out_raise();
        assert_eq!(gain(&mut tracker, 400.0), Action::Hold);
        assert_eq!(tracker.limit, 4);
        // A retry still lowers the limit: only raising is locked out.
        assert_eq!(tracker.sample(400.0, 1, false), Action::Lower(3));
    }

    #[test]
    fn the_limit_stays_between_one_and_the_maximum() {
        let mut tracker = Tracker::new(2);
        assert_eq!(tracker.limit, 2);
        assert_eq!(gain(&mut tracker, 100.0), Action::Hold);
        assert_eq!(gain(&mut tracker, 100.0), Action::Hold);
        assert_eq!(
            gain(&mut tracker, 400.0),
            Action::Hold,
            "already at the maximum"
        );
        assert_eq!(tracker.sample(400.0, 1, false), Action::Lower(1));
        assert_eq!(
            tracker.sample(400.0, 1, false),
            Action::Hold,
            "already at one"
        );
    }

    #[test]
    fn a_channel_error_reads_as_a_session_limit() {
        let refused = UksftaError::Input("ssh: channel open failed".to_string());
        assert!(is_session_limit(&refused));
        let missing = UksftaError::Input("no such file".to_string());
        assert!(!is_session_limit(&missing));
    }
}
