//!58e5db..58e620: projection onto the current walking segment.
//! Caller subtraction supplies the two back components on the x87 stack.
pub(super) fn progress(back: [f64; 2], relative: [f32; 2], control: u16) -> f32 {
    if control == 0x003f {
        let q = crate::pose::transform::retained_single;
        let [x, y] = back.map(q);
        let denominator = q(q(y * y) + q(x * x)) as f32;
        if denominator <= 0. {
            return 0.;
        }
        let dot = q(q(f64::from(relative[1]) * y) + q(f64::from(relative[0]) * x));
        q(1. - q(dot / f64::from(denominator))) as f32
    } else {
        // Preserve the existing037f profile's wider intermediates.
        let denominator = (back[0] * back[0] + back[1] * back[1]) as f32;
        if denominator <= 0. {
            0.
        } else {
            (1. - (f64::from(relative[1]) * back[1] + f64::from(relative[0]) * back[0])
                / f64::from(denominator)) as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Input {
        back: [f64; 2],
        relative: [f32; 2],
    }
    #[derive(Deserialize)]
    struct Row {
        input: Input,
        control: u16,
        progress_bits: u32,
    }
    #[derive(Deserialize)]
    struct Oracle {
        rows: Vec<Row>,
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn matches_original_projection_at_both_caller_precisions() {
        let oracle: Oracle = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/native-path-projection.json"
        ))
        .unwrap();
        assert_eq!(oracle.rows.len(), 256);
        for (i, row) in oracle.rows.iter().enumerate() {
            assert_eq!(
                progress(row.input.back, row.input.relative, row.control).to_bits(),
                row.progress_bits,
                "native projection row{i}"
            );
        }
    }
    #[test]
    fn captured_c18_projection_reaches_the_original_c19_sample() {
        let mut phase = progress([-23., 22.], [-24.457489013671875, 20.3466796875], 0x003f);
        assert_eq!(phase.to_bits(), 0x3b386f00);
        let step = 1_f32 / 1013_f32.sqrt();
        for _ in 0..5 {
            phase += step;
        }
        assert_eq!(phase.to_bits(), 0x3e23bf86);
    }
}
