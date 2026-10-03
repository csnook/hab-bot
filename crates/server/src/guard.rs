//! Protection against password guessing, and limits on unauthenticated calls.
//!
//! Everything here is in memory and lost on a restart, and IP addresses never
//! leave it. Times are a [`Clock`]: how long since the server started, so a
//! test can move it without waiting.
//!
//! An OPAQUE sign-in's password check happens on the client: a client that
//! guessed wrong sees it fail and never sends the last message. So the server
//! cannot wait to be told of a failure. Every sign-in start for an account is
//! counted as an attempt, and a successful sign-in wipes the count. An attempt
//! that is not followed by a success within [`FAIL_GRACE`] has failed, which
//! is when the notice for the user's devices counts it.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the server has been running, as seen by the guard.
pub type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

/// The real clock.
pub fn system_clock() -> Clock {
    let start = Instant::now();
    Arc::new(move || start.elapsed())
}

/// A clock that only moves when told to, for tests.
#[derive(Clone, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    pub fn new() -> ManualClock {
        ManualClock::default()
    }

    pub fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }

    pub fn clock(&self) -> Clock {
        let t = self.0.clone();
        Arc::new(move || Duration::from_millis(t.load(Ordering::SeqCst)))
    }
}

/// Failed attempts that cost nothing, per account.
pub const FREE_ATTEMPTS: u32 = 5;
/// The wait after the fifth failure, doubling with each one after, to the cap.
pub const FIRST_WAIT: Duration = Duration::from_secs(60);
pub const MAX_WAIT: Duration = Duration::from_secs(3600);
/// Password attempts one address may make in a minute.
pub const IP_ATTEMPTS_PER_MINUTE: usize = 20;
/// Calls one address may make a minute to the approvals' open, request,
/// collect and create routes. A new device polls twice a second, so this is
/// generous: it stops floods, not people.
pub const IP_APPROVAL_CALLS_PER_MINUTE: usize = 600;
/// An attempt with no success after this long has failed.
pub const FAIL_GRACE: Duration = Duration::from_secs(60);
/// Counts are forgotten after this long without an attempt.
pub const FORGET_AFTER: Duration = Duration::from_secs(24 * 3600);
/// A "failed sign-ins" notice goes out each time this many more have failed.
pub const NOTICE_EVERY: u32 = 5;

const MINUTE: Duration = Duration::from_secs(60);
/// Addresses and accounts tracked before old entries are swept out.
const SWEEP_AT: usize = 1024;

/// How long an account must wait before its next attempt, after `attempts`
/// of them: nothing up to the free ones, then 1 minute, doubling to an hour.
pub fn wait_after(attempts: u32) -> Duration {
    if attempts < FREE_ATTEMPTS {
        return Duration::ZERO;
    }
    let doublings = (attempts - FREE_ATTEMPTS).min(16);
    (FIRST_WAIT * 2u32.pow(doublings)).min(MAX_WAIT)
}

#[derive(Default)]
struct Account {
    /// When each attempt since the last success started.
    attempts: Vec<Duration>,
    /// How many of them a notice has been sent for.
    noticed: u32,
}

impl Account {
    fn last(&self) -> Option<Duration> {
        self.attempts.last().copied()
    }
}

/// An address, with an IPv6 address reduced to its /64, which is what one
/// subscriber gets.
fn key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            let mut o = v6.octets();
            o[8..].fill(0);
            IpAddr::V6(o.into())
        }
        v4 => v4,
    }
}

#[derive(Default)]
pub struct Guard {
    accounts: HashMap<i64, Account>,
    password_ips: HashMap<IpAddr, VecDeque<Duration>>,
    approval_ips: HashMap<IpAddr, VecDeque<Duration>>,
}

/// Count a call in a sliding minute. Refused calls aren't counted.
fn within_limit(
    map: &mut HashMap<IpAddr, VecDeque<Duration>>,
    ip: IpAddr,
    now: Duration,
    limit: usize,
) -> bool {
    if map.len() >= SWEEP_AT {
        map.retain(|_, v| v.back().is_some_and(|t| now.saturating_sub(*t) < MINUTE));
    }
    let seen = map.entry(key(ip)).or_default();
    while seen
        .front()
        .is_some_and(|t| now.saturating_sub(*t) >= MINUTE)
    {
        seen.pop_front();
    }
    if seen.len() >= limit {
        return false;
    }
    seen.push_back(now);
    true
}

impl Guard {
    /// A password attempt from `ip`: false if it has made too many this minute.
    pub fn password_attempt_from(&mut self, ip: IpAddr, now: Duration) -> bool {
        within_limit(&mut self.password_ips, ip, now, IP_ATTEMPTS_PER_MINUTE)
    }

    /// A call to an unauthenticated approval route from `ip`.
    pub fn approval_call_from(&mut self, ip: IpAddr, now: Duration) -> bool {
        within_limit(
            &mut self.approval_ips,
            ip,
            now,
            IP_APPROVAL_CALLS_PER_MINUTE,
        )
    }

    /// An attempt to sign in to `account` starts. False means the account is
    /// waiting out its backoff: the attempt must not be given anything to
    /// test a password against, and is not counted, so waiting is not
    /// extended by trying.
    pub fn start_attempt(&mut self, account: i64, now: Duration) -> bool {
        if self.accounts.len() >= SWEEP_AT {
            self.accounts.retain(|_, a| {
                a.last()
                    .is_some_and(|t| now.saturating_sub(t) < FORGET_AFTER)
            });
        }
        let a = self.accounts.entry(account).or_default();
        if a.last()
            .is_some_and(|t| now.saturating_sub(t) >= FORGET_AFTER)
        {
            *a = Account::default();
        }
        if let Some(last) = a.last() {
            if now < last + wait_after(a.attempts.len() as u32) {
                return false;
            }
        }
        a.attempts.push(now);
        true
    }

    /// The password was proven: nothing failed after all.
    pub fn succeeded(&mut self, account: i64) {
        self.accounts.remove(&account);
    }

    /// The password changed, so whoever was guessing the old one has nothing.
    pub fn forget(&mut self, account: i64) {
        self.accounts.remove(&account);
    }

    /// Notices that have become due: for each account, the number of failed
    /// sign-ins, whenever another [`NOTICE_EVERY`] have failed.
    pub fn settle(&mut self, now: Duration) -> Vec<(i64, u32)> {
        let mut due = Vec::new();
        for (id, a) in self.accounts.iter_mut() {
            let failed = a
                .attempts
                .iter()
                .filter(|t| now.saturating_sub(**t) >= FAIL_GRACE)
                .count() as u32;
            let level = failed / NOTICE_EVERY * NOTICE_EVERY;
            if level > a.noticed {
                a.noticed = level;
                due.push((*id, level));
            }
        }
        due.sort_unstable();
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn the_wait_starts_at_a_minute_after_five_and_doubles_to_an_hour() {
        let waits: Vec<u64> = (0..=13).map(|n| wait_after(n).as_secs()).collect();
        assert_eq!(
            waits,
            [0, 0, 0, 0, 0, 60, 120, 240, 480, 960, 1920, 3600, 3600, 3600]
        );
        assert_eq!(wait_after(u32::MAX), MAX_WAIT);
    }

    #[test]
    fn five_attempts_are_free_then_each_waits_twice_as_long() {
        let mut g = Guard::default();
        let mut t = s(0);
        for _ in 0..5 {
            assert!(g.start_attempt(1, t));
            t += s(1);
        }
        // The sixth waits a minute from the fifth.
        let fifth = t - s(1);
        assert!(!g.start_attempt(1, fifth + s(59)));
        assert!(g.start_attempt(1, fifth + s(60)));
        // The seventh waits two minutes, the eighth four.
        assert!(!g.start_attempt(1, fifth + s(60 + 119)));
        assert!(g.start_attempt(1, fifth + s(60 + 120)));
        assert!(!g.start_attempt(1, fifth + s(180 + 239)));
        assert!(g.start_attempt(1, fifth + s(180 + 240)));
        // Another account is unaffected.
        assert!(g.start_attempt(2, fifth + s(1)));
    }

    #[test]
    fn trying_during_the_wait_does_not_lengthen_it() {
        let mut g = Guard::default();
        for _ in 0..5 {
            assert!(g.start_attempt(1, s(0)));
        }
        for t in 1..60 {
            assert!(!g.start_attempt(1, s(t)));
        }
        assert!(g.start_attempt(1, s(60)));
    }

    #[test]
    fn the_wait_stops_growing_at_an_hour() {
        let mut g = Guard::default();
        let mut t = s(0);
        for _ in 0..40 {
            while !g.start_attempt(1, t) {
                t += s(10);
            }
        }
        let last = t;
        assert!(!g.start_attempt(1, last + s(3599)));
        assert!(g.start_attempt(1, last + s(3600)));
    }

    #[test]
    fn a_success_wipes_the_count_and_a_quiet_day_forgets_it() {
        let mut g = Guard::default();
        for _ in 0..5 {
            assert!(g.start_attempt(1, s(0)));
        }
        assert!(!g.start_attempt(1, s(1)));
        g.succeeded(1);
        assert!(g.start_attempt(1, s(1)));

        for _ in 0..4 {
            assert!(g.start_attempt(1, s(2)));
        }
        assert!(!g.start_attempt(1, s(3)));
        assert!(g.start_attempt(1, s(2) + FORGET_AFTER));
    }

    #[test]
    fn five_failures_make_a_notice_once_the_grace_has_passed_and_every_five_more() {
        let mut g = Guard::default();
        for i in 0..5 {
            assert!(g.start_attempt(1, s(i)));
        }
        // The fifth might still be on its way to succeeding.
        assert!(g.settle(s(4) + FAIL_GRACE - s(1)).is_empty());
        assert_eq!(g.settle(s(4) + FAIL_GRACE), [(1, 5)]);
        assert!(g.settle(s(5000)).is_empty());
        // Five more, each after its wait.
        let mut t = s(4) + FAIL_GRACE;
        for _ in 0..5 {
            while !g.start_attempt(1, t) {
                t += s(30);
            }
        }
        assert_eq!(g.settle(t + FAIL_GRACE), [(1, 10)]);
        assert!(g.settle(t + FAIL_GRACE).is_empty());
    }

    #[test]
    fn a_success_before_the_grace_means_no_notice() {
        let mut g = Guard::default();
        for _ in 0..5 {
            g.start_attempt(1, s(0));
        }
        g.succeeded(1);
        assert!(g.settle(s(1000)).is_empty());
    }

    #[test]
    fn an_address_gets_twenty_attempts_in_a_sliding_minute() {
        let mut g = Guard::default();
        let ip: IpAddr = "192.0.2.7".parse().unwrap();
        for i in 0..20 {
            assert!(g.password_attempt_from(ip, Duration::from_millis(i * 100)));
        }
        assert!(!g.password_attempt_from(ip, s(30)));
        assert!(!g.password_attempt_from(ip, Duration::from_millis(59_999)));
        assert!(g.password_attempt_from(ip, s(60)));
        // Others are not held up, and the two kinds of call count apart.
        assert!(g.password_attempt_from("192.0.2.8".parse().unwrap(), s(30)));
        assert!(g.approval_call_from(ip, s(30)));
    }

    #[test]
    fn one_ipv6_subscriber_is_one_address() {
        let mut g = Guard::default();
        for i in 0..20u16 {
            let ip: IpAddr = format!("2001:db8:1:2::{i:x}").parse().unwrap();
            assert!(g.password_attempt_from(ip, s(0)));
        }
        assert!(!g.password_attempt_from("2001:db8:1:2:ffff::1".parse().unwrap(), s(1)));
        assert!(g.password_attempt_from("2001:db8:1:3::1".parse().unwrap(), s(1)));
    }

    #[test]
    fn approval_calls_are_limited_too() {
        let mut g = Guard::default();
        let ip: IpAddr = "192.0.2.9".parse().unwrap();
        for _ in 0..IP_APPROVAL_CALLS_PER_MINUTE {
            assert!(g.approval_call_from(ip, s(0)));
        }
        assert!(!g.approval_call_from(ip, s(1)));
        assert!(g.approval_call_from(ip, s(61)));
    }

    #[test]
    fn nothing_is_kept_for_an_address_that_went_quiet() {
        let mut g = Guard::default();
        for i in 0..SWEEP_AT as u32 {
            g.password_attempt_from(IpAddr::from(i.to_be_bytes()), s(0));
        }
        assert_eq!(g.password_ips.len(), SWEEP_AT);
        g.password_attempt_from("10.9.9.9".parse().unwrap(), s(120));
        assert_eq!(g.password_ips.len(), 1);
    }
}
