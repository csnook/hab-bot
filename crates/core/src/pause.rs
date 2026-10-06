//! Setting a reminder or a whole reminder list aside for a period (spec:
//! Reminders → Acting on occurrences → Pause; ADR 0011).

use serde::{Deserialize, Serialize};

/// A pause: from when it began until a time, or until it is ended by hand.
///
/// `from` is when the pause was made. Instances before it are not in the
/// pause, so pausing never reaches back to what already fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pause {
    /// When the pause began, in Unix seconds.
    pub from: i64,
    /// When it ends, in Unix seconds; `None` lasts until it is resumed.
    pub until: Option<i64>,
}

impl Pause {
    /// Whether an instance at `t` falls in the pause.
    pub fn covers(&self, t: i64) -> bool {
        t >= self.from && self.until.is_none_or(|u| t < u)
    }
}

/// What put an occurrence under a pause: the reminder's own, or its list's.
/// The history attributes a skipped occurrence to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PauseCause {
    /// When that pause ends; `None` is until it is resumed.
    pub until: Option<i64>,
    /// The pause is the reminder list's, not the reminder's own.
    pub list: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pause_covers_from_when_it_began_until_it_ends() {
        let p = Pause {
            from: 100,
            until: Some(200),
        };
        assert!(!p.covers(99));
        assert!(p.covers(100) && p.covers(199));
        assert!(!p.covers(200));
        let forever = Pause {
            from: 100,
            until: None,
        };
        assert!(forever.covers(i64::MAX));
        assert!(!forever.covers(0));
    }
}
