//! Final three passes of native 44bf1a. Params name frame locals; incomplete
//! parameter pairs use the caller's person fields. Level bounds also gate offers.
use crate::program::CareerCatalog;

pub fn format_careers(
    text: &str,
    catalog: &CareerCatalog,
    person: [i16; 3],
    locals: Option<&[i16]>,
) -> Result<String, String> {
    replace(text, catalog, person, locals.map_or(0, <[i16]>::len), |i| {
        locals
            .and_then(|v| v.get(i))
            .copied()
            .ok_or("Dialog career local absent".into())
    })
}

pub(super) fn replace(
    text: &str,
    catalog: &CareerCatalog,
    person: [i16; 3],
    count: usize,
    mut read: impl FnMut(usize) -> Result<i16, String>,
) -> Result<String, String> {
    if !text.contains("$Job") {
        return Ok(text.to_owned());
    }
    let records = catalog
        .text
        .as_ref()
        .ok_or("native career text overlays unavailable")?;
    if records.len() != catalog.ids.len() {
        return Err("career text vector differs from native identities".into());
    }
    let mut result = text.to_owned();
    let mut level = None;
    let mut iterations = 0;
    for (kind, token) in ["$JobOffer", "$JobDesc", "$Job"].into_iter().enumerate() {
        let mut start = 0;
        while start < result.len() {
            while start < result.len() && !result.is_char_boundary(start) {
                start += 1;
            }
            let Some(at) = result[start..].find(token).map(|i| i + start) else {
                break;
            };
            let (end, params) = super::parameters(&result, at + token.len());
            let job = if params.contains(&-1) {
                level = Some(person[1]);
                person[0]
            } else if params.iter().all(|&i| i >= 0 && (i as usize) < count) {
                let job = read(params[0] as usize)?;
                level = Some(read(params[1] as usize)?);
                job
            } else {
                // Native defaults DI to zero and retains the prior level word.
                0
            };
            if let Some(ordinal) = catalog.ids.iter().position(|&id| id == i32::from(job)) {
                let level =
                    level.ok_or("native malformed career token depends on caller pointer bits")?;
                let record = &records[ordinal];
                if let Some(level) = usize::try_from(level)
                    .ok()
                    .and_then(|i| record.levels.get(i))
                {
                    let replacement = match kind {
                        0 => gender_text(&record.offers, person[2]),
                        1 => &level.description,
                        _ => gender_text(&level.titles, person[2]),
                    };
                    result.replace_range(at..end, replacement);
                }
            }
            start = at + 1;
            iterations += 1;
            if iterations > 65536 || result.len() > 65536 {
                return Err("Dialog career expansion exceeds limit".into());
            }
        }
    }
    Ok(result)
}

fn gender_text(pair: &[String; 2], gender: i16) -> &str {
    &pair[usize::from(gender & 1 != 0 && !pair[1].is_empty())]
}
