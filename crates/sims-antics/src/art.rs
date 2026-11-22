//! Deterministic procedural isometric furniture rasterizer.
//!
//! Renders 3D box-composite furniture across all 4 Sims 1 DGRP cardinal
//! rotations (`1, 4, 16, 64`) and all 3 zoom levels (`1` far, `2` mid, `3`
//! near) into 5-6-5 quantized `PALT` + z-buffered/alpha `SPR2` + `DGRP` chunks.

use crate::ir::{DgrpChunk, PaltChunk, Spr2Chunk, Spr2Frame, Spr2Pixel};
use crate::manifest::SpriteSpec;
use sims_formats::sprite::{DrawSprite, DrawView};
use std::collections::BTreeMap;

/// The 4 DGRP direction bitmasks expected by `sims-render` (NE/SE/SW/NW order).
pub const DGRP_DIRECTIONS: [u32; 4] = [1, 4, 16, 64];
/// The 3 DGRP zoom levels (`1` = far, `2` = mid, `3` = near).
pub const DGRP_ZOOMS: [u32; 3] = [1, 2, 3];

/// Built-in geometric furniture silhouettes for clean-room starter objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FurnitureShape {
    Bench,
    Cot,
    Basin,
    Table,
    Chair,
    Box,
    DoubleBed,
    Fridge,
    Counter,
    Stove,
    MealPlate,
    WallPhone,
    Shower,
}

/// An axis-aligned 3D box in normalized tile space (`x, y in [-0.5, 0.5]`, `z >= 0`).
#[derive(Debug, Clone, Copy)]
pub struct IsoBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub base_rgb: [u8; 3],
}

/// Generated sprite bundle ready to encode into an IFF file.
#[derive(Debug, Clone)]
pub struct GeneratedSprites {
    pub palt: PaltChunk,
    pub spr2: Spr2Chunk,
    pub dgrp: DgrpChunk,
}

/// Generates complete 4-rotation × 3-zoom isometric `PALT` + `SPR2` + `DGRP`
/// chunks for a procedural furniture specification.
#[must_use]
pub fn generate_furniture_sprites(
    spec: &SpriteSpec,
    palt_id: u16,
    spr2_id: u16,
    dgrp_id: u16,
    label: &str,
) -> GeneratedSprites {
    let boxes = shape_geometry(spec.shape, spec.primary_rgb, spec.accent_rgb, spec.trim_rgb);
    let mut palette = PaletteBuilder::new();
    let mut frames = Vec::with_capacity(12);
    let mut views = Vec::with_capacity(12);

    for (dir_idx, &direction) in DGRP_DIRECTIONS.iter().enumerate() {
        for &zoom in &DGRP_ZOOMS {
            let frame_index = frames.len() as u32;
            let frame = rasterize_view(&boxes, dir_idx, zoom, palt_id, &mut palette);
            let anchor_x = -(i32::from(frame.width) / 2);
            let anchor_y = -(i32::from(frame.height) * 3 / 4);
            frames.push(frame);
            views.push(DrawView {
                direction,
                zoom,
                sprites: vec![DrawSprite {
                    id: u32::from(spr2_id),
                    frame: frame_index,
                    x: anchor_x,
                    y: anchor_y,
                    flags: 0,
                    object: [0.0, 0.0, 0.0],
                }],
            });
        }
    }

    GeneratedSprites {
        palt: PaltChunk {
            id: palt_id,
            label: format!("{label} Palette"),
            colors: palette.colors,
        },
        spr2: Spr2Chunk {
            id: spr2_id,
            label: format!("{label} Sprites"),
            version: 1001,
            default_palette: u32::from(palt_id),
            frames,
        },
        dgrp: DgrpChunk {
            id: dgrp_id,
            label: format!("{label} DrawGroup"),
            version: 20004,
            views,
        },
    }
}

fn shape_geometry(
    shape: FurnitureShape,
    primary: [u8; 3],
    accent: [u8; 3],
    trim: [u8; 3],
) -> Vec<IsoBox> {
    match shape {
        FurnitureShape::Bench => vec![
            // Four corner legs
            IsoBox {
                min: [-0.42, -0.22, 0.0],
                max: [-0.34, -0.14, 0.28],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.34, -0.22, 0.0],
                max: [0.42, -0.14, 0.28],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.42, 0.14, 0.0],
                max: [-0.34, 0.22, 0.28],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.34, 0.14, 0.0],
                max: [0.42, 0.22, 0.28],
                base_rgb: trim,
            },
            // Bench seat plank
            IsoBox {
                min: [-0.45, -0.24, 0.28],
                max: [0.45, 0.24, 0.38],
                base_rgb: primary,
            },
            // Slatted backrest
            IsoBox {
                min: [-0.44, 0.16, 0.38],
                max: [0.44, 0.24, 0.72],
                base_rgb: accent,
            },
        ],
        FurnitureShape::Cot => vec![
            // Frame legs
            IsoBox {
                min: [-0.44, -0.44, 0.0],
                max: [-0.36, -0.36, 0.22],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.36, -0.44, 0.0],
                max: [0.44, -0.36, 0.22],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.44, 0.36, 0.0],
                max: [-0.36, 0.44, 0.22],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.36, 0.36, 0.0],
                max: [0.44, 0.44, 0.22],
                base_rgb: trim,
            },
            // Cot canvas base
            IsoBox {
                min: [-0.44, -0.44, 0.22],
                max: [0.44, 0.44, 0.36],
                base_rgb: primary,
            },
            // Pillow & blanket fold
            IsoBox {
                min: [-0.36, 0.18, 0.36],
                max: [0.36, 0.40, 0.48],
                base_rgb: accent,
            },
            IsoBox {
                min: [-0.42, -0.42, 0.36],
                max: [0.42, 0.14, 0.42],
                base_rgb: trim,
            },
        ],
        FurnitureShape::Basin => vec![
            // Ceramic pedestal
            IsoBox {
                min: [-0.20, -0.20, 0.0],
                max: [0.20, 0.20, 0.36],
                base_rgb: primary,
            },
            // Bowl rim
            IsoBox {
                min: [-0.32, -0.28, 0.36],
                max: [0.32, 0.28, 0.56],
                base_rgb: accent,
            },
            // Rear cistern / backsplash
            IsoBox {
                min: [-0.28, 0.14, 0.56],
                max: [0.28, 0.30, 0.92],
                base_rgb: primary,
            },
            // Metallic flush handle accent
            IsoBox {
                min: [-0.34, 0.18, 0.80],
                max: [-0.28, 0.24, 0.86],
                base_rgb: trim,
            },
        ],
        FurnitureShape::Table => vec![
            IsoBox {
                min: [-0.40, -0.40, 0.0],
                max: [-0.32, -0.32, 0.55],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.32, -0.40, 0.0],
                max: [0.40, -0.32, 0.55],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.40, 0.32, 0.0],
                max: [-0.32, 0.40, 0.55],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.32, 0.32, 0.0],
                max: [0.40, 0.40, 0.55],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.45, -0.45, 0.55],
                max: [0.45, 0.45, 0.65],
                base_rgb: primary,
            },
        ],
        FurnitureShape::Chair => vec![
            IsoBox {
                min: [-0.26, -0.26, 0.0],
                max: [-0.18, -0.18, 0.30],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.18, -0.26, 0.0],
                max: [0.26, -0.18, 0.30],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.26, 0.18, 0.0],
                max: [-0.18, 0.26, 0.30],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.18, 0.18, 0.0],
                max: [0.26, 0.26, 0.30],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.28, -0.28, 0.30],
                max: [0.28, 0.28, 0.38],
                base_rgb: primary,
            },
            IsoBox {
                min: [-0.28, 0.20, 0.38],
                max: [0.28, 0.28, 0.78],
                base_rgb: accent,
            },
        ],
        FurnitureShape::Box => vec![IsoBox {
            min: [-0.35, -0.35, 0.0],
            max: [0.35, 0.35, 0.60],
            base_rgb: primary,
        }],
        FurnitureShape::DoubleBed => vec![
            // Corner posts
            IsoBox {
                min: [-0.46, -0.46, 0.0],
                max: [-0.38, -0.38, 0.50],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.38, -0.46, 0.0],
                max: [0.46, -0.38, 0.50],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.46, 0.38, 0.0],
                max: [-0.38, 0.46, 0.86],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.38, 0.38, 0.0],
                max: [0.46, 0.46, 0.86],
                base_rgb: trim,
            },
            // Wooden base plinth & headboard
            IsoBox {
                min: [-0.46, -0.46, 0.14],
                max: [0.46, 0.46, 0.28],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.44, 0.38, 0.28],
                max: [0.44, 0.46, 0.80],
                base_rgb: trim,
            },
            // Low footboard rail
            IsoBox {
                min: [-0.44, -0.46, 0.28],
                max: [0.44, -0.40, 0.44],
                base_rgb: trim,
            },
            // Mattress & quilted comforter
            IsoBox {
                min: [-0.42, -0.40, 0.28],
                max: [0.42, 0.38, 0.46],
                base_rgb: primary,
            },
            // Twin pillows
            IsoBox {
                min: [-0.36, 0.16, 0.46],
                max: [-0.04, 0.34, 0.56],
                base_rgb: accent,
            },
            IsoBox {
                min: [0.04, 0.16, 0.46],
                max: [0.36, 0.34, 0.56],
                base_rgb: accent,
            },
            // Folded runner blanket
            IsoBox {
                min: [-0.41, -0.38, 0.46],
                max: [0.41, -0.10, 0.51],
                base_rgb: accent,
            },
        ],
        FurnitureShape::Fridge => vec![
            // Base toe kick
            IsoBox {
                min: [-0.36, -0.34, 0.0],
                max: [0.36, 0.34, 0.08],
                base_rgb: trim,
            },
            // Main lower refrigerator compartment
            IsoBox {
                min: [-0.38, -0.36, 0.08],
                max: [0.38, 0.36, 0.78],
                base_rgb: primary,
            },
            // Recessed door divider gasket
            IsoBox {
                min: [-0.36, -0.35, 0.78],
                max: [0.36, 0.35, 0.82],
                base_rgb: trim,
            },
            // Upper freezer compartment
            IsoBox {
                min: [-0.38, -0.36, 0.82],
                max: [0.38, 0.36, 1.16],
                base_rgb: primary,
            },
            // Top trim cap
            IsoBox {
                min: [-0.36, -0.34, 1.16],
                max: [0.36, 0.34, 1.20],
                base_rgb: accent,
            },
            // Vertical handles on front (-y) face
            IsoBox {
                min: [-0.30, -0.41, 0.34],
                max: [-0.24, -0.36, 0.72],
                base_rgb: accent,
            },
            IsoBox {
                min: [-0.30, -0.41, 0.88],
                max: [-0.24, -0.36, 1.08],
                base_rgb: accent,
            },
        ],
        FurnitureShape::Counter => vec![
            // Recessed toe kick
            IsoBox {
                min: [-0.44, -0.38, 0.0],
                max: [0.44, 0.44, 0.08],
                base_rgb: trim,
            },
            // Cabinet carcass
            IsoBox {
                min: [-0.46, -0.42, 0.08],
                max: [0.46, 0.46, 0.56],
                base_rgb: primary,
            },
            // Front drawer handle bar
            IsoBox {
                min: [-0.18, -0.46, 0.44],
                max: [0.18, -0.42, 0.48],
                base_rgb: trim,
            },
            // Overhanging countertop slab
            IsoBox {
                min: [-0.48, -0.46, 0.56],
                max: [0.48, 0.48, 0.64],
                base_rgb: accent,
            },
            // Rear backsplash lip
            IsoBox {
                min: [-0.48, 0.40, 0.64],
                max: [0.48, 0.48, 0.74],
                base_rgb: accent,
            },
        ],
        FurnitureShape::Stove => vec![
            // Oven body
            IsoBox {
                min: [-0.44, -0.42, 0.0],
                max: [0.44, 0.44, 0.58],
                base_rgb: primary,
            },
            // Oven door glass window
            IsoBox {
                min: [-0.28, -0.45, 0.16],
                max: [0.28, -0.42, 0.42],
                base_rgb: trim,
            },
            // Oven handle bar
            IsoBox {
                min: [-0.30, -0.47, 0.46],
                max: [0.30, -0.43, 0.50],
                base_rgb: accent,
            },
            // Cooktop deck
            IsoBox {
                min: [-0.46, -0.44, 0.58],
                max: [0.46, 0.46, 0.64],
                base_rgb: accent,
            },
            // Four burner grates
            IsoBox {
                min: [-0.34, -0.30, 0.64],
                max: [-0.10, -0.06, 0.67],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.10, -0.30, 0.64],
                max: [0.34, -0.06, 0.67],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.34, 0.06, 0.64],
                max: [-0.10, 0.30, 0.67],
                base_rgb: trim,
            },
            IsoBox {
                min: [0.10, 0.06, 0.64],
                max: [0.34, 0.30, 0.67],
                base_rgb: trim,
            },
            // Rear control console riser
            IsoBox {
                min: [-0.44, 0.34, 0.64],
                max: [0.44, 0.46, 0.84],
                base_rgb: primary,
            },
        ],
        FurnitureShape::MealPlate => vec![
            // Foot ring
            IsoBox {
                min: [-0.18, -0.18, 0.0],
                max: [0.18, 0.18, 0.04],
                base_rgb: trim,
            },
            // Ceramic plate platter
            IsoBox {
                min: [-0.28, -0.28, 0.04],
                max: [0.28, 0.28, 0.09],
                base_rgb: primary,
            },
            // Plated entree mound
            IsoBox {
                min: [-0.16, -0.14, 0.09],
                max: [0.12, 0.14, 0.17],
                base_rgb: accent,
            },
            // Side garnish
            IsoBox {
                min: [0.06, -0.16, 0.09],
                max: [0.20, 0.02, 0.15],
                base_rgb: trim,
            },
        ],
        FurnitureShape::WallPhone => vec![
            // Wall-mounted backplate
            IsoBox {
                min: [-0.16, 0.28, 0.56],
                max: [0.16, 0.44, 0.94],
                base_rgb: primary,
            },
            // Keypad faceplate
            IsoBox {
                min: [-0.10, 0.24, 0.62],
                max: [0.10, 0.28, 0.82],
                base_rgb: accent,
            },
            // Handset body on left cradle
            IsoBox {
                min: [-0.24, 0.22, 0.60],
                max: [-0.14, 0.34, 0.96],
                base_rgb: trim,
            },
            // Handset ear/mouth caps
            IsoBox {
                min: [-0.26, 0.18, 0.86],
                max: [-0.14, 0.34, 0.98],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.26, 0.18, 0.58],
                max: [-0.14, 0.34, 0.68],
                base_rgb: trim,
            },
        ],
        FurnitureShape::Shower => vec![
            // Porcelain shower base pan
            IsoBox {
                min: [-0.46, -0.46, 0.0],
                max: [0.46, 0.46, 0.10],
                base_rgb: primary,
            },
            // Rear tiled wall
            IsoBox {
                min: [-0.46, 0.38, 0.10],
                max: [0.46, 0.46, 1.18],
                base_rgb: primary,
            },
            // Right privacy side wall
            IsoBox {
                min: [0.38, -0.44, 0.10],
                max: [0.46, 0.46, 1.18],
                base_rgb: primary,
            },
            // Frosted glass left panel
            IsoBox {
                min: [-0.46, -0.44, 0.10],
                max: [-0.40, 0.40, 1.14],
                base_rgb: accent,
            },
            // Chrome mixing valve
            IsoBox {
                min: [-0.08, 0.32, 0.56],
                max: [0.08, 0.38, 0.68],
                base_rgb: trim,
            },
            // Overhead shower riser & head
            IsoBox {
                min: [-0.04, 0.22, 1.04],
                max: [0.04, 0.38, 1.10],
                base_rgb: trim,
            },
            IsoBox {
                min: [-0.10, 0.14, 0.98],
                max: [0.10, 0.26, 1.04],
                base_rgb: trim,
            },
        ],
    }
}

/// Quantizes RGB to 5-6-5 color space and deduplicates entries in a 256-entry palette.
struct PaletteBuilder {
    colors: Vec<[u8; 3]>,
    index: BTreeMap<[u8; 3], u8>,
}

impl PaletteBuilder {
    fn new() -> Self {
        // Reserve index 0 as transparent key color.
        let mut index = BTreeMap::new();
        index.insert([0, 0, 0], 0);
        Self {
            colors: vec![[0, 0, 0]],
            index,
        }
    }

    fn quantize_565(rgb: [u8; 3]) -> [u8; 3] {
        let r = (rgb[0] & 0xF8) | (rgb[0] >> 5);
        let g = (rgb[1] & 0xFC) | (rgb[1] >> 6);
        let b = (rgb[2] & 0xF8) | (rgb[2] >> 5);
        [r, g, b]
    }

    fn intern(&mut self, rgb: [u8; 3]) -> u8 {
        let q = Self::quantize_565(rgb);
        if let Some(&idx) = self.index.get(&q) {
            return idx;
        }
        if self.colors.len() < 256 {
            let idx = self.colors.len() as u8;
            self.colors.push(q);
            self.index.insert(q, idx);
            return idx;
        }
        // Fallback: nearest existing palette entry (excluding transparent index 0).
        let mut best_idx = 1u8;
        let mut best_dist = i32::MAX;
        for (i, c) in self.colors.iter().enumerate().skip(1) {
            let dr = i32::from(c[0]) - i32::from(q[0]);
            let dg = i32::from(c[1]) - i32::from(q[1]);
            let db = i32::from(c[2]) - i32::from(q[2]);
            let dist = dr * dr + dg * dg + db * db;
            if dist < best_dist {
                best_dist = dist;
                best_idx = i as u8;
            }
        }
        best_idx
    }
}

fn shade(rgb: [u8; 3], factor_num: u16, factor_den: u16) -> [u8; 3] {
    let scale = |c: u8| ((u16::from(c) * factor_num / factor_den).min(255)) as u8;
    [scale(rgb[0]), scale(rgb[1]), scale(rgb[2])]
}

fn rotate_box(b: IsoBox, dir_idx: usize) -> IsoBox {
    let corners = [
        [b.min[0], b.min[1]],
        [b.max[0], b.min[1]],
        [b.min[0], b.max[1]],
        [b.max[0], b.max[1]],
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for [x, y] in corners {
        let (rx, ry) = match dir_idx & 3 {
            0 => (x, y),
            1 => (-y, x),
            2 => (-x, -y),
            _ => (y, -x),
        };
        if rx < min_x {
            min_x = rx;
        }
        if rx > max_x {
            max_x = rx;
        }
        if ry < min_y {
            min_y = ry;
        }
        if ry > max_y {
            max_y = ry;
        }
    }
    IsoBox {
        min: [min_x, min_y, b.min[2]],
        max: [max_x, max_y, b.max[2]],
        base_rgb: b.base_rgb,
    }
}

fn rasterize_view(
    boxes: &[IsoBox],
    dir_idx: usize,
    zoom: u32,
    palt_id: u16,
    palette: &mut PaletteBuilder,
) -> Spr2Frame {
    // Tile pixel scales matching Sims 1 zooms: far=32x16, mid=64x32, near=128x64.
    let (width, height, half_w, half_h, z_scale) = match zoom {
        1 => (32usize, 36usize, 16.0f32, 8.0f32, 24.0f32),
        2 => (64usize, 72usize, 32.0f32, 16.0f32, 48.0f32),
        _ => (128usize, 144usize, 64.0f32, 32.0f32, 96.0f32),
    };
    let origin_x = (width as f32) * 0.5;
    let origin_y = (height as f32) * 0.78;

    let mut pixels = vec![
        Spr2Pixel {
            color: 0,
            depth: 255,
            alpha: 0,
        };
        width * height
    ];

    let project = |x: f32, y: f32, z: f32| -> (f32, f32, u8) {
        let sx = origin_x + (x - y) * half_w;
        let sy = origin_y + (x + y) * half_h - z * z_scale;
        // Smaller depth = closer to camera (larger x + y + z*0.5 is closer).
        let raw_depth = 160.0 - ((x + y) * 48.0 + z * 28.0);
        let depth = raw_depth.clamp(8.0, 250.0) as u8;
        (sx, sy, depth)
    };

    for &orig in boxes {
        let b = rotate_box(orig, dir_idx);
        let top_col = palette.intern(shade(b.base_rgb, 112, 100));
        let left_col = palette.intern(shade(b.base_rgb, 86, 100));
        let right_col = palette.intern(shade(b.base_rgb, 68, 100));

        // Top face: z = max[2]
        let p_t0 = project(b.min[0], b.min[1], b.max[2]);
        let p_t1 = project(b.max[0], b.min[1], b.max[2]);
        let p_t2 = project(b.max[0], b.max[1], b.max[2]);
        let p_t3 = project(b.min[0], b.max[1], b.max[2]);
        fill_quad(
            &mut pixels,
            width,
            height,
            [p_t0, p_t1, p_t2, p_t3],
            top_col,
        );

        // Front-left face: y = max[1] (faces +y -> screen bottom-left)
        let p_l0 = project(b.min[0], b.max[1], b.max[2]);
        let p_l1 = project(b.max[0], b.max[1], b.max[2]);
        let p_l2 = project(b.max[0], b.max[1], b.min[2]);
        let p_l3 = project(b.min[0], b.max[1], b.min[2]);
        fill_quad(
            &mut pixels,
            width,
            height,
            [p_l0, p_l1, p_l2, p_l3],
            left_col,
        );

        // Front-right face: x = max[0] (faces +x -> screen bottom-right)
        let p_r0 = project(b.max[0], b.min[1], b.max[2]);
        let p_r1 = project(b.max[0], b.max[1], b.max[2]);
        let p_r2 = project(b.max[0], b.max[1], b.min[2]);
        let p_r3 = project(b.max[0], b.min[1], b.min[2]);
        fill_quad(
            &mut pixels,
            width,
            height,
            [p_r0, p_r1, p_r2, p_r3],
            right_col,
        );
    }

    // Soften silhouette perimeter pixels to 5-bit alpha 24 for clean alpha channel coverage.
    let snapshot = pixels.clone();
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            let idx = y * width + x;
            if snapshot[idx].alpha == 31 {
                let neighbors_empty = snapshot[idx - 1].alpha == 0
                    || snapshot[idx + 1].alpha == 0
                    || snapshot[idx - width].alpha == 0
                    || snapshot[idx + width].alpha == 0;
                if neighbors_empty {
                    pixels[idx].alpha = 26;
                }
            }
        }
    }

    Spr2Frame {
        width: width as u16,
        height: height as u16,
        flags: 0x07, // color + z-buffer + alpha
        palette_id: palt_id,
        transparent_index: 0,
        x: 0,
        y: 0,
        pixels,
    }
}

fn fill_quad(
    pixels: &mut [Spr2Pixel],
    width: usize,
    height: usize,
    quad: [(f32, f32, u8); 4],
    color: u8,
) {
    fill_triangle(pixels, width, height, quad[0], quad[1], quad[2], color);
    fill_triangle(pixels, width, height, quad[0], quad[2], quad[3], color);
}

fn fill_triangle(
    pixels: &mut [Spr2Pixel],
    width: usize,
    height: usize,
    v0: (f32, f32, u8),
    v1: (f32, f32, u8),
    v2: (f32, f32, u8),
    color: u8,
) {
    let min_x = libm::floorf(v0.0.min(v1.0).min(v2.0)).max(0.0) as usize;
    let max_x = (libm::ceilf(v0.0.max(v1.0).max(v2.0)) as usize).min(width.saturating_sub(1));
    let min_y = libm::floorf(v0.1.min(v1.1).min(v2.1)).max(0.0) as usize;
    let max_y = (libm::ceilf(v0.1.max(v1.1).max(v2.1)) as usize).min(height.saturating_sub(1));

    let edge = |a: (f32, f32), b: (f32, f32), p: (f32, f32)| -> f32 {
        (p.0 - a.0) * (b.1 - a.1) - (p.1 - a.1) * (b.0 - a.0)
    };
    let area = edge((v0.0, v0.1), (v1.0, v1.1), (v2.0, v2.1));
    if libm::fabsf(area) < 1e-4 {
        return;
    }
    let inv_area = 1.0 / area;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = edge((v1.0, v1.1), (v2.0, v2.1), p) * inv_area;
            let w1 = edge((v2.0, v2.1), (v0.0, v0.1), p) * inv_area;
            let w2 = edge((v0.0, v0.1), (v1.0, v1.1), p) * inv_area;
            if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                let z = (w0 * f32::from(v0.2) + w1 * f32::from(v1.2) + w2 * f32::from(v2.2))
                    .clamp(0.0, 254.0) as u8;
                let idx = y * width + x;
                if pixels[idx].alpha == 0 || z <= pixels[idx].depth {
                    pixels[idx] = Spr2Pixel {
                        color,
                        depth: z,
                        alpha: 31,
                    };
                }
            }
        }
    }
}
