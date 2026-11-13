//! 5920c6 and 58eb47..58ece9: direction, wrapped turn, and steering clamp.
use super::Settings;
use crate::pose::transform::retained_single;

fn round(value: f64, control: u16) -> f64 {
    if control == 0x003f {
        retained_single(value)
    } else {
        value
    }
}
fn normalized(mut value: f64, minimum: f64, control: u16) -> Result<f64, String> {
    if !matches!(control, 0x003f | 0x037f) {
        return Err("unsupported native steering x87 control".into());
    }
    if control == 0x037f {
        return super::super::normalized(value, minimum);
    }
    if !value.is_finite() || value.abs() > 1_000_000. {
        return Err("native heading normalization exceeds diagnostic budget".into());
    }
    while value < minimum {
        value = round(value + 8., control);
    }
    while value >= minimum + 8. {
        value = round(value - 8., control);
    }
    Ok(value)
}
/// FPATAN retains its result until the next arithmetic operation. 5920c6's
/// negative-angle adjustment and every caller arithmetic operation obey PC.
pub(super) fn direction(vector: [f32; 2], control: u16) -> Result<f64, String> {
    let q = |v| round(v, control);
    let period = q(f64::from(std::f32::consts::PI) * 2.);
    let mut angle = f64::from(vector[1]).atan2(f64::from(vector[0]));
    if angle < 0. {
        angle = q(angle + period);
    }
    normalized(q(q(q(angle * 8.) / period) + 2.), 0., control)
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Adjustment {
    Turn,
    Step { heading: f32, bending: bool },
}
pub(super) fn adjust(
    desired: f64,
    current: f32,
    rate: f32,
    species: i16,
    settings: Settings,
    control: u16,
) -> Result<Adjustment, String> {
    let q = |v| round(v, control);
    let turn = normalized(q(desired - f64::from(current)), -4., control)?;
    let threshold = if species > 1 {
        f64::from(0.8_f32)
    } else if rate < 1. {
        q(f64::from(settings.turn_threshold) / f64::from(rate))
    } else {
        f64::from(settings.turn_threshold)
    };
    if turn >= threshold || turn <= -threshold {
        return Ok(Adjustment::Turn);
    }
    let mut increment = if species > 1 {
        0.057_f32
    } else {
        settings.turn_step
    };
    if rate < 1. {
        increment /= rate;
    }
    let clamped = turn.clamp(-f64::from(increment), f64::from(increment));
    Ok(Adjustment::Step {
        heading: q(clamped + f64::from(current)) as f32,
        bending: clamped != turn,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_heading_and_turn_precision_cases_match_every_word_and_branch() {
        let source: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/native-path-heading-precision.json"
        )))
        .unwrap();
        let rows = source["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 512);
        let mut differences = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let c = &row["input"];
            let vector: [f32; 2] = serde_json::from_value(c["vector"].clone()).unwrap();
            let control = row["control"].as_u64().unwrap() as u16;
            let desired = direction(vector, control).unwrap();
            let action = adjust(
                desired,
                c["current"].as_f64().unwrap() as f32,
                c["rate"].as_f64().unwrap() as f32,
                c["species"].as_i64().unwrap() as i16,
                Settings::COMPLETE_COLLECTION,
                control,
            )
            .unwrap();
            let result = match action {
                Adjustment::Turn => {
                    json!({"kind":"turn", "value_bits": (desired as f32).to_bits()})
                }
                Adjustment::Step { heading, bending } => {
                    json!({"kind":"step", "value_bits":heading.to_bits(),"bending":bending})
                }
            };
            if result != row["result"]
                || (desired as f32).to_bits() != row["desired_bits"].as_u64().unwrap() as u32
            {
                differences.push(json!({"index":index,"input":c,"control":control,"native":row["result"],"rust":result,"native_desired":row["desired_bits"],"rust_desired":(desired as f32).to_bits()}));
            }
        }
        assert!(
            differences.is_empty(),
            "{} differences; first {:?}",
            differences.len(),
            differences.first()
        );
    }
}
