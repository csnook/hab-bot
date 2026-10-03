//! Password strength, and a passphrase to suggest.
//!
//! The server holds an encrypted copy of the keys, so whoever runs it can try
//! to guess the password offline. That is why the bar is high: a password has
//! to be estimated at more than ten billion guesses.

use bip39::Language;
use rand::Rng;
use serde::Serialize;
use zxcvbn::{zxcvbn, Score};

/// zxcvbn's top score: at least 10^10 guesses.
const REQUIRED: Score = Score::Four;
const MIN_LENGTH: usize = 10;
const PASSPHRASE_WORDS: usize = 5;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PasswordCheck {
    /// 0 to 4, for the meter.
    pub score: u8,
    /// Whether Continue may be enabled.
    pub ok: bool,
    pub warning: Option<String>,
    pub suggestions: Vec<String>,
}

/// Judge `password`. `context` is anything about the user that a guesser
/// would try first, such as the username and display name.
pub fn check_password(password: &str, context: &[&str]) -> PasswordCheck {
    let entropy = zxcvbn(password, context);
    let score = entropy.score();
    let (warning, suggestions) = match entropy.feedback() {
        Some(f) => (
            f.warning().map(|w| w.to_string()),
            f.suggestions().iter().map(|s| s.to_string()).collect(),
        ),
        None => (None, Vec::new()),
    };
    let long_enough = password.chars().count() >= MIN_LENGTH;
    let mut warning = warning;
    if !long_enough && warning.is_none() {
        warning = Some(format!("Use at least {MIN_LENGTH} characters."));
    }
    PasswordCheck {
        score: score.into(),
        ok: score >= REQUIRED && long_enough,
        warning,
        suggestions,
    }
}

/// Five random words (about 55 bits, from a 2048-word list), which a person
/// can remember and which pass the check.
pub fn suggest_passphrase() -> String {
    let words = Language::English.word_list();
    let mut rng = rand::rngs::OsRng;
    (0..PASSPHRASE_WORDS)
        .map(|_| words[rng.gen_range(0..words.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guessable_passwords_are_refused() {
        for p in [
            "",
            "password",
            "hunter2",
            "Password123!",
            "qwertyuiop",
            "chris1990",
        ] {
            let c = check_password(p, &["chris"]);
            assert!(!c.ok, "{p} should be refused");
        }
        assert!(check_password("password", &[]).warning.is_some());
    }

    #[test]
    fn a_name_the_user_gave_counts_against_the_password() {
        assert!(!check_password("snookchris", &["snookchris"]).ok);
    }

    #[test]
    fn short_passwords_are_refused_even_if_random() {
        let c = check_password("x9$Qv7!m", &[]);
        assert!(!c.ok);
        assert!(c.warning.is_some());
    }

    #[test]
    fn long_random_ones_pass() {
        assert!(check_password("t7#Vq!9mZk2$Lp4x", &[]).ok);
    }

    #[test]
    fn the_suggested_passphrase_always_passes() {
        for _ in 0..50 {
            let p = suggest_passphrase();
            assert_eq!(p.split(' ').count(), 5);
            assert!(check_password(&p, &["chris", "Chris"]).ok, "{p}");
        }
    }

    #[test]
    fn suggestions_differ_each_time() {
        assert_ne!(suggest_passphrase(), suggest_passphrase());
    }
}
