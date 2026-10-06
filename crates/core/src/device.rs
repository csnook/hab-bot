//! What one device does with alerts on its own (spec: Alerts → Settings per
//! device): the loudest style it uses, and "Quiet this device until…".
//!
//! Both stay on the device and never sync (spec: Sync → What syncs where):
//! other devices still alert in full, and nothing is written to any list's
//! stream, so neither is a snooze and neither shows in an occurrence's
//! history except through the style the device's own alert was recorded in.

use serde::{Deserialize, Serialize};

use crate::priority::{AlertStyle, Priority};

/// The loudest alert style this device uses. The default, an alarm, caps
/// nothing.
///
/// Maximum is left out of the cap unless `caps_maximum`: "Maximum still gets
/// through unless the cap says otherwise".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoudestAlert {
    pub style: AlertStyle,
    pub caps_maximum: bool,
}

impl Default for LoudestAlert {
    fn default() -> Self {
        LoudestAlert {
            style: AlertStyle::Alarm,
            caps_maximum: false,
        }
    }
}

/// "Quiet this device until…": everything on this device is silent until
/// `until` (unix seconds). Maximum is left out unless `include_maximum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceQuiet {
    pub until: i64,
    pub include_maximum: bool,
}

/// Both limits, as the alerter reads them each pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceLimits {
    pub loudest: LoudestAlert,
    pub quiet: Option<DeviceQuiet>,
}

impl DeviceLimits {
    /// The quiet setting if it is still in force at `now`.
    pub fn quiet_at(&self, now: i64) -> Option<DeviceQuiet> {
        self.quiet.filter(|q| q.until > now)
    }

    /// The style an alert of `priority` that would be in `style` is in on
    /// this device at `now`: capped to the loudest style, and silent while
    /// the device is quiet. Maximum is exempt from each unless it is included.
    pub fn limit(&self, priority: Priority, style: AlertStyle, now: i64) -> AlertStyle {
        let maximum = priority == Priority::Maximum;
        let mut style = style;
        if !maximum || self.loudest.caps_maximum {
            style = style.min(self.loudest.style);
        }
        if let Some(q) = self.quiet_at(now) {
            if !maximum || q.include_maximum {
                style = AlertStyle::Silent;
            }
        }
        style
    }

    /// When the quiet setting ends, if it is in force: a moment for the
    /// alerter to look again.
    pub fn next_change(&self, now: i64) -> Option<i64> {
        self.quiet_at(now).map(|q| q.until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AlertStyle::*;

    fn caps(style: AlertStyle, caps_maximum: bool) -> DeviceLimits {
        DeviceLimits {
            loudest: LoudestAlert {
                style,
                caps_maximum,
            },
            quiet: None,
        }
    }

    fn quiet(include_maximum: bool) -> DeviceLimits {
        DeviceLimits {
            quiet: Some(DeviceQuiet {
                until: 100,
                include_maximum,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn the_default_changes_nothing() {
        let l = DeviceLimits::default();
        for p in [Priority::Low, Priority::Maximum] {
            for s in [Silent, Gentle, Insistent, Alarm] {
                assert_eq!(l.limit(p, s, 0), s);
            }
        }
    }

    #[test]
    fn a_cap_lowers_louder_styles_and_leaves_quieter_ones() {
        let l = caps(Insistent, false);
        assert_eq!(l.limit(Priority::High, Alarm, 0), Insistent);
        assert_eq!(l.limit(Priority::High, Insistent, 0), Insistent);
        assert_eq!(l.limit(Priority::High, Gentle, 0), Gentle);
        assert_eq!(l.limit(Priority::High, Silent, 0), Silent);
    }

    #[test]
    fn maximum_gets_through_a_cap_unless_the_cap_says_otherwise() {
        assert_eq!(
            caps(Gentle, false).limit(Priority::Maximum, Alarm, 0),
            Alarm
        );
        assert_eq!(
            caps(Gentle, true).limit(Priority::Maximum, Alarm, 0),
            Gentle
        );
    }

    #[test]
    fn quiet_makes_everything_silent_until_it_ends() {
        let l = quiet(false);
        assert_eq!(l.limit(Priority::High, Insistent, 99), Silent);
        assert_eq!(l.limit(Priority::High, Insistent, 100), Insistent);
        assert_eq!(l.next_change(50), Some(100));
        assert_eq!(l.next_change(100), None);
    }

    #[test]
    fn quiet_leaves_out_maximum_unless_included() {
        assert_eq!(quiet(false).limit(Priority::Maximum, Alarm, 0), Alarm);
        assert_eq!(quiet(true).limit(Priority::Maximum, Alarm, 0), Silent);
    }

    #[test]
    fn quiet_and_a_cap_together_take_the_quieter() {
        let mut l = quiet(false);
        l.loudest.style = Gentle;
        assert_eq!(l.limit(Priority::Maximum, Alarm, 0), Alarm);
        assert_eq!(l.limit(Priority::Low, Alarm, 0), Silent);
    }
}
