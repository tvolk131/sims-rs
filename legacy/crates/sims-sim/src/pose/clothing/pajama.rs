//! The 40e5ce/40e674 selection portion of 45f034, after 409315's migrations.
//! The caller owns binding and its post-migration backup/close. This reads the
//! bound profile without modifying it and never rechecks newly generated 33.
use super::{
    Best, Pattern, Profile,
    generator::{self, Bank, fixed},
};

struct Bound<'a, B>(&'a mut B);
impl<B: Bank> Profile for Bound<'_, B> {
    fn sex(&mut self) -> Result<i32, String> {
        generator::sex(self.0)
    }
    fn body_type(&mut self) -> Result<Vec<u8>, String> {
        generator::body_type(self.0)
    }
    fn tone(&mut self) -> Result<Vec<u8>, String> {
        generator::tone(self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    pub body: Vec<u8>,
    pub texture: Vec<u8>,
}

pub fn select(
    person_age: i16,
    bank: &mut impl Bank,
    bodies: &[Vec<u8>],
    textures: &[Vec<u8>],
) -> Result<Selected, String> {
    // Uses the live person age, not the profile age used by migration dispatch.
    // Age zero is an adult here; a negative signed word takes the child path.
    let template = if person_age != 0 && person_age < 18 {
        b"pjs$gchd".as_slice()
    } else {
        b"pajama$g$b"
    };
    select_templates(template, b"pajama$g$c", bank, bodies, textures)
}

/// Shared 40e5ce/40e674 calls used by pajamas and career uniforms. Both
/// wrappers use the same ordinary-texture suffix ranking, including its reads.
pub fn select_templates(
    body_template: &[u8],
    texture_template: &[u8],
    bank: &mut impl Bank,
    bodies: &[Vec<u8>],
    textures: &[Vec<u8>],
) -> Result<Selected, String> {
    let body_pattern = Pattern::expand(body_template, &mut Bound(bank))?;
    let body = body_pattern
        .first(bodies.iter().map(|v| fixed(v, 63)))
        .unwrap_or_default()
        .to_vec();
    let mut texture_pattern = Pattern::expand(texture_template, &mut Bound(bank))?;
    let normal = bank
        .read(2)?
        .ok_or("native pajama normal descriptor absent")?;
    let text = fixed(&normal, 255);
    let primary = text.split(|&b| b == b';').next().unwrap();
    let raw_texture = primary
        .iter()
        .position(|&b| b == b',')
        .map_or(&b""[..], |n| &primary[n + 1..]);
    let mut texture = fixed(raw_texture, 63).to_vec();
    let lower_start = texture.iter().position(|&b| b == b'=').map_or(0, |n| n + 1);
    texture[lower_start..].make_ascii_lowercase();
    // 40b75a skips BODY='s length plus 12 bytes, then permits an empty suffix.
    // A short descriptor would read outside its logical native string: retain
    // an explicit boundary rather than inventing those memory bytes.
    let suffix = texture
        .get(17..)
        .ok_or("native pajama suffix exceeds short normal texture")?;
    texture_pattern.either(suffix, b"");
    let mut best = Best::default();
    for candidate in textures {
        best.consider(&texture_pattern, fixed(candidate, 63));
    }
    let mut texture = vec![];
    if !best.name.is_empty() {
        texture.extend_from_slice(b"BODY=");
        texture.extend_from_slice(&best.name);
        texture.truncate(63);
    }
    Ok(Selected { body, texture })
}
