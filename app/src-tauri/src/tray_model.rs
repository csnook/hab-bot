//! What the tray shows, decided from the Inbox: the badge and the menu.
//!
//! Pure functions on [`hab_core::Inbox`], so they are tested without a desktop
//! (the tray's D-Bus side is in [`crate::tray`], the badge's picture in
//! [`crate::badge`]).
//!
//! ## What the badge counts
//!
//! From the spec (Desktop → The tray): due and overdue occurrences of Low
//! priority and above; red when something Medium or above is overdue, blue
//! otherwise; Minimum and waiting reminders never count.
//!
//! Decided here, where the spec is silent:
//! - **The window's filters don't apply.** `Core::inbox` is unfiltered on
//!   purpose and so is the tray: hiding a list in the sidebar "only hides it
//!   from the window. Its alerts still come", and the badge is the alerts'
//!   companion, not the window's.
//! - **A snoozed occurrence still counts.** It is open and in the Inbox's Due
//!   (or Overdue) section until it closes; the badge says how many are open,
//!   not how many are currently ringing. Its menu row says when the snooze
//!   ends.
//! - **Paused reminders don't count and aren't listed.** An open occurrence of
//!   a paused reminder is in `Inbox::paused`, not in Overdue or Due, so the
//!   badge, the tooltip and the menu's rows leave it out: pausing is for being
//!   left alone. They aren't in the Waiting section either, which is for
//!   reminders whose conditions aren't met; the Board's Paused column
//!   ([`hab_core::Core::paused_reminders`]) is where they show.
//! - **Time-based conditions never put anything in Waiting.** An instant
//!   outside them (weekdays only, on a Saturday) passes: no occurrence opens,
//!   so the badge, the tooltip and the menu see nothing, and the Waiting
//!   section stays empty until conditions that can't be predicted (places,
//!   weather) exist.
//! - **Snooze all doesn't lower the badge.** A held occurrence is open and
//!   still counts, exactly as a single snoozed one does (see above); the
//!   badge says how many are open, and the toolbar chip and the menu's
//!   "End snooze all" say that they are being held.
//! - **The menu's Snooze all offers lengths, not everything the dialog
//!   does.** Everything, Maximum left out, for 30 minutes, an hour, two hours
//!   or until tomorrow morning; "More choices…" opens the dialog for one list,
//!   a time of day or "include maximum". While one holds, each is listed to
//!   end early. Quiet hours aren't in the menu: they are set in Settings.
//! - **Quiet this device is a submenu after Snooze all,** with the same
//!   lengths, Maximum left out. It is not a snooze, so nothing in the menu's
//!   rows or the badge changes with it. While it is in force its first entry
//!   ends it; "More choices…" opens Settings → This device, which also has a
//!   time of day and "include maximum".
//! - **Overdue means `overdue_at <= now`,** the Inbox's own test.

use hab_core::{
    DeviceQuiet, DueItem, Inbox, Priority, Scope, SnoozeAllChoice, SnoozeAllView, Source,
};

/// The badge's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Something Medium or above is overdue.
    Red,
    Blue,
}

/// The number on the tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge {
    /// Due and overdue occurrences of Low priority and above. Zero shows no
    /// badge.
    pub count: usize,
    pub tone: Tone,
}

/// Whether an occurrence of this priority counts towards the badge.
pub fn counts(priority: Priority) -> bool {
    priority >= Priority::Low
}

fn open_items(inbox: &Inbox) -> impl Iterator<Item = &DueItem> {
    inbox.overdue.iter().chain(inbox.due.iter())
}

/// The badge for an Inbox.
pub fn badge(inbox: &Inbox) -> Badge {
    let count = open_items(inbox).filter(|d| counts(d.priority)).count();
    // `inbox.overdue` is the Inbox's own split by `overdue_at <= now`.
    let red = inbox.overdue.iter().any(|d| d.priority >= Priority::Medium);
    Badge {
        count,
        tone: if red { Tone::Red } else { Tone::Blue },
    }
}

/// The tooltip: "3 due, 1 overdue" or "Nothing due".
pub fn tooltip(inbox: &Inbox) -> String {
    let overdue = inbox.overdue.iter().filter(|d| counts(d.priority)).count();
    let due = inbox.due.iter().filter(|d| counts(d.priority)).count();
    match (due, overdue) {
        (0, 0) => "Nothing due".to_string(),
        (d, 0) => format!("{d} due"),
        (0, o) => format!("{o} overdue"),
        (d, o) => format!("{d} due, {o} overdue"),
    }
}

/// A reminder that is waiting for its conditions. Conditions aren't built
/// yet, so nothing supplies these today; the menu is ready for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingItem {
    pub reminder_id: String,
    pub title: String,
}

/// What choosing a menu entry does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    /// Left-click, and Open Reminders.
    OpenWindow,
    /// Open the window at an occurrence (or, for a waiting reminder, at its
    /// reminder).
    OpenOccurrence(String),
    Done(String),
    /// One choice: the priority's snooze length, as the notification's
    /// Snooze button does.
    Snooze(String),
    /// The third button. Today every occurrence is personal, so it is Skip;
    /// Claim and Release come with sharing.
    Skip(String),
    /// Snooze everything, Maximum left out, for a length.
    SnoozeAll(SnoozeAllChoice),
    /// End the snooze-all with this id early.
    EndSnoozeAll(String),
    /// Open the window at the Snooze all dialog.
    SnoozeAllDialog,
    /// Quiet this device for a length, Maximum left out. Not a snooze: other
    /// devices still alert.
    QuietDevice(SnoozeAllChoice),
    /// End "Quiet this device" early.
    EndQuietDevice,
    /// Open Settings → This device, for a time of day or to include Maximum.
    QuietDeviceDialog,
    Settings,
    Quit,
}

/// One entry of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Separator,
    /// A heading that can't be chosen, such as "Waiting".
    Heading(String),
    Item {
        label: String,
        action: TrayAction,
    },
    /// An occurrence's row, opening to its buttons.
    Submenu {
        label: String,
        children: Vec<Entry>,
    },
}

/// How many occurrences get their own row before "N more…".
pub const MAX_ROWS: usize = 12;
/// Longest title shown, in characters.
const MAX_TITLE: usize = 40;

fn shorten(title: &str) -> String {
    let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.chars().count() <= MAX_TITLE {
        return title;
    }
    let mut s: String = title.chars().take(MAX_TITLE - 1).collect();
    s.push('…');
    s
}

fn row(item: &DueItem, overdue: bool, now: i64, clock: &dyn Fn(i64) -> String) -> Entry {
    let mut label = shorten(&item.title);
    if let Some(until) = item.snoozed_until.filter(|&u| u > now) {
        label.push_str(&format!(" · snoozed until {}", clock(until)));
    } else if overdue {
        label.push_str(" · overdue");
    }
    let id = &item.occurrence_id;
    Entry::Submenu {
        label,
        children: vec![
            Entry::Item {
                label: "Done".into(),
                action: TrayAction::Done(id.clone()),
            },
            Entry::Item {
                label: "Snooze".into(),
                action: TrayAction::Snooze(id.clone()),
            },
            Entry::Item {
                label: "Skip".into(),
                action: TrayAction::Skip(id.clone()),
            },
        ],
    }
}

/// The Snooze all submenu: the ones holding now to end early, then the
/// lengths, then the dialog.
fn snooze_all_menu(holding: &[SnoozeAllView], clock: &dyn Fn(i64) -> String) -> Entry {
    let mut children = Vec::new();
    for h in holding.iter().filter(|h| h.source == Source::SnoozeAll) {
        let Some(id) = &h.id else { continue };
        let what = match (&h.scope, &h.list_name) {
            (Scope::All, _) => "All".to_string(),
            (Scope::List(_), Some(name)) => shorten(name),
            (Scope::List(_), None) => "List".to_string(),
        };
        children.push(Entry::Item {
            label: format!("End: {what} snoozed until {}", clock(h.until)),
            action: TrayAction::EndSnoozeAll(id.clone()),
        });
    }
    if !children.is_empty() {
        children.push(Entry::Separator);
    }
    for (label, choice) in [
        ("For 30 minutes", SnoozeAllChoice::Minutes(30)),
        ("For 1 hour", SnoozeAllChoice::Minutes(60)),
        ("For 2 hours", SnoozeAllChoice::Minutes(120)),
        ("Until tomorrow morning", SnoozeAllChoice::TomorrowMorning),
    ] {
        children.push(Entry::Item {
            label: label.into(),
            action: TrayAction::SnoozeAll(choice),
        });
    }
    children.push(Entry::Separator);
    children.push(Entry::Item {
        label: "More choices…".into(),
        action: TrayAction::SnoozeAllDialog,
    });
    Entry::Submenu {
        label: "Snooze all".into(),
        children,
    }
}

/// The Quiet this device submenu: ending the one in force, then the
/// lengths, then Settings → This device for the rest.
fn quiet_device_menu(quiet: Option<DeviceQuiet>, clock: &dyn Fn(i64) -> String) -> Entry {
    let mut children = Vec::new();
    if let Some(q) = quiet {
        children.push(Entry::Item {
            label: format!(
                "End: quiet until {}{}",
                clock(q.until),
                if q.include_maximum {
                    ", Maximum too"
                } else {
                    ""
                }
            ),
            action: TrayAction::EndQuietDevice,
        });
        children.push(Entry::Separator);
    }
    for (label, choice) in [
        ("For 30 minutes", SnoozeAllChoice::Minutes(30)),
        ("For 1 hour", SnoozeAllChoice::Minutes(60)),
        ("For 2 hours", SnoozeAllChoice::Minutes(120)),
        ("Until tomorrow morning", SnoozeAllChoice::TomorrowMorning),
    ] {
        children.push(Entry::Item {
            label: label.into(),
            action: TrayAction::QuietDevice(choice),
        });
    }
    children.push(Entry::Separator);
    children.push(Entry::Item {
        label: "More choices…".into(),
        action: TrayAction::QuietDeviceDialog,
    });
    Entry::Submenu {
        label: "Quiet this device until…".into(),
        children,
    }
}

/// The menu: Open Reminders; each open occurrence as a submenu with Done,
/// Snooze and its third button (overdue first, highest priority first, then
/// the Inbox's Due order); a Waiting section; Snooze all; Quiet this device
/// until…; Settings; Quit.
///
/// Every open occurrence is listed, Minimum included (only the badge skips
/// those). `clock` writes a time of day for "snoozed until".
///
/// `quiet` is "Quiet this device" if in force, to be ended from the menu.
pub fn menu(
    inbox: &Inbox,
    waiting: &[WaitingItem],
    holding: &[SnoozeAllView],
    quiet: Option<DeviceQuiet>,
    now: i64,
    clock: &dyn Fn(i64) -> String,
) -> Vec<Entry> {
    let mut entries = vec![Entry::Item {
        label: "Open Reminders".into(),
        action: TrayAction::OpenWindow,
    }];
    let rows: Vec<Entry> = inbox
        .overdue
        .iter()
        .map(|d| row(d, true, now, clock))
        .chain(inbox.due.iter().map(|d| row(d, false, now, clock)))
        .collect();
    if !rows.is_empty() {
        entries.push(Entry::Separator);
        let more = rows.len().saturating_sub(MAX_ROWS);
        entries.extend(rows.into_iter().take(MAX_ROWS));
        if more > 0 {
            entries.push(Entry::Item {
                label: format!("{more} more…"),
                action: TrayAction::OpenWindow,
            });
        }
    }
    if !waiting.is_empty() {
        entries.push(Entry::Separator);
        entries.push(Entry::Heading("Waiting".into()));
        for w in waiting.iter().take(MAX_ROWS) {
            entries.push(Entry::Item {
                label: shorten(&w.title),
                action: TrayAction::OpenOccurrence(w.reminder_id.clone()),
            });
        }
    }
    entries.push(Entry::Separator);
    entries.push(snooze_all_menu(holding, clock));
    entries.push(quiet_device_menu(quiet, clock));
    entries.push(Entry::Item {
        label: "Settings".into(),
        action: TrayAction::Settings,
    });
    entries.push(Entry::Item {
        label: "Quit".into(),
        action: TrayAction::Quit,
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, priority: Priority) -> DueItem {
        DueItem {
            occurrence_id: id.into(),
            reminder_id: format!("r-{id}"),
            list_id: "l".into(),
            title: format!("Reminder {id}"),
            note: String::new(),
            scheduled_at: 0,
            fired_at: 0,
            not_sent: false,
            snoozed_until: None,
            snoozed_at: None,
            acknowledged: false,
            acknowledged_at: None,
            priority,
            overdue_at: 0,
            expires_at: None,
        }
    }

    fn inbox(overdue: Vec<DueItem>, due: Vec<DueItem>) -> Inbox {
        Inbox {
            overdue,
            due,
            later_today: vec![],
            earlier_today: vec![],
            paused: vec![],
            folded: vec![],
        }
    }

    fn clock(t: i64) -> String {
        format!("t{t}")
    }

    /// The submenus that are an occurrence's row (not Snooze all).
    const QUIET_LABEL: &str = "Quiet this device until…";

    fn rows(m: &[Entry]) -> Vec<&Entry> {
        m.iter()
            .filter(|e| matches!(e, Entry::Submenu { label, .. } if label != "Snooze all" && label != QUIET_LABEL))
            .collect()
    }

    #[test]
    fn folding_does_not_change_the_badge_or_the_tooltip() {
        let mut i = inbox(
            vec![item("a", Priority::Low), item("b", Priority::Low)],
            vec![],
        );
        let (badge_before, tip_before) = (badge(&i), tooltip(&i));
        i.folded = vec!["a".into(), "b".into()];
        assert_eq!(badge(&i), badge_before);
        assert_eq!(tooltip(&i), tip_before);
        assert_eq!(badge(&i).count, 2);
    }

    #[test]
    fn a_paused_occurrence_is_not_counted_or_listed() {
        let mut i = inbox(vec![], vec![item("a", Priority::Low)]);
        i.paused.push(hab_core::PausedOpen {
            item: item("p", Priority::Maximum),
            pause: hab_core::PauseCause {
                until: Some(99),
                list: false,
            },
        });
        // Even a Maximum that is long overdue doesn't redden the badge.
        i.paused[0].item.overdue_at = -1000;
        let b = badge(&i);
        assert_eq!((b.count, b.tone), (1, Tone::Blue));
        assert_eq!(tooltip(&i), "1 due");
        let m = menu(&i, &[], &[], None, 10, &clock);
        assert_eq!(rows(&m).len(), 1);
        assert!(!m.contains(&Entry::Heading("Waiting".into())));
    }

    #[test]
    fn an_instant_outside_a_time_based_condition_leaves_the_tray_alone() {
        use hab_core::{Condition, Core, Schedule};
        // 2026-10-02 is a Friday: these are its 06:00, and 09:00 on the
        // Friday, Saturday and Monday.
        let (start, friday, saturday, monday) = (1790920800, 1790931600, 1791018000, 1791190800);
        let mut c = Core::open_in_memory().unwrap();
        c.set_device_zone("UTC").unwrap();
        let list = c.personal_list_id().to_string();
        c.create_repeating_reminder_in(
            &list,
            "Bins",
            vec![Schedule {
                start: "2026-10-01T09:00:00".into(),
                rule: "FREQ=DAILY".into(),
            }],
            vec![],
            vec![Condition::Days {
                days: vec!["MO".into()],
            }],
            Some("UTC"),
            start,
        )
        .unwrap();
        // Friday and Saturday 09:00 pass: nothing opens, so nothing shows.
        for t in [friday, saturday] {
            assert!(c.tick(t).unwrap().is_empty());
            let i = c.inbox(t);
            let b = badge(&i);
            assert_eq!((b.count, b.tone), (0, Tone::Blue));
            assert_eq!(tooltip(&i), "Nothing due");
            let m = menu(&i, &[], &[], None, t, &clock);
            assert!(rows(&m).is_empty());
            assert!(!m.contains(&Entry::Heading("Waiting".into())));
        }
        // Monday it fires and counts like any occurrence.
        assert_eq!(c.tick(monday).unwrap().len(), 1);
        let i = c.inbox(monday);
        assert_eq!(badge(&i).count, 1);
        assert_eq!(tooltip(&i), "1 due");
    }

    #[test]
    fn nothing_open_is_a_blue_zero() {
        let b = badge(&inbox(vec![], vec![]));
        assert_eq!((b.count, b.tone), (0, Tone::Blue));
        assert_eq!(tooltip(&inbox(vec![], vec![])), "Nothing due");
    }

    #[test]
    fn counts_low_and_above_due_and_overdue_and_never_minimum() {
        let i = inbox(
            vec![item("a", Priority::Low), item("b", Priority::Minimum)],
            vec![
                item("c", Priority::Minimum),
                item("d", Priority::Low),
                item("e", Priority::Maximum),
            ],
        );
        assert_eq!(badge(&i).count, 3);
        assert_eq!(tooltip(&i), "2 due, 1 overdue");
    }

    #[test]
    fn red_only_when_medium_or_above_is_overdue() {
        // Overdue Low, and a Maximum that is merely due: blue.
        let i = inbox(
            vec![item("a", Priority::Low)],
            vec![item("b", Priority::Maximum)],
        );
        assert_eq!(badge(&i).tone, Tone::Blue);
        // Overdue Minimum doesn't count at all, so doesn't redden.
        let i = inbox(vec![item("a", Priority::Minimum)], vec![]);
        assert_eq!(badge(&i).tone, Tone::Blue);
        for p in [Priority::Medium, Priority::High, Priority::Maximum] {
            let i = inbox(vec![item("a", p)], vec![]);
            assert_eq!(badge(&i).tone, Tone::Red, "{p:?}");
        }
    }

    #[test]
    fn a_snoozed_occurrence_still_counts() {
        let mut s = item("a", Priority::Medium);
        s.snoozed_until = Some(100);
        assert_eq!(badge(&inbox(vec![], vec![s])).count, 1);
    }

    #[test]
    fn menu_has_the_fixed_entries_and_a_submenu_per_open_occurrence() {
        let i = inbox(
            vec![item("o", Priority::High)],
            vec![item("d", Priority::Minimum)],
        );
        let m = menu(&i, &[], &[], None, 10, &clock);
        assert_eq!(
            m.first(),
            Some(&Entry::Item {
                label: "Open Reminders".into(),
                action: TrayAction::OpenWindow
            })
        );
        let rows = rows(&m);
        assert_eq!(rows.len(), 2, "Minimum is listed too");
        let Entry::Submenu { label, children } = rows[0] else {
            unreachable!()
        };
        assert_eq!(label, "Reminder o · overdue");
        let names: Vec<_> = children
            .iter()
            .map(|c| match c {
                Entry::Item { label, .. } => label.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(names, ["Done", "Snooze", "Skip"]);
        assert_eq!(
            children[0],
            Entry::Item {
                label: "Done".into(),
                action: TrayAction::Done("o".into())
            }
        );
        let n = m.len();
        assert_eq!(
            &m[n - 3..],
            &[
                m[n - 3].clone(),
                Entry::Item {
                    label: "Settings".into(),
                    action: TrayAction::Settings
                },
                Entry::Item {
                    label: "Quit".into(),
                    action: TrayAction::Quit
                }
            ]
        );
        // No waiting section when nothing waits.
        assert!(!m.contains(&Entry::Heading("Waiting".into())));
    }

    #[test]
    fn waiting_reminders_get_a_section_before_settings() {
        let w = [WaitingItem {
            reminder_id: "r1".into(),
            title: "Water the plants".into(),
        }];
        let m = menu(&inbox(vec![], vec![]), &w, &[], None, 0, &clock);
        let at = m
            .iter()
            .position(|e| *e == Entry::Heading("Waiting".into()))
            .unwrap();
        assert_eq!(
            m[at + 1],
            Entry::Item {
                label: "Water the plants".into(),
                action: TrayAction::OpenOccurrence("r1".into())
            }
        );
        assert!(
            m.iter()
                .position(|e| matches!(
                    e,
                    Entry::Item {
                        action: TrayAction::Settings,
                        ..
                    }
                ))
                .unwrap()
                > at
        );
    }

    #[test]
    fn a_snoozed_row_says_until_when_and_long_titles_are_cut() {
        let mut s = item("a", Priority::Medium);
        s.snoozed_until = Some(500);
        s.title = "x".repeat(100);
        let m = menu(&inbox(vec![], vec![s]), &[], &[], None, 10, &clock);
        let Entry::Submenu { label, .. } = &m[2] else {
            panic!("{m:?}")
        };
        assert!(label.ends_with("… · snoozed until t500"), "{label}");
        assert!(label.chars().count() < 70);
    }

    #[test]
    fn a_long_list_ends_in_n_more_which_opens_the_window() {
        let many: Vec<DueItem> = (0..15)
            .map(|n| item(&n.to_string(), Priority::Low))
            .collect();
        let m = menu(&inbox(vec![], many), &[], &[], None, 0, &clock);
        assert_eq!(rows(&m).len(), MAX_ROWS);
        assert!(m.contains(&Entry::Item {
            label: "3 more…".into(),
            action: TrayAction::OpenWindow
        }));
    }

    fn holding_view(
        id: Option<&str>,
        source: Source,
        scope: Scope,
        name: Option<&str>,
    ) -> SnoozeAllView {
        SnoozeAllView {
            id: id.map(String::from),
            source,
            scope,
            list_name: name.map(String::from),
            include_maximum: false,
            from: 0,
            until: 900,
        }
    }

    fn snooze_all_children(m: &[Entry]) -> Vec<Entry> {
        m.iter()
            .find_map(|e| match e {
                Entry::Submenu { label, children } if label == "Snooze all" => {
                    Some(children.clone())
                }
                _ => None,
            })
            .expect("a Snooze all submenu")
    }

    #[test]
    fn snooze_all_sits_between_waiting_and_settings_with_the_lengths_and_the_dialog() {
        let w = [WaitingItem {
            reminder_id: "r1".into(),
            title: "Water".into(),
        }];
        let m = menu(&inbox(vec![], vec![]), &w, &[], None, 0, &clock);
        let at = |f: &dyn Fn(&Entry) -> bool| m.iter().position(f).unwrap();
        let waiting = at(&|e| *e == Entry::Heading("Waiting".into()));
        let snooze = at(&|e| matches!(e, Entry::Submenu { label, .. } if label == "Snooze all"));
        let settings = at(&|e| {
            matches!(
                e,
                Entry::Item {
                    action: TrayAction::Settings,
                    ..
                }
            )
        });
        let quiet = at(&|e| matches!(e, Entry::Submenu { label, .. } if label == QUIET_LABEL));
        // Quiet this device follows Snooze all and comes before Settings.
        assert!(waiting < snooze && snooze + 1 == quiet && quiet + 1 == settings);
        let actions: Vec<TrayAction> = snooze_all_children(&m)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item { action, .. } => Some(action),
                _ => None,
            })
            .collect();
        assert_eq!(
            actions,
            [
                TrayAction::SnoozeAll(SnoozeAllChoice::Minutes(30)),
                TrayAction::SnoozeAll(SnoozeAllChoice::Minutes(60)),
                TrayAction::SnoozeAll(SnoozeAllChoice::Minutes(120)),
                TrayAction::SnoozeAll(SnoozeAllChoice::TomorrowMorning),
                TrayAction::SnoozeAllDialog,
            ]
        );
    }

    #[test]
    fn a_snooze_all_holding_is_listed_to_end_early_but_quiet_hours_are_not() {
        let holding = [
            holding_view(Some("s1"), Source::SnoozeAll, Scope::All, None),
            holding_view(
                Some("s2"),
                Source::SnoozeAll,
                Scope::List("l".into()),
                Some("Household"),
            ),
            holding_view(None, Source::QuietHours, Scope::All, None),
        ];
        let m = menu(&inbox(vec![], vec![]), &[], &holding, None, 0, &clock);
        let children = snooze_all_children(&m);
        assert_eq!(
            children[..3],
            [
                Entry::Item {
                    label: "End: All snoozed until t900".into(),
                    action: TrayAction::EndSnoozeAll("s1".into())
                },
                Entry::Item {
                    label: "End: Household snoozed until t900".into(),
                    action: TrayAction::EndSnoozeAll("s2".into())
                },
                Entry::Separator,
            ]
        );
        let ends = children
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Entry::Item {
                        action: TrayAction::EndSnoozeAll(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(ends, 2);
    }

    #[test]
    fn snooze_all_leaves_the_badge_and_tooltip_counting_what_is_held() {
        let mut held = item("a", Priority::High);
        held.snoozed_until = Some(1_000);
        let i = inbox(vec![held], vec![]);
        assert_eq!(badge(&i).count, 1);
        assert_eq!(tooltip(&i), "1 overdue");
    }

    #[test]
    fn a_snooze_all_in_the_core_shows_in_the_menu_and_on_the_row() {
        use hab_core::Core;
        let t = 1_790_000_000;
        let mut c = Core::open_in_memory().unwrap();
        c.set_device_zone("UTC").unwrap();
        let id = c.create_reminder("Bins", t, t - 100).unwrap();
        c.tick(t).unwrap();
        let sa = c.snooze_all(Scope::All, t + 3_600, false, t + 1).unwrap();
        let i = c.inbox(t + 2);
        let m = menu(&i, &[], &c.holding(t + 2), None, t + 2, &clock);
        let Entry::Submenu { label, .. } = &m[2] else {
            panic!("{m:?}")
        };
        assert!(label.contains("snoozed until"), "{label}");
        assert!(snooze_all_children(&m).contains(&Entry::Item {
            label: format!("End: All snoozed until t{}", t + 3_600),
            action: TrayAction::EndSnoozeAll(sa)
        }));
        let _ = id;
    }

    fn quiet_children(m: &[Entry]) -> Vec<Entry> {
        m.iter()
            .find_map(|e| match e {
                Entry::Submenu { label, children } if label == QUIET_LABEL => {
                    Some(children.clone())
                }
                _ => None,
            })
            .expect("a Quiet this device submenu")
    }

    #[test]
    fn quiet_this_device_offers_lengths_and_the_settings_dialog() {
        let m = menu(&inbox(vec![], vec![]), &[], &[], None, 0, &clock);
        let actions: Vec<TrayAction> = quiet_children(&m)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item { action, .. } => Some(action),
                _ => None,
            })
            .collect();
        assert_eq!(
            actions,
            [
                TrayAction::QuietDevice(SnoozeAllChoice::Minutes(30)),
                TrayAction::QuietDevice(SnoozeAllChoice::Minutes(60)),
                TrayAction::QuietDevice(SnoozeAllChoice::Minutes(120)),
                TrayAction::QuietDevice(SnoozeAllChoice::TomorrowMorning),
                TrayAction::QuietDeviceDialog,
            ]
        );
    }

    #[test]
    fn while_quiet_the_first_entry_ends_it() {
        let quiet = DeviceQuiet {
            until: 900,
            include_maximum: false,
        };
        let m = menu(&inbox(vec![], vec![]), &[], &[], Some(quiet), 0, &clock);
        let children = quiet_children(&m);
        assert_eq!(
            children[..2],
            [
                Entry::Item {
                    label: "End: quiet until t900".into(),
                    action: TrayAction::EndQuietDevice
                },
                Entry::Separator
            ]
        );
        let m = menu(
            &inbox(vec![], vec![]),
            &[],
            &[],
            Some(DeviceQuiet {
                include_maximum: true,
                ..quiet
            }),
            0,
            &clock,
        );
        assert!(matches!(&quiet_children(&m)[0],
            Entry::Item { label, .. } if label == "End: quiet until t900, Maximum too"));
        // Not quiet: nothing to end.
        let m = menu(&inbox(vec![], vec![]), &[], &[], None, 0, &clock);
        assert!(!quiet_children(&m).iter().any(|e| matches!(
            e,
            Entry::Item {
                action: TrayAction::EndQuietDevice,
                ..
            }
        )));
    }

    #[test]
    fn quiet_this_device_is_not_a_snooze_so_rows_and_the_badge_are_unchanged() {
        use hab_core::Core;
        let t = 1_790_000_000;
        let mut c = Core::open_in_memory().unwrap();
        c.set_device_zone("UTC").unwrap();
        c.create_reminder("Bins", t, t - 100).unwrap();
        c.tick(t).unwrap();
        let before = menu(&c.inbox(t + 1), &[], &c.holding(t + 1), None, t + 1, &clock);
        c.quiet_device_for(SnoozeAllChoice::Minutes(60), false, t + 1)
            .unwrap();
        let i = c.inbox(t + 2);
        assert_eq!(badge(&i).count, 1);
        let m = menu(
            &i,
            &[],
            &c.holding(t + 2),
            c.device_quiet(t + 2),
            t + 2,
            &clock,
        );
        assert_eq!(rows(&m), rows(&before));
        assert!(matches!(&quiet_children(&m)[0],
            Entry::Item { action: TrayAction::EndQuietDevice, label } if label.starts_with("End: quiet until")));
    }
}
