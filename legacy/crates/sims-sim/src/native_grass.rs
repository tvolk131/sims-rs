use crate::saved_matrix::{Allocation, Backing, Matrix, Stop};

const SIZE: usize = 64;
const DEFAULT_SEED: u32 = 0x1de48a;

fn backing(matrix: &Matrix) -> Result<&Backing, Stop> {
    if matrix.width != SIZE as i32 || matrix.height != SIZE as i32 || matrix.stride != 1 {
        return Err(Stop::Unsupported(
            "grass generation requires the original 64-square byte-matrix profile".into(),
        ));
    }
    let allocation = match &matrix.allocation {
        Allocation::Known(allocation) => allocation,
        Allocation::Unavailable => {
            return Err(Stop::Unavailable("grass matrix backing unavailable".into()));
        }
        _ => {
            return Err(Stop::Unsupported(
                "grass matrix backing absent or released".into(),
            ));
        }
    };
    if allocation.token == 0
        || allocation.data.len() != SIZE * SIZE
        || allocation.column_offsets
            != (0..SIZE)
                .map(|column| (column * SIZE) as u32)
                .collect::<Vec<_>>()
    {
        return Err(Stop::Unsupported(
            "grass matrix does not retain its original constructor allocation layout".into(),
        ));
    }
    Ok(allocation)
}

fn sample(data: &[u8], column: i32, row: i32) -> i32 {
    let column = column.rem_euclid(SIZE as i32) as usize;
    let row = row.rem_euclid(SIZE as i32) as usize;
    i32::from(data[column * SIZE + row] as i8)
}

fn refine(data: &mut [u8], step: usize, amplitude: u32, random: &mut u32) {
    for column in (0..SIZE).step_by(step) {
        for row in (0..SIZE).step_by(step) {
            let column_even = column % (step * 2) == 0;
            let row_even = row % (step * 2) == 0;
            if column_even && row_even {
                continue;
            }
            let column_signed = column as i32;
            let row_signed = row as i32;
            let distance = step as i32;
            let average = if column_even {
                (sample(data, column_signed, row_signed - distance)
                    + sample(data, column_signed, row_signed + distance))
                    / 2
            } else if row_even {
                (sample(data, column_signed - distance, row_signed)
                    + sample(data, column_signed + distance, row_signed))
                    / 2
            } else {
                (sample(data, column_signed - distance, row_signed - distance)
                    + sample(data, column_signed + distance, row_signed - distance)
                    + sample(data, column_signed - distance, row_signed + distance)
                    + sample(data, column_signed + distance, row_signed + distance))
                    / 4
            };
            let noise = (sims_vm::rng::next(random) % amplitude) as i32 - (amplitude / 2) as i32;
            data[column * SIZE + row] = (average + noise).clamp(-128, 127) as i8 as u8;
        }
    }
}

pub fn regenerate(
    grass: &mut Matrix,
    target: &mut Matrix,
    requested_seed: u32,
    random: &mut u32,
) -> Result<(), Stop> {
    let grass_token = backing(grass)?.token;
    let target_token = backing(target)?.token;
    if grass_token == target_token {
        return Err(Stop::Unsupported(
            "constructor grass matrices require distinct retained owners".into(),
        ));
    }
    let saved_random = *random;
    *random = if requested_seed == 0 {
        DEFAULT_SEED
    } else {
        requested_seed
    };
    let Allocation::Known(grass) = &mut grass.allocation else {
        unreachable!()
    };
    let Allocation::Known(target) = &mut target.allocation else {
        unreachable!()
    };
    for column in (0..SIZE).step_by(16) {
        for row in (0..SIZE).step_by(16) {
            let value = (sims_vm::rng::next(random) % 160) as i32 - 32;
            grass.data[column * SIZE + row] = value.clamp(-128, 127) as i8 as u8;
        }
    }
    for (step, amplitude) in [(8, 80), (4, 40), (2, 20), (1, 10)] {
        refine(&mut grass.data, step, amplitude, random);
    }
    *random = saved_random;
    target.data.copy_from_slice(&grass.data);
    Ok(())
}

#[cfg(test)]
mod tests;
