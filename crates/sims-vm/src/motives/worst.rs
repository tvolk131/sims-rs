use crate::{Program, Thread, Vm};

impl Vm {
    pub(crate) fn find_worst_motives(
        &mut self,
        program: &Program,
        thread: &Thread,
        operands: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let frame = thread
            .stack
            .last()
            .ok_or("Find Five Worst Motives frame absent")?;
        let selector = u16::from_le_bytes([operands[4], operands[5]]);
        let target = match selector {
            1 => frame.stack_object,
            2 => return Err("Find Five Worst Motives invalid person (native error 0x0a)".into()),
            _ => frame.caller,
        };
        let object = self
            .world
            .objects
            .get(&target)
            .ok_or("Find Five Worst Motives invalid person (native error 0x0a)")?;
        let definition = object
            .definition
            .as_ref()
            .ok_or("Find Five Worst Motives person definition absent")?;
        let kind = program
            .modules
            .get(&definition.source)
            .and_then(|module| module.object_definition_words.get(&definition.object))
            .and_then(|words| words.get(9))
            .ok_or("Find Five Worst Motives person definition type absent")?;
        if *kind != 2 {
            return Err("Find Five Worst Motives invalid person (native error 0x0a)".into());
        }
        if u16::from_le_bytes([operands[6], operands[7]]) > 2 {
            return Err("Find Five Worst Motives invalid mode (native error 0x1e)".into());
        }
        let values = object
            .motives
            .as_ref()
            .ok_or("Find Five Worst Motives person motive storage absent")?;
        let ranked = ranked_motives(values);
        self.world
            .objects
            .get_mut(&frame.caller)
            .ok_or("Find Five Worst Motives executor absent")?
            .temps[..5]
            .copy_from_slice(&ranked);
        Ok((true, format!("five worst motives on #{target}: {ranked:?}")))
    }
}

fn ranked_motives(values: &[u32; 16]) -> [i16; 5] {
    let mut indices = [5usize, 6, 7, 8, 9, 13, 14, 15];
    for tail in (1..indices.len()).rev() {
        let mut greatest = 0;
        for candidate in 1..=tail {
            let difference = f32::from_bits(values[indices[candidate]])
                - f32::from_bits(values[indices[greatest]]);
            if f64::from(difference) > 0.01 {
                greatest = candidate;
            }
        }
        indices.swap(greatest, tail);
    }
    std::array::from_fn(|index| indices[index] as i16)
}

#[cfg(test)]
mod tests {
    use super::ranked_motives;

    #[test]
    fn unordered_and_infinite_differences_preserve_native_shortsort_ties() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(ranked_motives(&[value.to_bits(); 16]), [6, 7, 8, 9, 13]);
        }
    }

    #[test]
    fn unordered_values_do_not_replace_initial_or_later_maxima() {
        for (motives, expected) in [
            ([f32::NAN, -3., -2., -1., 0., 1., 2., 3.], [6, 7, 8, 9, 13]),
            ([-3., f32::NAN, -2., -1., 0., 1., 2., 3.], [6, 5, 7, 8, 9]),
        ] {
            let mut values = [0; 16];
            for (index, motive) in [5, 6, 7, 8, 9, 13, 14, 15].into_iter().zip(motives) {
                values[index] = motive.to_bits();
            }
            assert_eq!(ranked_motives(&values), expected);
        }
    }
}
