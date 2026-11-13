//! Original 40b7ed / 40c376 / 40bfeb / 40c855 legacy outfit generators.
//! Catalogs must already be in original order. Both body and texture selection
//! use ranked matching here (unlike 40e5ce's first-match pajama body lookup).
use super::{Best, Pattern};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Generator {
    Adult,
    Child,
    HighFashion,
    Winter,
}

/// StringSet reads distinguish an absent entry from an empty, present string.
/// Writes affect the caller's runtime resource, never the immutable source file.
pub trait Bank {
    fn read(&mut self, entry: usize) -> Result<Option<Vec<u8>>, String>;
    fn set(&mut self, entry: usize, value: Vec<u8>) -> Result<(), String>;
}

pub fn fixed(bytes: &[u8], maximum: usize) -> &[u8] {
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(bytes.len())
        .min(maximum);
    &bytes[..end]
}

pub fn sex(bank: &mut impl Bank) -> Result<i32, String> {
    let value = bank.read(13)?.ok_or("native clothing sex entry absent")?;
    let value = fixed(&value, 15);
    Ok([
        b"male".as_slice(),
        b"female",
        b"dogmale",
        b"dogfemale",
        b"catmale",
    ]
    .iter()
    .position(|name| value.eq_ignore_ascii_case(name))
    .map_or(5, |n| n as i32))
}

pub fn child(bank: &mut impl Bank) -> Result<bool, String> {
    let value = bank.read(14)?.ok_or("native clothing age entry absent")?;
    Ok(crate::pose::layer::atoi(&String::from_utf8_lossy(fixed(&value, 15))) < 18)
}

pub fn tone(bank: &mut impl Bank) -> Result<Vec<u8>, String> {
    Ok(fixed(bank.read(15)?.as_deref().unwrap_or(b"???"), 3).to_vec())
}

pub fn body_type(bank: &mut impl Bank) -> Result<Vec<u8>, String> {
    let value = bank
        .read(2)?
        .ok_or("native clothing normal-body entry absent")?;
    let value = fixed(&value, 255);
    let end = value
        .iter()
        .position(|&b| b == b',' || b == b';')
        .unwrap_or(value.len())
        .min(63);
    let name = &value[..end];
    if name.len() < 6 {
        return Err("native clothing body-type read exceeds short normal descriptor".into());
    }
    Ok(name[6..name.len().min(9)].to_ascii_lowercase())
}

fn valid_tone(bank: &mut impl Bank) -> Result<Vec<u8>, String> {
    for supported in [b"lgt", b"med", b"drk"] {
        if tone(bank)?.eq_ignore_ascii_case(supported) {
            // Native rereads a recognized color rather than reusing the compare.
            return tone(bank);
        }
    }
    Ok(b"lgt".to_vec())
}

/// Generate all selected descriptors before publishing the ordered profile
/// writes. Native caller transaction ownership governs resource failures.
pub fn generate(
    generator: Generator,
    bank: &mut impl Bank,
    bodies: &[Vec<u8>],
    textures: &[Vec<u8>],
) -> Result<(), String> {
    let shape = body_type(bank)?;
    let adult_shape = if [b"skn".as_slice(), b"fit", b"fat"].contains(&shape.as_slice()) {
        shape.as_slice()
    } else {
        b"fit"
    };
    let entries: &[(usize, &[u8])] = match generator {
        Generator::Adult => &[(31, b"f"), (32, b"s"), (33, b"l")],
        Generator::Child => &[(32, b"s"), (33, b"l")],
        Generator::HighFashion => &[(35, b"h")],
        Generator::Winter => &[(34, b"w")],
    };
    let mut patterns = vec![];
    for &(entry, prefix) in entries {
        let mut body = Pattern::default();
        body.literal(prefix);
        if generator == Generator::HighFashion {
            body.literal(b"5");
            body.skip(2);
        } else {
            body.literal(b"100");
        }
        match sex(bank)? {
            0 => body.literal(b"m"),
            1 => body.literal(b"f"),
            _ => (),
        }
        match generator {
            Generator::Adult => {
                body.literal(b"a");
                body.literal(adult_shape);
            }
            Generator::Child => {
                body.literal(b"c");
                body.literal(&shape);
            }
            Generator::HighFashion | Generator::Winter => {
                if !child(bank)? {
                    body.literal(b"a");
                    body.literal(adult_shape);
                } else if generator == Generator::Winter {
                    body.literal(b"cchd");
                }
            }
        }
        let mut texture = body.clone();
        texture.literal(&valid_tone(bank)?);
        patterns.push((entry, body, texture, Best::default(), Best::default()));
    }
    // Original loop ordering: visit each catalog candidate, then all patterns.
    for candidate in bodies {
        for (_, pattern, _, best, _) in &mut patterns {
            best.consider(pattern, fixed(candidate, 63));
        }
    }
    for candidate in textures {
        for (_, _, pattern, _, best) in &mut patterns {
            best.consider(pattern, fixed(candidate, 63));
        }
    }
    for (entry, _, _, body, texture) in patterns {
        let mut descriptor = body.name;
        descriptor.extend_from_slice(b",BODY=");
        descriptor.extend_from_slice(&texture.name);
        descriptor.truncate(63);
        bank.set(entry, descriptor)?;
    }
    Ok(())
}
