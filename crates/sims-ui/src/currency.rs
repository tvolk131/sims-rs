//! Money as the interface writes it: the original's locale currency
//! formatter (`cTSLocale`, 0x6007df and the routines its vtable names).
//!
//! It is a number formatter, not a string-table lookup: the amount's
//! digits, grouped by threes with the language's separator, after the
//! currency symbol. A negative amount is written `-$5` in US English (the
//! minus before the symbol). Mined from the old prototype
//! (`legacy/crates/sims-sim/src/person_notification/currency.rs`), which
//! read it from the exe.
//!
//! The panel's font draws `$` as the Simoleon glyph, so `$` is what is
//! written.

/// The game's language code for US English, the one the original's
/// locale is made with (+8 = 1).
pub const US_ENGLISH: u8 = 1;

/// The original's locale (`cTSLocale`): the words the currency formatter
/// reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    /// Word +8: the game's language code (1 US English).
    pub language: u8,
    /// Byte +0xc: write the language's own currency symbol. The game
    /// leaves it clear, so every language writes `$` (the Simoleon).
    pub real_currency: bool,
}

impl Default for Locale {
    fn default() -> Self {
        Self::US
    }
}

impl Locale {
    /// The locale the original makes (language 1, Simoleons).
    pub const US: Self = Self {
        language: US_ENGLISH,
        real_currency: false,
    };

    /// The thousands separator of the language, or None for a language
    /// code outside the table (the original then writes a placeholder
    /// word, which is not reproduced).
    fn separator(self) -> Option<&'static str> {
        Some(match self.language {
            1 | 2 | 12 | 15 | 17..=20 => ",",
            3..=9 | 13 | 14 => ".",
            10 | 11 | 16 => "",
            _ => return None,
        })
    }

    /// The currency symbol, as Windows-1252 bytes read as Latin-1
    /// characters (the bytes used here are the same in both).
    fn symbol(self) -> Option<&'static str> {
        if !self.real_currency {
            return Some("$");
        }
        Some(match self.language {
            1 => "$",
            2 => "\u{a3}",
            3 | 7 => "F",
            4 => "DM",
            5 => "L.",
            6 => "Pts",
            8..=10 => "kr",
            11 => "mk",
            12 => "NIS",
            13 => "py\u{e1}",
            14 => "Esc.",
            15 => "\u{a5}",
            16 => "Z\u{b3}",
            17 | 18 => "RMB\u{a5}",
            19 => "\u{df}",
            _ => return None,
        })
    }

    /// `amount` as the original writes it in this locale, or None for a
    /// language outside the original's table.
    ///
    /// - The digits are grouped by threes from the right.
    /// - A positive amount or zero is the symbol and the digits: `$2,542`.
    /// - A negative one is `-` and the symbol and the digits in languages
    ///   1 to 19 (`-$5`), except languages 8 and 12 (` -` and the digits,
    ///   no symbol) and 10 (`-` and the digits). The original builds those
    ///   three by writing the symbol and then overwriting it with the
    ///   finished number, so the symbol is lost. Any other language
    ///   writes nothing for a negative amount.
    /// - The amount is negated in 64 bits, as the original does, so even
    ///   `i32::MIN` keeps all its digits.
    pub fn format(self, amount: i64) -> Option<String> {
        if amount < 0 && !(1..=19).contains(&self.language) {
            return Some(String::new());
        }
        let separator = self.separator()?;
        let symbol = self.symbol()?;
        let digits = amount.unsigned_abs().to_string();
        let mut grouped = String::with_capacity(digits.len() + digits.len() / 3 * 2);
        for (i, ch) in digits.chars().enumerate() {
            if i != 0 && (digits.len() - i).is_multiple_of(3) {
                grouped.push_str(separator);
            }
            grouped.push(ch);
        }
        let lead = if amount < 0 {
            match self.language {
                8 | 12 => " -".to_owned(),
                10 => "-".to_owned(),
                _ => format!("-{symbol}"),
            }
        } else {
            symbol.to_owned()
        };
        Some(lead + &grouped)
    }
}

/// `amount` in Simoleons as the game writes money (the original's locale,
/// US English): `$2,542`, `-$5`.
pub fn simoleons(amount: i64) -> String {
    Locale::US.format(amount).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_english_groups_by_threes_and_puts_the_minus_before_the_symbol() {
        assert_eq!(simoleons(0), "$0");
        assert_eq!(simoleons(5), "$5");
        assert_eq!(simoleons(999), "$999");
        assert_eq!(simoleons(1_000), "$1,000");
        assert_eq!(simoleons(2_542), "$2,542");
        assert_eq!(simoleons(1_234_567), "$1,234,567");
        assert_eq!(simoleons(-5), "-$5");
        assert_eq!(simoleons(-1_000), "-$1,000");
        assert_eq!(
            simoleons(i64::from(i32::MIN)),
            "-$2,147,483,648",
            "negated in 64 bits"
        );
        assert_eq!(simoleons(i64::MIN), "-$9,223,372,036,854,775,808");
    }

    #[test]
    fn the_game_writes_simoleons_in_every_language() {
        for language in 1..=19 {
            let locale = Locale {
                language,
                real_currency: false,
            };
            let text = locale.format(1_000).unwrap();
            assert!(text.starts_with('$'), "language {language}: {text}");
        }
        let german = Locale {
            language: 4,
            real_currency: false,
        };
        assert_eq!(german.format(1_234_567).as_deref(), Some("$1.234.567"));
        let finnish = Locale {
            language: 11,
            real_currency: false,
        };
        assert_eq!(finnish.format(1_234_567).as_deref(), Some("$1234567"));
    }

    #[test]
    fn real_currency_negatives_follow_the_native_branches() {
        let real = |language| Locale {
            language,
            real_currency: true,
        };
        assert_eq!(real(2).format(-1_500).as_deref(), Some("-\u{a3}1,500"));
        assert_eq!(real(4).format(-1_500).as_deref(), Some("-DM1.500"));
        // The symbol is written and then overwritten.
        assert_eq!(real(8).format(-1_500).as_deref(), Some(" -1.500"));
        assert_eq!(real(12).format(-1_500).as_deref(), Some(" -1,500"));
        assert_eq!(real(10).format(-1_500).as_deref(), Some("-1500"));
        assert_eq!(real(20).format(-1_500).as_deref(), Some(""));
        assert_eq!(real(20).format(1_500), None, "no symbol for language 20");
        assert_eq!(real(0).format(1), None);
        assert_eq!(real(0).format(-1).as_deref(), Some(""));
        assert_eq!(
            Locale {
                language: 21,
                real_currency: false
            }
            .format(1),
            None
        );
    }
}
