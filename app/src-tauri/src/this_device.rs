//! Settings → This device: what belongs to this device alone (#51).
//!
//! - Its **loudest alert** and **"Quiet this device until…"** are device
//!   settings in the core: they never sync and never touch another device.
//! - Its **name** and whether it is **portable** are the user's personal
//!   settings (events in the personal list), so their other devices see them.
//!
//! Pure functions on [`hab_core::Core`], tested without a desktop; the Tauri
//! commands that call them are in [`crate`].

use std::path::Path;

use hab_core::{Core, DeviceQuiet, LoudestAlert};
use serde::Serialize;

/// Where Linux lists power supplies.
pub const POWER_SUPPLY_DIR: &str = "/sys/class/power_supply";

/// Whether a battery suggests a laptop: stationary unless one is found. A
/// mouse's or a phone's battery (scope `Device`) doesn't count.
pub fn detect_portable(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let read = |name: &str| {
            std::fs::read_to_string(e.path().join(name))
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        read("type").eq_ignore_ascii_case("battery")
            && !read("scope").eq_ignore_ascii_case("device")
    })
}

/// What Settings → This device shows.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ThisDevice {
    pub name: String,
    pub portable: bool,
    pub loudest: LoudestAlert,
    /// "Quiet this device until…", if in force.
    pub quiet: Option<DeviceQuiet>,
}

/// The device's settings now. `fallback_name` and `fallback_portable` are
/// what it was set up with, before it said anything in the personal list
/// (the profile's, or the host name and the battery check).
pub fn view(core: &Core, fallback_name: &str, fallback_portable: bool, now: i64) -> ThisDevice {
    ThisDevice {
        name: core
            .own_device_name()
            .filter(|n| !n.is_empty())
            .unwrap_or(fallback_name)
            .to_string(),
        portable: core
            .device_portable(core.device_id())
            .unwrap_or(fallback_portable),
        loudest: core.loudest_alert(),
        quiet: core.device_quiet(now),
    }
}

/// Renames this device in the personal list. Returns the name as kept.
pub fn rename(core: &mut Core, name: &str, now: i64) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("This device needs a name.".into());
    }
    core.name_device(name, now).map_err(|e| e.to_string())?;
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hab_core::AlertStyle;

    fn supply(dir: &Path, name: &str, files: &[(&str, &str)]) {
        let d = dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        for (f, v) in files {
            std::fs::write(d.join(f), format!("{v}\n")).unwrap();
        }
    }

    #[test]
    fn a_battery_suggests_a_laptop_and_nothing_else_does() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!detect_portable(&dir.path().join("missing")));
        assert!(!detect_portable(dir.path()));
        supply(dir.path(), "AC", &[("type", "Mains")]);
        supply(dir.path(), "ups", &[("type", "UPS")]);
        assert!(!detect_portable(dir.path()));
        // A wireless mouse's battery says its scope is the device.
        supply(
            dir.path(),
            "hidpp_battery_0",
            &[("type", "Battery"), ("scope", "Device")],
        );
        assert!(!detect_portable(dir.path()));
        supply(dir.path(), "BAT0", &[("type", "Battery")]);
        assert!(detect_portable(dir.path()));
    }

    #[test]
    fn the_view_reads_the_personal_list_before_the_fallbacks() {
        let mut c = Core::open_in_memory().unwrap();
        let t = 1_790_000_000;
        let v = view(&c, "host", true, t);
        assert_eq!((v.name.as_str(), v.portable), ("host", true));
        assert_eq!(v.loudest, LoudestAlert::default());
        assert_eq!(v.quiet, None);

        assert_eq!(
            rename(&mut c, "  Study desktop ", t).unwrap(),
            "Study desktop"
        );
        c.set_portable(false, t).unwrap();
        let v = view(&c, "host", true, t);
        assert_eq!((v.name.as_str(), v.portable), ("Study desktop", false));
    }

    #[test]
    fn the_view_shows_the_cap_and_the_quiet_setting_until_it_ends() {
        let c = Core::open_in_memory().unwrap();
        let t = 1_790_000_000;
        c.set_loudest_alert(LoudestAlert {
            style: AlertStyle::Gentle,
            caps_maximum: true,
        })
        .unwrap();
        c.quiet_device(t + 600, true, t).unwrap();
        let v = view(&c, "host", false, t);
        assert_eq!(v.loudest.style, AlertStyle::Gentle);
        assert!(v.loudest.caps_maximum);
        assert_eq!(
            v.quiet,
            Some(DeviceQuiet {
                until: t + 600,
                include_maximum: true
            })
        );
        assert_eq!(view(&c, "host", false, t + 600).quiet, None);
    }

    #[test]
    fn an_empty_name_is_refused() {
        let mut c = Core::open_in_memory().unwrap();
        assert!(rename(&mut c, "   ", 0).is_err());
        assert_eq!(c.own_device_name(), None);
    }
}
