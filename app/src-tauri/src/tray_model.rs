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
//! - **Overdue means `overdue_at <= now`,** the Inbox's own test.

use hab_core::{DueItem, Inbox, Priority};

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

/// The menu: Open Reminders; each open occurrence as a submenu with Done,
/// Snooze and its third button (overdue first, highest priority first, then
/// the Inbox's Due order); a Waiting section; Settings; Quit.
///
/// Every open occurrence is listed, Minimum included (only the badge skips
/// those). `clock` writes a time of day for "snoozed until".
///
/// Snooze all and Quiet this device are added by their own tickets, between
/// Waiting and Settings (see the spec's menu order).
pub fn menu(
    inbox: &Inbox,
    waiting: &[WaitingItem],
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
        }
    }

    fn clock(t: i64) -> String {
        format!("t{t}")
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
        let m = menu(&i, &[], 10, &clock);
        assert_eq!(
            m.first(),
            Some(&Entry::Item {
                label: "Open Reminders".into(),
                action: TrayAction::OpenWindow
            })
        );
        let rows: Vec<&Entry> = m
            .iter()
            .filter(|e| matches!(e, Entry::Submenu { .. }))
            .collect();
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
            &m[n - 2..],
            &[
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
        let m = menu(&inbox(vec![], vec![]), &w, 0, &clock);
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
        let m = menu(&inbox(vec![], vec![s]), &[], 10, &clock);
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
        let m = menu(&inbox(vec![], many), &[], 0, &clock);
        let rows = m
            .iter()
            .filter(|e| matches!(e, Entry::Submenu { .. }))
            .count();
        assert_eq!(rows, MAX_ROWS);
        assert!(m.contains(&Entry::Item {
            label: "3 more…".into(),
            action: TrayAction::OpenWindow
        }));
    }
}
