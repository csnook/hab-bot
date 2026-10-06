//! Delays: how a reminder's overdue time or an expiry is counted from an
//! occurrence's scheduled time. Either a duration ("1 hour") or the next time
//! a schedule matches ("the next 1st at 00:00"). Neither makes expected
//! occurrences (spec: Sources → Overdue times and expiry delays).

use jiff::tz::TimeZone;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::schedule::{Pattern, Schedule};

/// How far ahead a "next time a schedule matches" looks: five years, enough
/// for any yearly schedule.
const HORIZON: i64 = 5 * 366 * 86_400;

/// A moment counted from an occurrence's scheduled time.
///
/// Serialised as a bare number of seconds (which is what format 3 wrote) or
/// as a schedule object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Delay {
    /// This many seconds after the scheduled time.
    After(i64),
    /// The next time this schedule matches after the scheduled time.
    Next(Schedule),
}

impl Delay {
    /// Whether it can be used: a duration is not negative, a schedule is
    /// readable.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Delay::After(d) if *d < 0 => Err("a duration can't be negative".into()),
            Delay::After(_) => Ok(()),
            Delay::Next(s) => s.validate(),
        }
    }

    /// The instant it comes for an occurrence scheduled at `from`, with a
    /// schedule's times read in `zone`. `None` if the schedule never matches
    /// again.
    pub fn resolve(&self, from: i64, zone: &TimeZone) -> Option<i64> {
        match self {
            Delay::After(d) => Some(from.saturating_add(*d)),
            Delay::Next(s) => s
                .instances(zone, from, from.saturating_add(HORIZON), 1)
                .first()
                .copied(),
        }
    }

    /// Whether an app that reads only format 3 can read it.
    pub(crate) fn is_legacy(&self) -> bool {
        matches!(self, Delay::After(_))
    }
}

/// The date a "next time" schedule starts from. Long past, so that it has
/// instances after any occurrence; the pattern says which days count.
const SPEC_START: &str = "2000-01-03";

/// A delay as the editor shows and sends it: a duration, or the next time a
/// pattern matches ("the next 1st at 00:00").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DelaySpec {
    After {
        seconds: i64,
    },
    Next {
        pattern: Pattern,
        time: String,
    },
    /// A schedule written by hand that isn't one of the editor's patterns.
    /// The editor shows its rule and leaves it alone.
    Other {
        rule: String,
    },
}

impl DelaySpec {
    /// The delay it means.
    pub fn to_delay(&self) -> Result<Delay, String> {
        match self {
            DelaySpec::After { seconds } => Ok(Delay::After(*seconds)),
            DelaySpec::Next { pattern, time } => {
                Schedule::from_pattern(pattern, SPEC_START, time).map(Delay::Next)
            }
            DelaySpec::Other { .. } => Err("that schedule can't be edited here".into()),
        }
    }
}

impl Delay {
    /// How the editor shows it.
    pub fn spec(&self) -> DelaySpec {
        match self {
            Delay::After(d) => DelaySpec::After { seconds: *d },
            Delay::Next(s) => match s.parts() {
                Some(p) => DelaySpec::Next {
                    pattern: p.pattern,
                    time: p.time,
                },
                None => DelaySpec::Other {
                    rule: s.rule.clone(),
                },
            },
        }
    }
}

/// A reminder's expiry delays as one setting. Format 3 wrote one optional
/// number: `null` is none, a number one duration. It still reads and writes
/// that where it can, and writes a list otherwise.
pub(crate) mod expiries {
    use super::*;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        None(()),
        One(i64),
        Many(Vec<Delay>),
    }

    pub fn serialize<S: Serializer>(v: &[Delay], s: S) -> Result<S::Ok, S::Error> {
        match v {
            [] => s.serialize_none(),
            [Delay::After(d)] => s.serialize_i64(*d),
            many => many.serialize(s),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Delay>, D::Error> {
        Ok(match Wire::deserialize(d)? {
            Wire::None(()) => Vec::new(),
            Wire::One(n) => vec![Delay::After(n)],
            Wire::Many(v) => v,
        })
    }
}
