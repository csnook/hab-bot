//! Starting at login: an XDG autostart entry.
//!
//! The entry is `$XDG_CONFIG_HOME/autostart/io.github.csnook.hab-bot.desktop`
//! (`~/.config/autostart` when that isn't set). It is written directly rather
//! than through `tauri-plugin-autostart`, which would add a dependency for
//! about forty lines: the plugin (through `auto-launch` 0.5) hard-codes
//! `~/.config`, ignoring `XDG_CONFIG_HOME`, and gives the file no icon or
//! `X-GNOME-Autostart-enabled` key. A sandboxed build would use the Background
//! portal instead; this is for the `.deb` and unpackaged builds.
//!
//! The entry runs the app with [`HIDDEN_ARG`], which starts it in the tray
//! with no window (see [`crate::launch`]).
//!
//! "Start at login" is a setting of this device, so it is not an event and
//! does not sync: the file itself is the setting. Whether it is on is whether
//! the file exists and isn't `Hidden=true`. If the file is on but names a
//! different program than the one running (the app moved or was updated into
//! another path), [`Autostart::repair`] rewrites it.
//!
//! Which login mechanism picks the file up (Plasma's autostart, GNOME's, or
//! systemd's xdg-autostart-generator) has not been tried here; the file
//! follows the Desktop Entry and Autostart specifications as the research
//! read them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use crate::launch::HIDDEN_ARG;

/// The entry's file name: the app's identifier, like its desktop file.
pub const FILE_NAME: &str = "io.github.csnook.hab-bot.desktop";

/// An autostart directory.
pub struct Autostart {
    dir: PathBuf,
    /// The program to start.
    exec: PathBuf,
}

/// `$XDG_CONFIG_HOME/autostart`, or `~/.config/autostart`.
pub fn default_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    Some(config.join("autostart"))
}

/// The program an entry should start: the AppImage if that is what runs,
/// otherwise this executable.
pub fn current_exec() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
}

/// A value for the `Exec` key: the program quoted as the Desktop Entry
/// specification wants when it holds a space or a reserved character, then
/// the argument.
fn exec_line(program: &Path) -> String {
    let p = program.to_string_lossy();
    let reserved = |c: char| " \t\n\"'\\><~|&;$*?#()`".contains(c);
    let program = if p.chars().any(reserved) {
        let mut q = String::from("\"");
        for c in p.chars() {
            match c {
                '"' | '`' | '$' | '\\' => {
                    q.push('\\');
                    q.push(c);
                }
                // `%` starts a field code; `%%` is a literal one.
                '%' => q.push_str("%%"),
                c => q.push(c),
            }
        }
        q.push('"');
        q
    } else {
        p.replace('%', "%%")
    };
    format!("{program} {HIDDEN_ARG}")
}

/// The text of the entry.
pub fn entry(program: &Path) -> String {
    // The string values are escaped by the specification's rules; ours hold
    // nothing that needs it.
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name=Reminders\n\
         Comment=Reminders in the system tray\n\
         Exec={}\n\
         Icon=io.github.csnook.hab-bot\n\
         Terminal=false\n\
         StartupNotify=false\n\
         X-GNOME-Autostart-enabled=true\n",
        exec_line(program)
    )
}

/// The `Exec` value of an entry's text, for comparing.
fn exec_of(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|l| l.strip_prefix("Exec="))
        .map(str::trim)
}

fn is_hidden(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        l.eq_ignore_ascii_case("Hidden=true")
            || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

impl Autostart {
    pub fn new(dir: PathBuf, exec: PathBuf) -> Self {
        Self { dir, exec }
    }

    /// For this device: the user's autostart directory and this program.
    pub fn for_this_device() -> Option<Self> {
        Some(Self::new(default_dir()?, current_exec()?))
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    pub fn is_enabled(&self) -> bool {
        fs::read_to_string(self.path()).is_ok_and(|t| !is_hidden(&t))
    }

    /// Writes the entry (replacing one that is there), through a temporary
    /// file so a crash never leaves half of one.
    pub fn enable(&self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(".{FILE_NAME}.tmp"));
        fs::write(&tmp, entry(&self.exec))?;
        fs::rename(&tmp, self.path())
    }

    /// Removes the entry. Not being there is fine.
    pub fn disable(&self) -> io::Result<()> {
        match fs::remove_file(self.path()) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn set(&self, on: bool) -> io::Result<()> {
        if on {
            self.enable()
        } else {
            self.disable()
        }
    }

    /// If the entry is on but starts some other program, points it at this
    /// one. Returns whether it rewrote it. Left alone: no entry, and an entry
    /// the user switched off by hand.
    pub fn repair(&self) -> io::Result<bool> {
        let Ok(text) = fs::read_to_string(self.path()) else {
            return Ok(false);
        };
        if is_hidden(&text) || exec_of(&text) == exec_of(&entry(&self.exec)) {
            return Ok(false);
        }
        self.enable().map(|_| true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hab-autostart-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_entry_is_a_hidden_start_of_this_program() {
        let text = entry(Path::new("/usr/bin/hab-app"));
        assert!(text.starts_with("[Desktop Entry]\n"));
        for line in [
            "Type=Application",
            "Name=Reminders",
            "Exec=/usr/bin/hab-app --hidden",
            "Terminal=false",
            "X-GNOME-Autostart-enabled=true",
        ] {
            assert!(text.lines().any(|l| l == line), "{line} in\n{text}");
        }
    }

    #[test]
    fn paths_with_spaces_and_reserved_characters_are_quoted() {
        assert_eq!(
            exec_line(Path::new("/opt/My Apps/hab app")),
            "\"/opt/My Apps/hab app\" --hidden"
        );
        assert_eq!(
            exec_line(Path::new("/opt/a\"b$c")),
            "\"/opt/a\\\"b\\$c\" --hidden"
        );
        assert_eq!(exec_line(Path::new("/opt/100%")), "/opt/100%% --hidden");
    }

    #[test]
    fn enable_and_disable_come_and_go_with_the_file() {
        let d = dir("toggle");
        let a = Autostart::new(d.join("config/autostart"), "/usr/bin/hab-app".into());
        assert!(!a.is_enabled());
        a.disable().expect("disabling what isn't there is fine");
        a.enable().unwrap();
        assert!(a.is_enabled());
        assert_eq!(
            fs::read_to_string(a.path()).unwrap(),
            entry(Path::new("/usr/bin/hab-app"))
        );
        // Twice is fine.
        a.set(true).unwrap();
        assert!(a.is_enabled());
        a.set(false).unwrap();
        assert!(!a.is_enabled());
        assert!(!a.path().exists());
        // No temporary file is left behind.
        assert_eq!(fs::read_dir(d.join("config/autostart")).unwrap().count(), 0);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn an_entry_switched_off_by_hand_counts_as_off() {
        let d = dir("hidden");
        let a = Autostart::new(d.clone(), "/usr/bin/hab-app".into());
        fs::create_dir_all(&d).unwrap();
        fs::write(a.path(), format!("{}Hidden=true\n", entry(Path::new("/x")))).unwrap();
        assert!(!a.is_enabled());
        assert!(!a.repair().unwrap(), "left as the user set it");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn repair_follows_the_program_when_it_moves() {
        let d = dir("repair");
        let old = Autostart::new(d.clone(), "/old/hab-app".into());
        let new = Autostart::new(d.clone(), "/new/hab-app".into());
        assert!(!new.repair().unwrap(), "nothing to repair when off");
        assert!(!new.path().exists());
        old.enable().unwrap();
        assert!(!old.repair().unwrap(), "already right");
        assert!(new.repair().unwrap());
        assert!(fs::read_to_string(new.path())
            .unwrap()
            .contains("Exec=/new/hab-app --hidden"));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn the_directory_follows_xdg_config_home() {
        // Both variables are process-wide; this is the only test touching them.
        let saved = (
            std::env::var_os("XDG_CONFIG_HOME"),
            std::env::var_os("HOME"),
        );
        std::env::set_var("XDG_CONFIG_HOME", "/custom/config");
        assert_eq!(default_dir(), Some("/custom/config/autostart".into()));
        std::env::set_var("XDG_CONFIG_HOME", "relative/ignored");
        std::env::set_var("HOME", "/home/me");
        assert_eq!(default_dir(), Some("/home/me/.config/autostart".into()));
        std::env::remove_var("XDG_CONFIG_HOME");
        assert_eq!(default_dir(), Some("/home/me/.config/autostart".into()));
        for (k, v) in [("XDG_CONFIG_HOME", saved.0), ("HOME", saved.1)] {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}
