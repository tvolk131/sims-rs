//! Borrowed derived-vertex stores extracted from4778fc and479d9a.
//! Keep the two cursors separate across the actual object-construction barrier.
//! These stores do not implement active-size assignment, allocation, primary
//! rooms, room callbacks, or first-clear behavior. They are not native yields.
use crate::{
    native_wall_io,
    saved_matrix::{Allocation, Matrix, Stop},
    saved_world,
    surface_remap::Coord,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Layout {
    token: u64,
    width: i32,
    height: i32,
    stride: i32,
    columns: Vec<u32>,
    bytes: usize,
}
impl Layout {
    fn of(matrix: &Matrix) -> Result<Self, Stop> {
        let b = match &matrix.allocation {
            Allocation::Known(b) => b,
            Allocation::Unavailable => {
                return Err(Stop::Unavailable(
                    "vertex matrix allocation unavailable".into(),
                ));
            }
            _ => {
                return Err(Stop::Unsupported(
                    "vertex matrix allocation absent or released".into(),
                ));
            }
        };
        Ok(Self {
            token: b.token,
            width: matrix.width,
            height: matrix.height,
            stride: matrix.stride,
            columns: b.column_offsets.clone(),
            bytes: b.data.len(),
        })
    }
    fn matches(&self, matrix: &Matrix) -> Result<bool, Stop> {
        let b = match &matrix.allocation {
            Allocation::Known(b) => b,
            Allocation::Unavailable => {
                return Err(Stop::Unavailable(
                    "vertex matrix allocation unavailable".into(),
                ));
            }
            _ => return Ok(false),
        };
        Ok(self.token == b.token
            && self.width == matrix.width
            && self.height == matrix.height
            && self.stride == matrix.stride
            && self.columns == b.column_offsets
            && self.bytes == b.data.len())
    }
}

/// An actual single-byte store. `before` is deliberately not demanded: these
/// native instructions overwrite the destination without consuming its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    pub at: Coord,
    pub value: u8,
    pub instruction: u32,
}

fn check_layouts(
    matrices: [&Matrix; 2],
    layouts: &mut Option<[Layout; 2]>,
    stride: i32,
    active: u8,
) -> Result<(), Stop> {
    // This is an admitted host profile, not a fabricated native error/status.
    // Validate both distinct owners before entering the ordered store prefix.
    if let Some(expected) = layouts {
        for (matrix, layout) in matrices.iter().zip(expected) {
            if !layout.matches(matrix)? {
                return Err(Stop::Unsupported(
                    "retained matrix identity or layout changed during vertex stores".into(),
                ));
            }
        }
        return Ok(());
    }
    let current = [Layout::of(matrices[0])?, Layout::of(matrices[1])?];
    if current[0].token == current[1].token {
        return Err(Stop::Unsupported(
            "two levels need distinct retained matrix owners".into(),
        ));
    }
    for (matrix, layout) in matrices.iter().zip(&current) {
        if matrix.width < i32::from(active)
            || matrix.height < i32::from(active)
            || matrix.stride != stride
            || layout.columns.len() < usize::from(active)
        {
            return Err(Stop::Unsupported(
                "vertex operation outside retained matrix layout".into(),
            ));
        }
    }
    *layouts = Some(current);
    Ok(())
}

fn store(matrix: &mut Matrix, write: Write) -> Result<Write, Stop> {
    saved_world::cell_mut(matrix, usize::from(write.at.x), usize::from(write.at.y), 1)?[0] =
        write.value;
    Ok(write)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Border {
    FarX,
    FarY,
    Complete,
}

/// Existing-backing shrink stores only. No allocation/replacement profile is
/// guessed. Retain this cursor and both owners together, without interleaved
/// mutations, if a research/checkpoint boundary splits the native call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShrinkBorders {
    active: u8,
    border: Border,
    coordinate: u8,
    level: u8,
    layouts: Option<[Layout; 2]>,
}
impl ShrinkBorders {
    pub fn new(old_active: i32, new_active: i32) -> Result<Self, Stop> {
        if !(4..=127).contains(&new_active) || !(new_active..=127).contains(&old_active) {
            return Err(Stop::Unsupported(
                "outside existing-backing shrink profile".into(),
            ));
        }
        Ok(Self {
            active: new_active as u8,
            border: if old_active == new_active {
                Border::Complete
            } else {
                Border::FarX
            },
            coordinate: 0,
            level: 1,
            layouts: None,
        })
    }
    pub fn completed(&self) -> bool {
        self.border == Border::Complete
    }
    pub fn step(&mut self, vertices: &mut [Matrix; 2]) -> Result<Option<Write>, Stop> {
        if self.completed() {
            return Ok(None);
        }
        check_layouts(
            [&vertices[0], &vertices[1]],
            &mut self.layouts,
            1,
            self.active,
        )?;
        let (x, y, instruction) = match self.border {
            Border::FarX => (self.active - 1, self.coordinate, 0x477a4b),
            Border::FarY => (self.coordinate, self.active - 1, 0x477bb6),
            Border::Complete => unreachable!(),
        };
        let write = store(
            &mut vertices[usize::from(self.level - 1)],
            Write {
                at: Coord {
                    x,
                    y,
                    level: self.level,
                },
                value: 0,
                instruction,
            },
        )?;
        if self.level == 1 {
            self.level = 2;
        } else {
            self.level = 1;
            self.coordinate += 1;
            if self.coordinate == self.active {
                self.coordinate = 0;
                self.border = match self.border {
                    Border::FarX => Border::FarY,
                    Border::FarY => Border::Complete,
                    Border::Complete => unreachable!(),
                };
            }
        }
        Ok(Some(write))
    }
}

/// Only the vertex-byte output of479d9a: level1, then level2, X outer/Y inner.
/// The original routine also performs room operations between these stores;
/// callers must preserve those effects/order separately. The raw matrices and
/// this cursor must not be mutated/interleaved while executing this component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rebuild {
    active: u8,
    at: Coord,
    complete: bool,
    walls: Option<[Layout; 2]>,
    vertices: Option<[Layout; 2]>,
}
impl Rebuild {
    pub fn new(active: i32) -> Result<Self, Stop> {
        if !(0..=127).contains(&active) {
            return Err(Stop::Unsupported(
                "outside signed-coordinate vertex rebuild profile".into(),
            ));
        }
        Ok(Self {
            active: active as u8,
            at: Coord {
                x: 0,
                y: 0,
                level: 1,
            },
            complete: active == 0,
            walls: None,
            vertices: None,
        })
    }
    pub fn completed(&self) -> bool {
        self.complete
    }
    pub fn step(
        &mut self,
        walls: [&Matrix; 2],
        vertices: &mut [Matrix; 2],
    ) -> Result<Option<Write>, Stop> {
        if self.complete {
            return Ok(None);
        }
        check_layouts(
            [&vertices[0], &vertices[1]],
            &mut self.vertices,
            1,
            self.active,
        )?;
        // For active<=2 the native helper reads no raw wall cell at all.
        if self.active > 2 {
            check_layouts(walls, &mut self.walls, 14, self.active)?;
            if self.walls.as_ref().unwrap().iter().any(|w| {
                self.vertices
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|v| v.token == w.token)
            }) {
                return Err(Stop::Unsupported(
                    "wall/vertex owners alias outside borrowed profile".into(),
                ));
            }
        }
        let value = native_wall_io::vertex(
            walls[usize::from(self.at.level - 1)],
            i32::from(self.active),
            self.at,
        )?;
        let write = store(
            &mut vertices[usize::from(self.at.level - 1)],
            Write {
                at: self.at,
                value,
                instruction: 0x479f72,
            },
        )?;
        self.at.y += 1;
        if self.at.y == self.active {
            self.at.y = 0;
            self.at.x += 1;
            if self.at.x == self.active {
                self.at.x = 0;
                if self.at.level == 1 {
                    self.at.level = 2;
                } else {
                    self.complete = true;
                }
            }
        }
        Ok(Some(write))
    }
}

#[cfg(test)]
mod tests;
