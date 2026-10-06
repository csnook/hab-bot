//! What a launch of the app asks for, from its command line.
//!
//! The first launch and every later one are read the same way: the first
//! becomes the running app, and a later one is handed to it by the
//! single-instance plugin (`tauri-plugin-single-instance` owns the D-Bus name
//! `io.github.csnook.hab-bot.SingleInstance` and forwards the later launch's
//! arguments) and then exits.
//!
//! - `--hidden`: start in the tray with no window. Autostart uses it. A later
//!   launch with it (a second login entry, say) does nothing.
//! - `--open <occurrence>`: bring the window forward at that occurrence, as a
//!   click on its notification does. A launcher or script can use it.
//! - `--settings`: bring the window forward with Settings open.
//! - Nothing: bring the window forward. This is what launching the app from
//!   the menu twice does.

/// The argument autostart passes.
pub const HIDDEN_ARG: &str = "--hidden";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Launch {
    pub hidden: bool,
    pub open: Option<String>,
    pub settings: bool,
}

/// Reads a command line. The first element is the program, as in `argv`.
/// Anything not understood is ignored: a later launch must never fail.
pub fn parse(args: &[String]) -> Launch {
    let mut launch = Launch::default();
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            HIDDEN_ARG => launch.hidden = true,
            "--settings" => launch.settings = true,
            "--open" => launch.open = args.next().filter(|a| !a.is_empty()).cloned(),
            other => {
                if let Some(id) = other.strip_prefix("--open=").filter(|a| !a.is_empty()) {
                    launch.open = Some(id.to_string());
                }
            }
        }
    }
    launch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_plain_launch_asks_for_the_window() {
        assert_eq!(parse(&args(&["hab-app"])), Launch::default());
        assert_eq!(parse(&[]), Launch::default());
    }

    #[test]
    fn autostart_asks_for_no_window() {
        assert!(parse(&args(&["hab-app", "--hidden"])).hidden);
    }

    #[test]
    fn open_takes_an_occurrence_in_either_spelling() {
        assert_eq!(
            parse(&args(&["hab-app", "--open", "occ-1"]))
                .open
                .as_deref(),
            Some("occ-1")
        );
        assert_eq!(
            parse(&args(&["hab-app", "--open=occ-2"])).open.as_deref(),
            Some("occ-2")
        );
        assert_eq!(parse(&args(&["hab-app", "--open"])).open, None);
        assert_eq!(parse(&args(&["hab-app", "--open="])).open, None);
    }

    #[test]
    fn settings_and_unknown_arguments() {
        let l = parse(&args(&["hab-app", "--settings", "--what", "--hidden"]));
        assert!(l.settings && l.hidden && l.open.is_none());
    }

    #[test]
    fn the_program_name_is_never_an_argument() {
        assert_eq!(parse(&args(&["--hidden"])), Launch::default());
    }
}
