//! Arithmetic and measure/draw ordering in 4384c8, after local view setup.
//! The caller owns the resource, optional draw settings and local view. Before
//! each request it sets the view zoom (4b4509), projects the lot-center point
//! (4b9613), then calls 43878f with the supplied arguments. Neither this helper
//! nor a renderer is allowed to advance gameplay to obtain a preview.
use serde::{Deserialize, Serialize};

pub type Rect = [i32; 4];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    /// Signed normalized OBJD words 10, 11 and 61.
    pub group: i16,
    pub subindex: i16,
    pub direction: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub zoom: i32,
    pub facing: i32,
    pub offset: [i32; 2],
    pub bounds_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub zoom: i32,
    /// Non-base multipart definitions return 1 without writing the output rect.
    pub bounds: Option<Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Measure(i32),
    Draw { zoom: i32, offset: [i32; 2] },
    Done(Output),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fit {
    definition: Definition,
    target: [i32; 2],
    supplied_zoom: Option<i32>,
    stage: Stage,
}

impl Fit {
    pub fn new(definition: Definition, target: [i32; 2], zoom: Option<i32>) -> Self {
        Self {
            definition,
            target,
            supplied_zoom: zoom,
            stage: if definition.group != 0 && definition.subindex != -1 {
                Stage::Done(Output {
                    zoom: 1,
                    bounds: None,
                })
            } else {
                // A nonnull native zoom pointer is used verbatim, even if its
                // value lies outside 1..3. Do not confuse it with automatic fit.
                Stage::Measure(zoom.unwrap_or(3))
            },
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let skipped = self.definition.group != 0 && self.definition.subindex != -1;
        let zoom = match self.stage {
            Stage::Done(Output {
                zoom: 1,
                bounds: None,
            }) if skipped => return Ok(()),
            Stage::Measure(z) | Stage::Draw { zoom: z, .. } if !skipped => z,
            Stage::Done(Output {
                zoom,
                bounds: Some(_),
            }) if !skipped => zoom,
            _ => return Err("invalid object preview fitting stage".into()),
        };
        if self
            .supplied_zoom
            .map_or(!(1..=3).contains(&zoom), |z| zoom != z)
        {
            return Err("object preview fitting zoom changed".into());
        }
        Ok(())
    }

    pub fn pending(&self) -> Option<Request> {
        let (zoom, offset, bounds_only) = match self.stage {
            Stage::Measure(zoom) => (zoom, [0, 0], true),
            Stage::Draw { zoom, offset } => (zoom, offset, false),
            Stage::Done(_) => return None,
        };
        Some(Request {
            zoom,
            // 4385bb..4385c4 is signed IDIV, not unsigned wrapping/modulo.
            facing: (i32::from(self.definition.direction) + 4) % 8,
            offset,
            bounds_only,
        })
    }

    /// Bind each reply to the exact outstanding request. The caller may save
    /// and restore before rendering, but may not acknowledge a different zoom
    /// or publish a dialog while measurement/drawing is still pending.
    pub fn respond(&mut self, request: Request, bounds: Rect) -> Result<(), String> {
        self.validate()?;
        if self.pending() != Some(request) {
            return Err("object preview reply does not match pending draw".into());
        }
        match self.stage {
            Stage::Measure(zoom) => {
                // x86 SUB/ADD wrap before signed division truncates toward 0.
                // Keeping these operations in this order matters for the
                // empty-rect sentinel and out-of-range caller dimensions.
                let margins = [0, 1].map(|n| {
                    bounds[n]
                        .wrapping_sub(bounds[n + 2])
                        .wrapping_add(self.target[n])
                        / 2
                });
                let offset = [0, 1].map(|n| margins[n].wrapping_sub(bounds[n]));
                if self.supplied_zoom.is_none() && zoom > 1 && (margins[0] < 3 || margins[1] < 3) {
                    self.stage = Stage::Measure(zoom - 1);
                } else {
                    // If even zoom 1 is too large, draw at 1 using its measured
                    // offsets. There is no additional scale/crop adjustment.
                    self.stage = Stage::Draw { zoom, offset };
                }
            }
            Stage::Draw { zoom, .. } => {
                // The native output is the DRAW result, not the measured rect
                // translated by our chosen offsets.
                self.stage = Stage::Done(Output {
                    zoom,
                    bounds: Some(bounds),
                });
            }
            Stage::Done(_) => unreachable!(),
        }
        Ok(())
    }

    pub fn finish(&self) -> Result<Output, String> {
        self.validate()?;
        match self.stage {
            Stage::Done(output) => Ok(output),
            _ => Err("object preview measurement or draw is pending".into()),
        }
    }
}
