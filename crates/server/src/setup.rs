//! The one-time setup code that lets whoever holds it create the first account.
//!
//! It is random and long, and stops working 24 hours after the server starts
//! or once the first account exists. It lives only in memory, so a restart
//! makes a new one.

use rand::Rng;
use std::time::{Duration, Instant};

pub const LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
/// Crockford base32: no I, L, O or U to misread.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// 32 characters of 5 bits each: 160 bits.
const LENGTH: usize = 32;

pub struct SetupCode {
    code: String,
    created: Instant,
}

impl SetupCode {
    pub fn generate(now: Instant) -> SetupCode {
        let mut rng = rand::rng();
        let code = (0..LENGTH)
            .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
            .collect();
        SetupCode { code, created: now }
    }

    /// The code as shown in the console, in groups of four.
    pub fn display(&self) -> String {
        self.code
            .as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).expect("ascii"))
            .collect::<Vec<_>>()
            .join("-")
    }

    pub fn expires_at(&self) -> Instant {
        self.created + LIFETIME
    }

    /// Whether `candidate` is this code and it still works.
    pub fn accepts(&self, candidate: &str, now: Instant, account_exists: bool) -> bool {
        if account_exists || now >= self.expires_at() {
            return false;
        }
        let normalized: Vec<u8> = candidate
            .bytes()
            .filter(|b| !matches!(b, b'-' | b' ' | b'\t' | b'\n' | b'\r'))
            .map(|b| b.to_ascii_uppercase())
            .collect();
        constant_time_eq(&normalized, self.code.as_bytes())
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_long_and_random() {
        let now = Instant::now();
        let a = SetupCode::generate(now);
        let b = SetupCode::generate(now);
        assert_eq!(a.display().replace('-', "").len(), 32);
        assert_ne!(a.display(), b.display());
        assert!(a
            .display()
            .bytes()
            .all(|c| c == b'-' || ALPHABET.contains(&c)));
    }

    #[test]
    fn works_as_shown_or_loosely_typed() {
        let now = Instant::now();
        let c = SetupCode::generate(now);
        assert!(c.accepts(&c.display(), now, false));
        assert!(c.accepts(&c.display().to_lowercase().replace('-', " "), now, false));
        assert!(!c.accepts("nope", now, false));
        assert!(!c.accepts("", now, false));
    }

    #[test]
    fn stops_after_24_hours() {
        let now = Instant::now();
        let c = SetupCode::generate(now);
        let almost = now + LIFETIME - Duration::from_secs(1);
        assert!(c.accepts(&c.display(), almost, false));
        assert!(!c.accepts(&c.display(), now + LIFETIME, false));
    }

    #[test]
    fn stops_once_an_account_exists() {
        let now = Instant::now();
        let c = SetupCode::generate(now);
        assert!(!c.accepts(&c.display(), now, true));
    }
}
