//! The nightly backup: SQLite's online backup into a folder, while the server
//! keeps serving. Restoring is putting a file back as `hab-server.db`.

use rusqlite::backup::Backup;
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::time::Duration;

const PREFIX: &str = "hab-server-";
const SUFFIX: &str = ".db";

/// A time of day in UTC, such as the 03:00 default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    pub hour: u32,
    pub minute: u32,
}

impl TimeOfDay {
    pub const DEFAULT: TimeOfDay = TimeOfDay { hour: 3, minute: 0 };

    pub fn parse(text: &str) -> Result<TimeOfDay, String> {
        let bad = || format!("bad backup time `{text}`: use HH:MM, 24-hour, UTC");
        let (h, m) = text.trim().split_once(':').ok_or_else(bad)?;
        let (hour, minute): (u32, u32) =
            (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
        if hour > 23 || minute > 59 {
            return Err(bad());
        }
        Ok(TimeOfDay { hour, minute })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupConfig {
    pub dir: PathBuf,
    pub at: TimeOfDay,
    /// How many backups to keep; older ones are deleted.
    pub keep: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("cannot use backup folder {0}: {1}")]
    Folder(PathBuf, std::io::Error),
    #[error("backup failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("backup failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// The file a backup taken at `unix` seconds is written to.
pub fn file_name(unix: i64) -> String {
    let (y, m, d) = civil(unix.div_euclid(86_400));
    format!("{PREFIX}{y:04}-{m:02}-{d:02}{SUFFIX}")
}

fn is_backup_name(name: &str) -> bool {
    let Some(date) = name
        .strip_prefix(PREFIX)
        .and_then(|n| n.strip_suffix(SUFFIX))
    else {
        return false;
    };
    let b = date.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// How long until the next `at` (UTC), counted from `now` in unix seconds. A
/// time that is exactly now means the next day, so one backup never runs twice.
pub fn delay_until(now: i64, at: TimeOfDay) -> Duration {
    let target = i64::from(at.hour) * 3600 + i64::from(at.minute) * 60;
    let mut wait = target - now.rem_euclid(86_400);
    if wait <= 0 {
        wait += 86_400;
    }
    Duration::from_secs(wait as u64)
}

/// Copy the database at `db_path` into `dir` as that day's backup, then delete
/// backups beyond the newest `keep`. It reads through its own connection, so
/// the server's connection stays free; SQLite copies a few pages at a time and
/// starts over if a write lands meanwhile. Returns the file written.
pub fn run_once(db_path: &Path, dir: &Path, keep: usize, now: i64) -> Result<PathBuf, BackupError> {
    std::fs::create_dir_all(dir).map_err(|e| BackupError::Folder(dir.to_path_buf(), e))?;
    let name = file_name(now);
    let target = dir.join(&name);
    let partial = dir.join(format!("{name}.partial"));
    let _ = std::fs::remove_file(&partial);

    let result = (|| -> Result<(), BackupError> {
        let source = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        source.busy_timeout(Duration::from_secs(30))?;
        let mut copy = Connection::open(&partial)?;
        #[cfg(unix)]
        {
            // The database holds the certificate's private key.
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600))?;
        }
        {
            let backup = Backup::new(&source, &mut copy)?;
            backup.run_to_completion(64, Duration::from_millis(5), None)?;
        }
        let check: String = copy.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(BackupError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
                Some(format!("the copy failed its integrity check: {check}")),
            )));
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    std::fs::rename(&partial, &target)?;
    prune(dir, keep);
    Ok(target)
}

/// Delete all but the newest `keep` backups. Only files named like ours.
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_backup_name(n))
        .collect();
    names.sort(); // dates sort as text
    let extra = names.len().saturating_sub(keep.max(1));
    for name in names.into_iter().take(extra) {
        if let Err(e) = std::fs::remove_file(dir.join(&name)) {
            tracing::warn!("cannot delete old backup {name}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, FILE_NAME};

    #[test]
    fn names_follow_the_utc_date() {
        assert_eq!(file_name(0), "hab-server-1970-01-01.db");
        assert_eq!(file_name(1_709_251_199), "hab-server-2024-02-29.db");
        assert_eq!(file_name(1_709_251_200), "hab-server-2024-03-01.db");
        assert!(is_backup_name("hab-server-2024-03-01.db"));
        assert!(!is_backup_name("hab-server.db"));
        assert!(!is_backup_name("hab-server-2024-03-01.db.partial"));
        assert!(!is_backup_name("notes.db"));
    }

    #[test]
    fn parses_times_of_day() {
        assert_eq!(
            TimeOfDay::parse("03:30").unwrap(),
            TimeOfDay {
                hour: 3,
                minute: 30
            }
        );
        assert!(TimeOfDay::parse("24:00").is_err());
        assert!(TimeOfDay::parse("3").is_err());
        assert!(TimeOfDay::parse("aa:bb").is_err());
    }

    #[test]
    fn waits_until_the_next_occurrence_of_the_time() {
        let at = TimeOfDay { hour: 3, minute: 0 };
        assert_eq!(delay_until(0, at), Duration::from_secs(3 * 3600));
        // 04:00 -> 23 hours; exactly 03:00 -> a full day.
        assert_eq!(delay_until(4 * 3600, at), Duration::from_secs(23 * 3600));
        assert_eq!(delay_until(3 * 3600, at), Duration::from_secs(86_400));
    }

    #[test]
    fn copies_the_database_and_keeps_only_the_newest() {
        let data = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let db = Db::open(data.path()).unwrap();
        db.set("server_name", "Home").unwrap();
        let path = data.path().join(FILE_NAME);

        let day = 86_400;
        for n in 0..4 {
            run_once(&path, backups.path(), 2, 1_700_000_000 + n * day).unwrap();
        }
        // The database is still open and usable afterwards.
        db.set("server_name", "Later").unwrap();

        let mut names: Vec<String> = std::fs::read_dir(backups.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names.len(), 2, "{names:?}");
        assert_eq!(names[1], file_name(1_700_000_000 + 3 * day));

        // Restoring is putting the file back as the database.
        let restore = tempfile::tempdir().unwrap();
        std::fs::copy(
            backups.path().join(&names[1]),
            restore.path().join(FILE_NAME),
        )
        .unwrap();
        let restored = Db::open(restore.path()).unwrap();
        assert_eq!(
            restored.get("server_name").unwrap().as_deref(),
            Some("Home")
        );
    }

    #[test]
    fn a_failed_backup_leaves_no_partial_file() {
        let data = tempfile::tempdir().unwrap();
        let backups = tempfile::tempdir().unwrap();
        let missing = data.path().join("nope.db");
        assert!(run_once(&missing, backups.path(), 2, 0).is_err());
        assert_eq!(std::fs::read_dir(backups.path()).unwrap().count(), 0);
    }
}
