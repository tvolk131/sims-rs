//! Original cTSLocale currency formatter (0x6007df and its native vtable).
//! Returns original encoded bytes. This is a number formatter, not a STR# lookup.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locale {
    /// Native locale word at +8; the constructor installs 1 (US English).
    pub language: u32,
    /// Native byte at +c; false keeps the dollar symbol in every language.
    pub real_currency: bool,
}

impl Locale {
    pub fn format(self, amount: i64) -> Vec<u8> {
        // The original negates in 64 bits before signed printf. Keep its
        // double-minus result at i64::MIN instead of silently widening it.
        let number = if amount < 0 {
            amount.wrapping_neg()
        } else {
            amount
        }
        .to_string();
        let separator: &[u8] = match self.language {
            1 | 2 | 12 | 15 | 17..=20 => b",",
            3..=9 | 13 | 14 => b".",
            10 | 11 | 16 => b"",
            _ => b"untranslated",
        };
        let symbol: &[u8] = if !self.real_currency {
            b"$"
        } else {
            match self.language {
                1 => b"$",
                2 => b"\xa3",
                3 | 7 => b"F",
                4 => b"DM",
                5 => b"L.",
                6 => b"Pts",
                8..=10 => b"kr",
                11 => b"mk",
                12 => b"NIS",
                13 => b"py\xe1",
                14 => b"Esc.",
                15 => b"\xa5",
                16 => b"Z\xb3",
                17 | 18 => b"RMB\xa5",
                19 => b"\xdf",
                _ => b"untranslated",
            }
        };
        let mut grouped = Vec::new();
        for (i, byte) in number.bytes().enumerate() {
            if i != 0 && (number.len() - i) % 3 == 0 {
                grouped.extend_from_slice(separator);
            }
            grouped.push(byte);
        }
        let mut result = Vec::new();
        if amount < 0 {
            match self.language {
                // These native branches write the symbol to the destination
                // first, then overwrite it with the completed temporary.
                8 | 12 => result.extend_from_slice(b" -"),
                10 => result.push(b'-'),
                1..=19 => {
                    result.push(b'-');
                    result.extend_from_slice(symbol);
                }
                _ => return result,
            }
        } else {
            result.extend_from_slice(symbol);
        }
        result.extend(grouped);
        result
    }
}
