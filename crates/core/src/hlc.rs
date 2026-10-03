//! A hybrid logical clock (ADR 0005): orders the changes made to a setting on
//! several devices without trusting any one device's clock to be right.

use std::fmt;
use std::str::FromStr;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

/// How far ahead of the time a device recorded an event its clock may claim to
/// be. The server rejects events recorded more than 10 minutes ahead of its own
/// clock; this allows for that and for one device having seen another's. A
/// clock further ahead than this is cut back, so one device with a wrong
/// clock can't win every edit forever.
pub const MAX_AHEAD_MS: i64 = 1_200_000;

/// A point in the hybrid logical clock's order: the wall clock in
/// milliseconds, a counter for changes in the same millisecond or made by a
/// device whose clock is behind what it has seen, and the device that made it,
/// so two clocks are never equal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Hlc {
    pub wall_ms: i64,
    pub counter: u32,
    pub node: String,
}

impl Hlc {
    /// The clock for a change made at `now` (Unix seconds) by `node`, after
    /// everything up to and including `seen`. It is always later than `seen`.
    pub fn next(now: i64, node: &str, seen: &Hlc) -> Hlc {
        let wall_ms = (now.saturating_mul(1000)).max(seen.wall_ms);
        let counter = if wall_ms == seen.wall_ms {
            seen.counter + 1
        } else {
            0
        };
        Hlc {
            wall_ms,
            counter,
            node: node.to_string(),
        }
    }

    /// This clock, cut back if it claims to be far ahead of when the event was
    /// recorded (Unix seconds).
    pub fn clamped(&self, recorded_at: i64) -> Hlc {
        let limit = recorded_at
            .saturating_mul(1000)
            .saturating_add(MAX_AHEAD_MS);
        if self.wall_ms > limit {
            Hlc {
                wall_ms: limit,
                counter: 0,
                node: self.node.clone(),
            }
        } else {
            self.clone()
        }
    }
}

/// Written so that the text sorts the same way as the clock.
impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016}.{:08}.{}", self.wall_ms, self.counter, self.node)
    }
}

impl FromStr for Hlc {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(3, '.');
        let (Some(w), Some(c), Some(n)) = (parts.next(), parts.next(), parts.next()) else {
            return Err("not a clock");
        };
        Ok(Hlc {
            wall_ms: w.parse().map_err(|_| "not a clock")?,
            counter: c.parse().map_err(|_| "not a clock")?,
            node: n.to_string(),
        })
    }
}

impl Serialize for Hlc {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hlc {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_wall_clock_is_later_and_the_counter_breaks_ties() {
        let a = Hlc::next(100, "a", &Hlc::default());
        let b = Hlc::next(100, "a", &a);
        let c = Hlc::next(101, "a", &b);
        assert!(a < b && b < c);
        assert_eq!((b.wall_ms, b.counter), (100_000, 1));
        assert_eq!((c.wall_ms, c.counter), (101_000, 0));
    }

    #[test]
    fn a_device_whose_clock_is_behind_still_orders_after_what_it_has_seen() {
        let ahead = Hlc::next(500, "phone", &Hlc::default());
        let behind = Hlc::next(100, "desktop", &ahead);
        assert!(behind > ahead);
    }

    #[test]
    fn two_devices_at_the_same_instant_are_ordered_by_device() {
        let a = Hlc::next(10, "1", &Hlc::default());
        let b = Hlc::next(10, "2", &Hlc::default());
        assert_ne!(a, b);
        assert!(a < b);
    }

    #[test]
    fn it_round_trips_as_text_that_sorts_like_the_clock() {
        let a = Hlc::next(9, "7", &Hlc::default());
        let b = Hlc::next(10, "7", &a);
        assert_eq!(a.to_string().parse::<Hlc>().unwrap(), a);
        assert!(a.to_string() < b.to_string());
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Hlc>(&json).unwrap(), b);
        assert!("nonsense".parse::<Hlc>().is_err());
    }

    #[test]
    fn a_clock_far_ahead_of_when_the_event_was_recorded_is_cut_back() {
        let wild = Hlc::next(10_000_000, "x", &Hlc::default());
        assert_eq!(wild.clamped(100).wall_ms, 100_000 + MAX_AHEAD_MS);
        let ok = Hlc::next(100, "x", &Hlc::default());
        assert_eq!(ok.clamped(100), ok);
    }
}
