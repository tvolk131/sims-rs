//! The import refresh's body and head lookup and outfit selectors.
use super::unsupported;
use crate::import_wardrobe::atoi;
use crate::{ImportWardrobe, Result};

pub(super) fn refresh(rows: &mut Vec<Vec<u8>>, wardrobe: &ImportWardrobe) -> Result<()> {
    rows.resize_with(rows.len().max(35), Vec::new); // 0x587b4b
    let body = Descriptor::parse(&rows[1]);
    let sex = fixed(&rows[12], 15).to_ascii_lowercase();
    let pet = match sex.as_slice() {
        b"catmale" | b"catfemale" => Some(b"cat"),
        b"dogmale" | b"dogfemale" => Some(b"dog"),
        b"female" | b"male" => None,
        _ => return Err(unsupported("0x40ccac", "unknown imported sex")),
    };
    if let Some(species) = pet {
        if !matches_pet(wardrobe, body, species) {
            return Err(rng_fallback("pet body"));
        }
        // 0x40a5d5..0x40a614 set all six rows regardless of their old text.
        for row in [3, 31, 32, 33, 34, 35] {
            rows[row - 1] = b"NONE".to_vec();
        }
        return Ok(());
    }

    let tone = fixed(&rows[14], 3).to_vec();
    let person = Person {
        gender: sex[0],
        child: atoi(fixed(&rows[13], 15)) < 18,
        tone: &tone,
    };
    if !matches_human(wardrobe, body, person, b'b', 9) {
        return Err(rng_fallback("body"));
    }
    if !matches_human(wardrobe, Descriptor::parse(&rows[2]), person, b'c', 6) {
        return Err(rng_fallback("head"));
    }
    let shape = body.suit[6..9].to_ascii_lowercase();
    check_existing_outfits(rows, wardrobe, person, &shape)?;
    let suffix = body.texture.get(12..).unwrap_or_default().to_vec();

    // 0x409315 fills a missing high-fashion row for adults. 0x409432 and
    // 0x40a69e write the child marker.
    if person.child {
        rows[34] = b"ADDED".to_vec();
    } else if rows[34].is_empty() {
        rows[34] = high_fashion(wardrobe, person, &shape);
    } else {
        return Err(unsupported(
            "0x40e18f",
            "preexisting high-fashion outfit validation",
        ));
    }
    rebuild_nude_and_accessories(rows, wardrobe, person, &shape, &suffix);
    Ok(())
}

fn rng_fallback(part: &str) -> crate::Error {
    unsupported(
        "0x40ea58",
        format!("random appearance fallback for imported {part}"),
    )
}

#[derive(Clone, Copy)]
struct Person<'a> {
    gender: u8,
    child: bool,
    tone: &'a [u8],
}

#[derive(Clone, Copy)]
struct Descriptor<'a> {
    suit: &'a [u8],
    texture: &'a [u8],
}

impl<'a> Descriptor<'a> {
    // 0x472ec4 and 0x5874ca compare the first suit and texture only.
    fn parse(text: &'a [u8]) -> Self {
        let first = fixed(text, 255)
            .split(|&byte| byte == b';')
            .next()
            .unwrap_or_default();
        let (suit, binding) = split_once(first, b',').unwrap_or((first, &[]));
        let texture = split_once(binding, b'=').map_or(binding, |(_, texture)| texture);
        Self {
            suit: fixed(suit, 63),
            texture: fixed(texture, 63),
        }
    }

    fn exists(self, wardrobe: &ImportWardrobe) -> bool {
        wardrobe
            .suits
            .iter()
            .any(|suit| suit.eq_ignore_ascii_case(self.suit))
            && wardrobe
                .textures
                .iter()
                .any(|texture| texture.eq_ignore_ascii_case(self.texture))
    }
}

fn split_once(text: &[u8], delimiter: u8) -> Option<(&[u8], &[u8])> {
    let position = text.iter().position(|&byte| byte == delimiter)?;
    Some((&text[..position], &text[position + 1..]))
}

fn fixed(text: &[u8], limit: usize) -> &[u8] {
    &text[..text
        .iter()
        .take(limit)
        .position(|&byte| byte == 0)
        .unwrap_or(text.len().min(limit))]
}

// 0x40d0a4 / 0x40d408 compare the parsed pair to a list entry whose texture
// begins with the suit's matched prefix followed by the person's skin tone.
fn matches_human(
    wardrobe: &ImportWardrobe,
    pair: Descriptor<'_>,
    person: Person<'_>,
    kind: u8,
    prefix_len: usize,
) -> bool {
    let Some(prefix) = pair.suit.get(..prefix_len) else {
        return false;
    };
    let age = if person.child { b'c' } else { b'a' };
    prefix[0].eq_ignore_ascii_case(&kind)
        && (kind != b'c' || !prefix[1].eq_ignore_ascii_case(&b't'))
        && [person.gender, b'u'].contains(&prefix[4].to_ascii_lowercase())
        && prefix[5].eq_ignore_ascii_case(&age)
        && starts_with(pair.texture, prefix)
        && starts_with(&pair.texture[prefix_len..], person.tone)
        && pair.exists(wardrobe)
}

fn matches_pet(wardrobe: &ImportWardrobe, pair: Descriptor<'_>, species: &[u8]) -> bool {
    pair.suit.get(..7).is_some_and(|prefix| {
        prefix[0].eq_ignore_ascii_case(&b'b')
            && prefix[4..].eq_ignore_ascii_case(species)
            && starts_with(pair.texture, prefix)
            && pair.exists(wardrobe)
    })
}

// 0x409315's missing legacy outfits and 0x40a588's invalid legacy outfits
// enter separate generators. The import path here retains exact pairs.
fn check_existing_outfits(
    rows: &[Vec<u8>],
    wardrobe: &ImportWardrobe,
    person: Person<'_>,
    shape: &[u8],
) -> Result<()> {
    for (row, kind, address) in [
        (31, b'f', "0x40d52f"),
        (32, b's', "0x40d847"),
        (33, b'l', "0x40db5f"),
        (34, b'w', "0x40de77"),
    ] {
        let value = &rows[row - 1];
        if person.child && row == 31 {
            if value.is_empty() {
                return Err(unsupported("0x40c376", "child legacy outfit generation"));
            }
            continue;
        }
        let pair = Descriptor::parse(value);
        if !matches_human(wardrobe, pair, person, kind, 9)
            || (matches!(shape, b"fit" | b"skn" | b"fat" | b"chd")
                && !pair.suit[6..9].eq_ignore_ascii_case(shape))
        {
            return Err(unsupported(
                address,
                format!("legacy outfit replacement for row {row}"),
            ));
        }
    }
    Ok(())
}

fn starts_with(name: &[u8], prefix: &[u8]) -> bool {
    name.get(..prefix.len())
        .is_some_and(|part| part.eq_ignore_ascii_case(prefix))
}

/// 0x472a25 compares the unrounded quotient against the previous f32 score.
/// A longer match also wins within the original epsilon at 0x667878.
#[derive(Default)]
struct Best<'a> {
    name: &'a [u8],
    score: f32,
    matched: usize,
}

impl<'a> Best<'a> {
    fn consider(&mut self, name: &'a [u8], matched: usize) {
        if matched == 0 {
            return;
        }
        let fraction = matched as f64 / name.len() as f64;
        let old = f64::from(self.score);
        if fraction > old
            || ((fraction - old).abs() <= f64::from(0.0001_f32) && matched > self.matched)
        {
            self.name = name;
            self.score = fraction as f32;
            self.matched = matched;
        }
    }
}

fn descriptor(suit: &[u8], binding: &[u8], texture: &[u8]) -> Vec<u8> {
    let mut text = [suit, b",", binding, b"=", texture].concat();
    text.truncate(63);
    text
}

fn high_fashion(wardrobe: &ImportWardrobe, person: Person<'_>, shape: &[u8]) -> Vec<u8> {
    let shape = if matches!(shape, b"fit" | b"skn" | b"fat") {
        shape
    } else {
        b"fit"
    };
    let tone = person.tone.to_ascii_lowercase();
    let tone = if matches!(tone.as_slice(), b"lgt" | b"med" | b"drk") {
        tone.as_slice()
    } else {
        b"lgt"
    };
    let body_prefix = [b"ma".as_slice(), shape].concat();
    let mut body_prefix = body_prefix;
    body_prefix[0] = person.gender;
    let matched = |name: &[u8], tail: &[u8]| {
        if starts_with(name, b"h5") && name.get(4..).is_some_and(|part| starts_with(part, tail)) {
            4 + tail.len()
        } else {
            0
        }
    };
    let mut suit = Best::default();
    for name in &wardrobe.suits {
        suit.consider(name, matched(name, &body_prefix));
    }
    let texture_prefix = [body_prefix.as_slice(), tone].concat();
    let mut texture = Best::default();
    for name in &wardrobe.textures {
        texture.consider(name, matched(name, &texture_prefix));
    }
    descriptor(suit.name, b"BODY", texture.name)
}

fn nude_prefix(name: &[u8], kind: u8, gender: u8, shape: &[u8]) -> usize {
    if name.len() >= 2
        && name[0].eq_ignore_ascii_case(&kind)
        && [b'u', gender].contains(&name[1].to_ascii_lowercase())
        && starts_with(&name[2..], shape)
    {
        2 + shape.len()
    } else {
        0
    }
}

fn nude_and_formal_suits<'a>(
    suits: &'a [Vec<u8>],
    gender: u8,
    shape: &[u8],
) -> (&'a [u8], &'a [u8]) {
    let mut nude = Best::default();
    let mut formal = Best::default();
    for name in suits {
        nude.consider(name, nude_prefix(name, b'n', gender, shape));
        // 0x40b304 skips the formal selector once the nude selector has
        // selected a name, including on the iteration that first finds it.
        if nude.name.is_empty() {
            formal.consider(name, nude_prefix(name, b'f', gender, shape));
        }
    }
    (nude.name, formal.name)
}

fn optional_prefix(text: &[u8], prefix: &[u8]) -> usize {
    if starts_with(text, prefix) {
        prefix.len()
    } else {
        0
    }
}

fn rebuild_nude_and_accessories(
    rows: &mut [Vec<u8>],
    wardrobe: &ImportWardrobe,
    person: Person<'_>,
    shape: &[u8],
    suffix: &[u8],
) {
    let (nude, formal) = nude_and_formal_suits(&wardrobe.suits, person.gender, shape);
    let mut body_textures = [Best::default(), Best::default(), Best::default()];
    for name in &wardrobe.textures {
        for (kind, best) in [b'n', b'u', b'f'].into_iter().zip(&mut body_textures) {
            let prefix = nude_prefix(name, kind, person.gender, shape);
            if prefix != 0 && starts_with(&name[prefix..], person.tone) {
                let end = prefix + person.tone.len();
                best.consider(name, end + optional_prefix(&name[end..], suffix));
            }
        }
    }
    let [nude_texture, underwear, formal_texture] = body_textures.map(|best| best.name);
    rows[15] = descriptor(nude, b"BODY", nude_texture);
    let underwear = if person.child && underwear.is_empty() {
        nude_texture
    } else {
        underwear
    };
    rows[16] = descriptor(nude, b"BODY", underwear);
    rows[27] = descriptor(formal, b"BODY", formal_texture);

    for (row, kind, side, end) in [
        (29, b'g', b'l', b'o'),
        (30, b'g', b'r', b'o'),
        (18, b'h', b'l', b'o'),
        (19, b'h', b'r', b'o'),
        (22, b'h', b'l', b'c'),
        (23, b'h', b'r', b'c'),
        (20, b'h', b'l', b'p'),
        (21, b'h', b'r', b'p'),
    ] {
        let mut best = Best::default();
        for name in &wardrobe.textures {
            if let [first, gender, hand, last, rest @ ..] = name.as_slice()
                && first.eq_ignore_ascii_case(&kind)
                && [b'u', person.gender].contains(&gender.to_ascii_lowercase())
                && [b'a', side].contains(&hand.to_ascii_lowercase())
                && last.eq_ignore_ascii_case(&end)
            {
                let without_tone = optional_prefix(rest, suffix);
                let with_tone = optional_prefix(rest, person.tone);
                let with_tone = with_tone + optional_prefix(&rest[with_tone..], suffix);
                best.consider(name, 4 + without_tone.max(with_tone));
            }
        }
        let gender = if person.child { b'u' } else { person.gender };
        rows[row - 1] = if best.name.is_empty() {
            Vec::new()
        } else {
            descriptor(
                &[
                    b'H',
                    gender,
                    side.to_ascii_uppercase(),
                    end.to_ascii_uppercase(),
                ],
                b"HAND",
                best.name,
            )
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nude_match_stops_the_formal_selector() {
        let suits = [
            b"fumore".as_slice(),
            b"fufit_long",
            b"nmfit_long",
            b"fufit",
            b"nufit",
        ]
        .map(<[u8]>::to_vec);
        let (nude, formal) = nude_and_formal_suits(&suits, b'm', b"fit");
        assert_eq!(nude, b"nufit");
        assert_eq!(formal, b"fufit_long");
        let (nude, formal) = nude_and_formal_suits(&suits[2..], b'm', b"fit");
        assert_eq!(nude, b"nufit");
        assert_eq!(formal, b"");
    }
}
