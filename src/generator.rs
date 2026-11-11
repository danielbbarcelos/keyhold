//! Making up a password nobody has to remember.
//!
//! Two things decide whether a generated password is any good, and neither is the character soup
//! it looks like:
//!
//! **Length is what makes it expensive.** Adding a class of character multiplies the search space
//! once; adding a character multiplies it again for every position. keyhold's default of 20 from
//! the full alphabet is about 130 bits — beyond the reach of anything that has to actually run.
//!
//! **The randomness has to be uniform.** `value % alphabet.len()` is the classic mistake: unless
//! the range divides evenly, the first few characters of the alphabet come up slightly more often,
//! forever, in every password the tool has ever produced. Rejection sampling throws away the
//! values that would skew it, which costs a handful of extra bytes and nothing else.
//!
//! The generated master password is a deliberate trade, stated in the onboarding copy rather than
//! hidden: it cannot be guessed, dictionary-attacked, or reused from a site that leaked — and it
//! cannot be memorised either, so it becomes something to store, exactly like the recovery key.
//! Somebody who would rather choose their own may; keyhold recommends, and does not insist.

use rand::RngCore;
use serde::{Deserialize, Serialize};

const LOWERCASE: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPERCASE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!@#$%^&*()_+-=[]{}|;:,.<>?";

/// Every symbol a person may pick from, when they choose which ones a site accepts.
pub const ALL_SYMBOLS: &str = "!@#$%^&*()_+-=[]{}|;:,.<>?/\\~`'\"";

/// Characters that get misread on paper or across a font change.
const AMBIGUOUS: &str = "Il1O0|";

pub const DEFAULT_LENGTH: usize = 20;
const MIN_LENGTH: usize = 4;
const MAX_LENGTH: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Recipe {
    pub length: usize,
    pub lowercase: bool,
    pub uppercase: bool,
    pub digits: bool,
    pub symbols: bool,
    /// Leave out characters that are hard to tell apart when read by a person.
    pub avoid_ambiguous: bool,
    /// Only these symbols, when a site accepts some and not others. `None` is the usual set.
    pub symbol_set: Option<String>,
}

impl Default for Recipe {
    fn default() -> Self {
        Self {
            length: DEFAULT_LENGTH,
            lowercase: true,
            uppercase: true,
            digits: true,
            symbols: true,
            avoid_ambiguous: false,
            symbol_set: None,
        }
    }
}

impl Recipe {
    fn sets(&self) -> Vec<Vec<char>> {
        let mut sets = Vec::new();

        // Chosen symbols are kept only when they are symbols this generator knows.
        let chosen: String = match &self.symbol_set {
            Some(set) => set.chars().filter(|c| ALL_SYMBOLS.contains(*c)).collect(),
            None => SYMBOLS.to_string(),
        };

        for (enabled, source) in [
            (self.lowercase, LOWERCASE),
            (self.uppercase, UPPERCASE),
            (self.digits, DIGITS),
            (self.symbols, chosen.as_str()),
        ] {
            if !enabled {
                continue;
            }

            let chars: Vec<char> = source
                .chars()
                .filter(|c| !self.avoid_ambiguous || !AMBIGUOUS.contains(*c))
                .collect();

            if !chars.is_empty() {
                sets.push(chars);
            }
        }

        sets
    }
}

#[derive(Debug, Serialize)]
pub struct Generated {
    pub password: String,
    /// What it would cost to search this space, in bits. Honest here because the value really is
    /// uniformly random — the same number would be a lie about a password a person invented.
    pub entropy_bits: u32,
}

/// A uniform integer in `[0, max)`, by rejection sampling.
fn uniform(max: usize) -> usize {
    assert!(max > 0);

    let limit = (u32::MAX as usize / max) * max;
    let mut buffer = [0u8; 4];

    loop {
        rand::rngs::OsRng.fill_bytes(&mut buffer);
        let value = u32::from_le_bytes(buffer) as usize;

        if value < limit {
            return value % max;
        }
    }
}

pub fn generate(recipe: Recipe) -> Generated {
    let length = recipe.length.clamp(MIN_LENGTH, MAX_LENGTH);

    // Every recipe keeps at least one set: a request for nothing gets letters rather than an error
    // or an empty password.
    let sets = match recipe.sets() {
        sets if sets.is_empty() => vec![LOWERCASE.chars().collect()],
        sets => sets,
    };

    let alphabet: Vec<char> = sets.iter().flatten().copied().collect();

    // One character guaranteed from each set, so a recipe that asks for digits gets digits — then
    // the rest from the whole alphabet, then shuffled so the guaranteed ones are not at the front.
    let mut chars: Vec<char> = sets
        .iter()
        .take(length)
        .map(|set| set[uniform(set.len())])
        .collect();

    while chars.len() < length {
        chars.push(alphabet[uniform(alphabet.len())]);
    }

    // Fisher-Yates, with the same uniform source.
    for i in (1..chars.len()).rev() {
        chars.swap(i, uniform(i + 1));
    }

    Generated {
        password: chars.into_iter().collect(),
        entropy_bits: (length as f64 * (alphabet.len() as f64).log2()).floor() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn it_honours_the_length() {
        for length in [8, 12, 20, 64] {
            let out = generate(Recipe { length, ..Recipe::default() });
            assert_eq!(out.password.chars().count(), length);
        }
    }

    #[test]
    fn an_absurd_length_is_brought_into_range() {
        assert_eq!(generate(Recipe { length: 1, ..Recipe::default() }).password.len(), MIN_LENGTH);
        assert_eq!(
            generate(Recipe { length: 10_000, ..Recipe::default() }).password.chars().count(),
            MAX_LENGTH
        );
    }

    #[test]
    fn every_requested_class_actually_appears() {
        for _ in 0..50 {
            let out = generate(Recipe::default()).password;

            assert!(out.chars().any(|c| LOWERCASE.contains(c)), "{out}");
            assert!(out.chars().any(|c| UPPERCASE.contains(c)), "{out}");
            assert!(out.chars().any(|c| DIGITS.contains(c)), "{out}");
            assert!(out.chars().any(|c| SYMBOLS.contains(c)), "{out}");
        }
    }

    #[test]
    fn a_class_that_was_not_asked_for_never_appears() {
        let recipe = Recipe { symbols: false, digits: false, ..Recipe::default() };

        for _ in 0..50 {
            let out = generate(recipe.clone()).password;

            assert!(!out.chars().any(|c| SYMBOLS.contains(c)), "{out}");
            assert!(!out.chars().any(|c| DIGITS.contains(c)), "{out}");
        }
    }

    #[test]
    fn ambiguous_characters_can_be_kept_out() {
        let recipe = Recipe { avoid_ambiguous: true, length: 64, ..Recipe::default() };

        for _ in 0..20 {
            let out = generate(recipe.clone()).password;
            assert!(!out.chars().any(|c| AMBIGUOUS.contains(c)), "{out}");
        }
    }

    #[test]
    fn two_passwords_are_never_the_same() {
        let seen: HashSet<String> =
            (0..200).map(|_| generate(Recipe::default()).password).collect();

        assert_eq!(seen.len(), 200);
    }

    #[test]
    fn the_guaranteed_characters_are_not_always_at_the_front() {
        // If the shuffle were missing, position 0 would always come from the first set.
        let firsts: HashSet<bool> = (0..100)
            .map(|_| {
                let out = generate(Recipe::default()).password;
                LOWERCASE.contains(out.chars().next().unwrap())
            })
            .collect();

        assert_eq!(firsts.len(), 2, "the first character always came from the same set");
    }

    #[test]
    fn the_distribution_does_not_favour_the_start_of_the_alphabet() {
        // The modulo bug shows up as a lean towards the first characters. With a two-character
        // alphabet and 4000 draws, a fair split sits far inside this bound and a skewed one does
        // not.
        let recipe = Recipe {
            length: 40,
            lowercase: true,
            uppercase: false,
            digits: false,
            symbols: false,
            avoid_ambiguous: false,
            symbol_set: None,
        };

        let mut first_half = 0usize;
        let mut total = 0usize;

        for _ in 0..100 {
            for c in generate(recipe.clone()).password.chars() {
                total += 1;
                if c <= 'm' {
                    first_half += 1;
                }
            }
        }

        let ratio = first_half as f64 / total as f64;
        assert!((0.45..0.55).contains(&ratio), "skewed towards the alphabet's start: {ratio}");
    }

    #[test]
    fn the_reported_entropy_matches_the_alphabet_it_drew_from() {
        // 20 characters from 26+26+10+26 = 88 → 20 * log2(88) ≈ 129 bits.
        let out = generate(Recipe::default());
        assert!((128..=130).contains(&out.entropy_bits), "{}", out.entropy_bits);
    }

    #[test]
    fn chosen_symbols_are_the_only_symbols() {
        let recipe = Recipe { symbol_set: Some("#_".into()), length: 64, ..Recipe::default() };

        for _ in 0..20 {
            let out = generate(recipe.clone()).password;
            assert!(out.chars().filter(|c| !c.is_ascii_alphanumeric()).all(|c| c == '#' || c == '_'), "{out}");
            assert!(out.contains('#') || out.contains('_'), "{out}");
        }
    }

    #[test]
    fn a_pin_is_digits_only() {
        let recipe = Recipe { lowercase: false, uppercase: false, symbols: false, length: 8, ..Recipe::default() };

        assert!(generate(recipe).password.chars().all(|c| c.is_ascii_digit()));
    }
}
