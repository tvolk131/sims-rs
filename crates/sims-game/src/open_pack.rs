//! Clean-room standalone `open-pack/` discovery, staging, procedural base
//! surfaces, and 2D Sim sprite/portrait presentation for zero-commercial-asset
//! `sims-game` play.
//!
//! All generated IFFs (`GameData/Behavior.iff`, `GameData/Global/Global.iff`,
//! `GameData/Objects/open_indicators.iff`, `GameData/floors.iff`,
//! `GameData/walls.iff`) and portraits are 100% original procedural data; no
//! EA/Maxis files are read or required.

use sims_antics::compile_standalone_pack_dir;
use sims_antics::encode::{
    encode_bhav, encode_iff_with_rsmp, encode_legacy_spr, encode_objd, encode_objf, encode_palt,
    encode_spr2, encode_string_table,
};
use sims_antics::house::StarterSimSpec;
use sims_antics::ir::{
    IFF_CONVERTED_FLAG, LegacySprChunk, LegacySprFrame, ObjdSpec, ObjfChunk, PaltChunk, Spr2Chunk,
    Spr2Frame, Spr2Pixel, StringEntry, StringFormat, StringTableChunk,
};
use sims_formats::bitmap::Image;
use sims_formats::execution::ObjectFunction;
use sims_formats::iff::ChunkData;
use sims_render::canvas::Image as CanvasImage;
use sims_render::depth::{Anchor, Depth};
use sims_render::{Camera, Canvas, Lot as RenderLot};
use sims_sim::World;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Temporary staging directory holding a compiled standalone `open-pack/`
/// install tree for the lifetime of a standalone [`crate::Game`], cleaned up
/// via [`std::fs::remove_dir_all`] on drop.
pub(crate) struct OpenPackStagingDir(PathBuf);

impl OpenPackStagingDir {
    fn create() -> Result<Self, String> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let base = std::env::temp_dir();
        for _ in 0..1024 {
            let id = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = base.join(format!("sims-rs-open-pack-{pid}-{id}"));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(format!(
                        "creating open-pack staging directory {}: {e}",
                        dir.display()
                    ));
                }
            }
        }
        Err("could not allocate a unique open-pack staging directory".into())
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for OpenPackStagingDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A compiled and staged standalone `open-pack/` install ready for [`crate::Game`].
pub(crate) struct StagedOpenPack {
    pub(crate) staging_dir: OpenPackStagingDir,
    pub(crate) house_path: PathBuf,
    pub(crate) save_slot: String,
    pub(crate) sounds_wav: BTreeMap<String, Vec<u8>>,
    pub(crate) starter_sims: Vec<StarterSimSpec>,
}

/// Computes a stable, pack- and lot-scoped save slot key derived from the
/// source `open-pack/` directory and manifest identity (never from the
/// temporary staging directory) so multiple standalone packs sharing one
/// `--saves` file never overwrite one another.
pub fn open_pack_save_slot(pack_dir: &Path, pack_name: &str, lot_number: i32) -> String {
    let canonical = std::fs::canonicalize(pack_dir).unwrap_or_else(|_| pack_dir.to_path_buf());
    format!(
        "open-pack:{}:{}:lot{lot_number}",
        canonical.to_string_lossy(),
        pack_name.trim()
    )
}

/// Returns `true` when `dir` looks like a valid `open-pack/` source directory.
pub fn is_open_pack_dir(dir: &Path) -> bool {
    dir.is_dir() && dir.join("pack.toml").is_file() && dir.join("starter-house.toml").is_file()
}

/// Discovers the default `open-pack/` directory from the current working
/// directory, the running executable's ancestors, or the workspace manifest
/// directory (for `cargo run` / `cargo test`).
pub fn discover_open_pack_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors().take(4) {
            candidates.push(ancestor.join("open-pack"));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().take(6) {
            candidates.push(ancestor.join("open-pack"));
        }
    }
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("open-pack");
    candidates.push(manifest_root);

    candidates.into_iter().find(|p| is_open_pack_dir(p))
}

/// Compiles `pack_dir` via `sims_antics::compile_standalone_pack_dir` and
/// stages a self-contained clean-room directory tree with zero commercial files.
pub(crate) fn stage_open_pack(pack_dir: &Path) -> Result<StagedOpenPack, String> {
    let compiled = compile_standalone_pack_dir(pack_dir)
        .map_err(|e| format!("compiling open-pack at {}: {e}", pack_dir.display()))?;

    let staging_dir = OpenPackStagingDir::create()?;
    let root = staging_dir.path();

    let game_data = root.join("GameData");
    let global_dir = game_data.join("Global");
    let objects_dir = game_data.join("Objects");
    let floors_dir = game_data.join("Floors");
    let walls_dir = game_data.join("Walls");
    let user_data = root.join("UserData");
    let houses_dir = user_data.join("Houses");
    let chars_dir = user_data.join("Characters");

    for dir in [
        &global_dir,
        &objects_dir,
        &floors_dir,
        &walls_dir,
        &houses_dir,
        &chars_dir,
    ] {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }

    let write_bytes = |path: PathBuf, bytes: &[u8]| -> Result<(), String> {
        std::fs::write(&path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))
    };

    // Minimal clean-room Behavior.iff and Global.iff (satisfy Install::detect,
    // Catalog, and VM motive tuning / mood curves with zero commercial tables).
    let minimal_iff = encode_iff_with_rsmp(&[]).map_err(|e| format!("empty IFF: {e}"))?;
    write_bytes(game_data.join("Behavior.iff"), &minimal_iff)?;
    let global_iff = build_clean_room_global_iff()?;
    write_bytes(global_dir.join("Global.iff"), &global_iff)?;

    // Clean-room objects FAR (fixed `.far` name distinct from `open_indicators.iff`
    // so arbitrary manifest `archive_name` values always mount cleanly) + hidden
    // lot indicator & Buy/Build cursor definitions.
    write_bytes(
        objects_dir.join("open_pack_objects.far"),
        &compiled.pack.far_bytes,
    )?;
    let indicators_iff = build_clean_room_indicators_iff()?;
    write_bytes(objects_dir.join("open_indicators.iff"), &indicators_iff)?;

    // Compiled clean-room `.flr` floors & `.wll` walls.
    for (name, bytes) in &compiled.floors {
        write_bytes(floors_dir.join(name), bytes)?;
    }
    for (name, bytes) in &compiled.walls {
        write_bytes(walls_dir.join(name), bytes)?;
    }

    // Procedural base `floors.iff` and `walls.iff` (wall shape masks + fallback patterns).
    let base_floors_iff = build_clean_room_base_floors_iff()?;
    write_bytes(game_data.join("floors.iff"), &base_floors_iff)?;
    let base_walls_iff = build_clean_room_base_walls_iff()?;
    write_bytes(game_data.join("walls.iff"), &base_walls_iff)?;

    // Compiled clean-room Neighborhood, LotZoning, starter House, and starter Sim characters.
    write_bytes(
        user_data.join("Neighborhood.iff"),
        &compiled.neighborhood_iff,
    )?;
    let zoning_iff = build_clean_room_lot_zoning_iff(compiled.house_manifest.lot_number)?;
    write_bytes(user_data.join("LotZoning.iff"), &zoning_iff)?;
    let house_path = houses_dir.join(&compiled.house_iff_name);
    write_bytes(house_path.clone(), &compiled.house_iff)?;
    for (name, bytes) in &compiled.characters {
        write_bytes(chars_dir.join(name), bytes)?;
    }

    let save_slot = open_pack_save_slot(
        pack_dir,
        &compiled.pack.name,
        compiled.house_manifest.lot_number,
    );

    Ok(StagedOpenPack {
        staging_dir,
        house_path,
        save_slot,
        sounds_wav: compiled.pack.sounds_wav,
        starter_sims: compiled.starter_sims,
    })
}

/// Builds a minimal clean-room `GameData/Global/Global.iff` containing empty
/// `FCNS` tuning tables (`1`, `2`, `4`, `5`, `6`) and linear clean-room mood
/// curves (`STR# 502`, `504`, `505`) so `sims-vm`'s tick loop runs cleanly
/// without any commercial global tables.
fn build_clean_room_global_iff() -> Result<Vec<u8>, String> {
    let mut chunks = Vec::new();
    // Valid FCNS v1 header with 0 entries (`Rates::from_lookup` and
    // `autonomy_constants` use clean defaults for each entry).
    let mut empty_fcns = Vec::with_capacity(16);
    empty_fcns.extend_from_slice(&0u32.to_le_bytes()); // reserved
    empty_fcns.extend_from_slice(&1u32.to_le_bytes()); // version 1
    empty_fcns.extend_from_slice(b"SNCF");
    empty_fcns.extend_from_slice(&0u32.to_le_bytes()); // count = 0
    for &(id, label) in &[
        (1u16, "Open Motive Rates (Adult)"),
        (2u16, "Open Autonomy Constants"),
        (4u16, "Open House Constants"),
        (5u16, "Open Motive Rates (Cat)"),
        (6u16, "Open Motive Rates (Dog)"),
    ] {
        chunks.push(ChunkData {
            tag: *b"FCNS",
            id,
            flags: IFF_CONVERTED_FLAG,
            label: label.to_owned(),
            data: empty_fcns.clone(),
        });
    }

    // 7 linear clean-room mood weight curves for adult (`502`) & child (`504`).
    let mood_entries: Vec<StringEntry> = (0..7)
        .map(|_| StringEntry::english("(-100;-100)(0;0)(100;100)"))
        .collect();
    for &(id, label) in &[
        (502u16, "Open Adult Mood Curves"),
        (504u16, "Open Child Mood Curves"),
    ] {
        let table = StringTableChunk::new(
            *b"STR#",
            id,
            label,
            StringFormat::Localized,
            mood_entries.clone(),
        );
        chunks.push(ChunkData {
            tag: *b"STR#",
            id,
            flags: IFF_CONVERTED_FLAG,
            label: table.label.clone(),
            data: encode_string_table(&table).map_err(|e| e.to_string())?,
        });
    }

    // Clean-room House rating curves (`STR# 505`: Size & Furnishings).
    let house_curves = StringTableChunk::new(
        *b"STR#",
        505,
        "Open House Curves",
        StringFormat::Localized,
        vec![
            StringEntry::english("(0;0)(100;50)(300;100)"),
            StringEntry::english("(0;0)(1;50)(3;100)"),
        ],
    );
    chunks.push(ChunkData {
        tag: *b"STR#",
        id: 505,
        flags: IFF_CONVERTED_FLAG,
        label: house_curves.label.clone(),
        data: encode_string_table(&house_curves).map_err(|e| e.to_string())?,
    });

    encode_iff_with_rsmp(&chunks).map_err(|e| e.to_string())
}

/// Builds a minimal clean-room `UserData/LotZoning.iff` marking `lot_number`
/// as residential (`1`) so household selection recognizes starter Sims as
/// residents of the lot.
fn build_clean_room_lot_zoning_iff(lot_number: i32) -> Result<Vec<u8>, String> {
    let table = StringTableChunk::new(
        *b"STR#",
        1,
        "Lot Zoning",
        StringFormat::Localized,
        vec![StringEntry::english(format!("{lot_number},residential"))],
    );
    let data = encode_string_table(&table).map_err(|e| e.to_string())?;
    encode_iff_with_rsmp(&[ChunkData {
        tag: *b"STR#",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: table.label,
        data,
    }])
    .map_err(|e| e.to_string())
}

/// Builds a minimal clean-room IFF defining the two hidden lot indicator GUIDs
/// (`0x2f72_6fff` and `0x0baa_71c5`) with trivial `return true` `Init` and a
/// quiet `while 0 == 0 { idle(300); }` `Main` loop so standard VM lot load
/// creates indicators cleanly and ticks indefinitely with 0 traps.
fn build_clean_room_indicators_iff() -> Result<Vec<u8>, String> {
    let compiled = sims_antics::compile_antics(
        r#"
tree 4096 "Init" {
    return true;
}
tree 4097 "Main" (args: 1) {
    while 0 == 0 {
        idle(300);
    }
}
"#,
    )
    .map_err(|e| format!("compiling clean-room indicators: {e}"))?;

    let mut chunks = Vec::new();
    for (&id, bhav) in &compiled.behaviors {
        chunks.push(ChunkData {
            tag: *b"BHAV",
            id,
            flags: IFF_CONVERTED_FLAG,
            label: bhav.label.clone(),
            data: encode_bhav(bhav).map_err(|e| e.to_string())?,
        });
    }

    for &(chunk_id, guid, object_type, name) in &[
        (128u16, 0x2f72_6fffu32, 4u16, "Open Selected Indicator"),
        (129u16, 0x0baa_71c5u32, 4u16, "Open Date Indicator"),
        (130u16, 0x0000_0437u32, 9u16, "Open Tile Cursor"),
        (131u16, 0x0000_0439u32, 9u16, "Open Wall Cursor"),
        (132u16, 0xf3aa_a62fu32, 4u16, "Open ComeAndSee Marker"),
    ] {
        let mut spec = ObjdSpec::new(chunk_id, name, guid);
        spec.object_type = object_type;
        spec.init_id = 4096;
        spec.main_id = 4097;
        spec.base_graphic_id = 0;
        spec.num_graphics = 0;
        spec.tree_table_id = 0;
        spec.slots_id = 0;
        spec.catalog_strings_id = 0;
        chunks.push(ChunkData {
            tag: *b"OBJD",
            id: chunk_id,
            flags: IFF_CONVERTED_FLAG,
            label: name.to_owned(),
            data: encode_objd(&spec.to_chunk()).map_err(|e| e.to_string())?,
        });
        let objf = ObjfChunk::new(
            chunk_id,
            format!("{name} Functions"),
            vec![
                ObjectFunction {
                    condition: 0,
                    action: 4096,
                },
                ObjectFunction {
                    condition: 0,
                    action: 4097,
                },
            ],
        );
        chunks.push(ChunkData {
            tag: *b"OBJf",
            id: chunk_id,
            flags: IFF_CONVERTED_FLAG,
            label: objf.label.clone(),
            data: encode_objf(&objf).map_err(|e| e.to_string())?,
        });
    }

    encode_iff_with_rsmp(&chunks).map_err(|e| e.to_string())
}

/// Builds a clean-room procedural `GameData/floors.iff` providing isometric
/// diamond tile sprites for built-in floor IDs `0` (grass fallback) and `1`
/// (neutral stone tile) across all 3 zooms (`id`, `id + 256`, `id + 512`).
fn build_clean_room_base_floors_iff() -> Result<Vec<u8>, String> {
    let palt = PaltChunk {
        id: 1,
        label: "Open Base Floors Palette".to_owned(),
        colors: vec![
            [0, 0, 0],
            [78, 128, 62],
            [92, 146, 74],
            [178, 174, 166],
            [156, 152, 144],
        ],
    };
    let mut chunks = vec![ChunkData {
        tag: *b"PALT",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: palt.label.clone(),
        data: encode_palt(&palt).map_err(|e| e.to_string())?,
    }];

    for (floor_id, primary, accent) in [(0u16, 1u8, 2u8), (1u16, 3u8, 4u8)] {
        for (zoom_idx, (w, h)) in [(32u16, 16u16), (64, 32), (128, 64)]
            .into_iter()
            .enumerate()
        {
            let width = usize::from(w);
            let height = usize::from(h);
            let half_w = width as i32 / 2;
            let half_h = height as i32 / 2;
            let mut pixels = vec![
                Spr2Pixel {
                    color: 0,
                    depth: 255,
                    alpha: 0,
                };
                width * height
            ];
            for y in 0..height {
                for x in 0..width {
                    let dx = (x as i32 * 2 + 1 - width as i32).abs();
                    let dy = (y as i32 * 2 + 1 - height as i32).abs();
                    if dx * half_h + dy * half_w <= 2 * half_w * half_h {
                        let color = if ((x + y) % 4) == 0 { accent } else { primary };
                        pixels[y * width + x] = Spr2Pixel {
                            color,
                            depth: 128,
                            alpha: 31,
                        };
                    }
                }
            }
            let spr_id = floor_id + (zoom_idx as u16) * 256;
            let spr2 = Spr2Chunk {
                id: spr_id,
                label: format!("Open Base Floor {floor_id} Z{zoom_idx}"),
                version: 1000,
                default_palette: 1,
                frames: vec![Spr2Frame {
                    width: w,
                    height: h,
                    flags: 7,
                    palette_id: 1,
                    transparent_index: 0,
                    x: 0,
                    y: 0,
                    pixels,
                }],
            };
            chunks.push(ChunkData {
                tag: *b"SPR2",
                id: spr_id,
                flags: IFF_CONVERTED_FLAG,
                label: spr2.label.clone(),
                data: encode_spr2(&spr2).map_err(|e| e.to_string())?,
            });
        }
    }

    encode_iff_with_rsmp(&chunks).map_err(|e| e.to_string())
}

/// Builds a clean-room procedural `GameData/walls.iff` supplying isometric wall
/// shape masks (`SPR# style*2 + cut + [0, 512, 1024]`) for standing and cutaway
/// walls across all 3 zooms plus fallback wall patterns (`SPR# id + [1536, 1792, 2048]`).
fn build_clean_room_base_walls_iff() -> Result<Vec<u8>, String> {
    let palt = PaltChunk {
        id: 1,
        label: "Open Base Walls Palette".to_owned(),
        colors: vec![
            [0, 0, 0],
            [255, 255, 255],
            [212, 204, 192],
            [184, 174, 160],
            [138, 128, 114],
        ],
    };
    let mut chunks = vec![ChunkData {
        tag: *b"PALT",
        id: 1,
        flags: IFF_CONVERTED_FLAG,
        label: palt.label.clone(),
        data: encode_palt(&palt).map_err(|e| e.to_string())?,
    }];

    // Shape masks: style 1 (standard wall) and style 2 (fence/half) and style 9 (diagonal cap),
    // both standing (`cut = false`) and cutaway (`cut = true`), across all 3 zooms.
    for style in [1u16, 2, 9] {
        for cut in [false, true] {
            for (zoom_idx, (w, h)) in [(16u16, 48u16), (32, 96), (64, 192)]
                .into_iter()
                .enumerate()
            {
                let width = usize::from(w);
                let height = usize::from(h);
                let slope_h = 8usize << zoom_idx; // half-tile isometric vertical span
                let wall_h = if cut {
                    6usize << zoom_idx
                } else {
                    height.saturating_sub(slope_h)
                };
                let mut frames = Vec::with_capacity(4);
                for frame_idx in 0..4usize {
                    let mut pixels = vec![None; width * height];
                    for x in 0..width {
                        let (y_bottom, y_top) = match frame_idx {
                            // Frame 0: west edge (slopes up-right -> down-left).
                            0 => {
                                let bot = height.saturating_sub(1 + (x / 2));
                                (bot, bot.saturating_sub(wall_h))
                            }
                            // Frame 1: north edge (slopes up-left -> down-right).
                            1 => {
                                let bot = height.saturating_sub(slope_h) + (x / 2);
                                (bot.min(height - 1), bot.saturating_sub(wall_h))
                            }
                            // Frames 2 & 3: diagonal edges.
                            _ => {
                                let bot = height.saturating_sub(1);
                                (bot, bot.saturating_sub(wall_h))
                            }
                        };
                        for y in y_top..=y_bottom.min(height - 1) {
                            pixels[y * width + x] = Some(1);
                        }
                    }
                    frames.push(LegacySprFrame {
                        width: w,
                        height: h,
                        pixels,
                    });
                }
                let spr_id = style * 2 + u16::from(cut) + [0u16, 512, 1024][zoom_idx];
                let spr = LegacySprChunk {
                    id: spr_id,
                    label: format!("Open Wall Shape S{style} C{} Z{zoom_idx}", u8::from(cut)),
                    version: 503,
                    default_palette: 1,
                    frames,
                };
                chunks.push(ChunkData {
                    tag: *b"SPR#",
                    id: spr_id,
                    flags: IFF_CONVERTED_FLAG,
                    label: spr.label.clone(),
                    data: encode_legacy_spr(&spr).map_err(|e| e.to_string())?,
                });
            }
        }
    }

    // Default fallback wall patterns (IDs 0 and 1, plus 252 cap strip) across all 3 zooms.
    for pat_id in [0u16, 1, 252] {
        for (zoom_idx, (w, h)) in [(16u16, 48u16), (32, 96), (64, 192)]
            .into_iter()
            .enumerate()
        {
            let width = usize::from(w);
            let height = usize::from(h);
            let mut frames = Vec::with_capacity(4);
            for side in 0..4usize {
                let base_color = if side % 2 == 0 { 2u8 } else { 3u8 };
                let mut pixels = vec![None; width * height];
                for y in 0..height {
                    let color = if y >= height.saturating_sub(3 << zoom_idx) {
                        4u8
                    } else {
                        base_color
                    };
                    for x in 0..width {
                        pixels[y * width + x] = Some(color);
                    }
                }
                frames.push(LegacySprFrame {
                    width: w,
                    height: h,
                    pixels,
                });
            }
            let spr_id = pat_id + [1536u16, 1792, 2048][zoom_idx];
            let spr = LegacySprChunk {
                id: spr_id,
                label: format!("Open Base Wall Pattern {pat_id} Z{zoom_idx}"),
                version: 503,
                default_palette: 1,
                frames,
            };
            chunks.push(ChunkData {
                tag: *b"SPR#",
                id: spr_id,
                flags: IFF_CONVERTED_FLAG,
                label: spr.label.clone(),
                data: encode_legacy_spr(&spr).map_err(|e| e.to_string())?,
            });
        }
    }

    encode_iff_with_rsmp(&chunks).map_err(|e| e.to_string())
}

/// Generates a clean-room procedural 35×41 portrait bitmap for a starter Sim
/// so the live HUD household strip renders distinct, recognizable Sim portraits
/// without any commercial BMP assets.
pub(crate) fn procedural_sim_portrait(sim_index: usize) -> Image {
    let w = 35usize;
    let h = 41usize;
    let (bg, shirt, hair, skin) = match sim_index % 4 {
        0 => (
            [38u8, 66, 108],
            [62u8, 134, 196],
            [78u8, 52, 34],
            [232u8, 192, 162],
        ),
        1 => (
            [42u8, 78, 86],
            [186u8, 88, 112],
            [44u8, 36, 48],
            [218u8, 174, 144],
        ),
        2 => (
            [54u8, 66, 98],
            [72u8, 158, 102],
            [148u8, 96, 48],
            [238u8, 202, 174],
        ),
        _ => (
            [64u8, 56, 94],
            [176u8, 136, 58],
            [62u8, 44, 32],
            [192u8, 146, 114],
        ),
    };
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let dx = x as i32 - 17;
            let dy_head = y as i32 - 15;
            let in_head = dx * dx * 10 + dy_head * dy_head * 8 <= 680;
            let in_hair = in_head && (y < 12 || (y < 17 && dx.abs() > 5));
            let in_shoulders = y >= 25 && dx.abs() <= (11 + (y as i32 - 25) / 2).min(15);
            let is_eye = y == 15 && (dx == -3 || dx == 3);
            let is_smile = y == 20 && (-2..=2).contains(&dx);
            let rgb = if is_eye {
                [26, 30, 38]
            } else if is_smile {
                [168, 86, 86]
            } else if in_hair {
                hair
            } else if in_head {
                skin
            } else if in_shoulders {
                shirt
            } else {
                bg
            };
            let p = (y * w + x) * 4;
            rgba[p] = rgb[0];
            rgba[p + 1] = rgb[1];
            rgba[p + 2] = rgb[2];
            rgba[p + 3] = 255;
        }
    }
    Image {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

/// Projects an `open-pack/` Sim's fine world position `[x, y, level]` to screen
/// pixels and its matching [`Depth`] anchor using terrain steps (`altitude_at`)
/// converted to near-zoom pixels (`steps * 16`) for [`Camera::project_fine`]
/// and passed directly in terrain steps to [`Depth::anchor`].
pub fn project_open_pack_sim(
    camera: &Camera,
    lot_shape: &RenderLot,
    pos: [i32; 3],
) -> ([i32; 2], Anchor) {
    let [x, y, level] = pos;
    let altitude = lot_shape.altitude_at([x, y]);
    let elevation_near_px = (altitude * 16.0).round() as i32;
    let rotated = camera.rotate_fine([x, y]);
    let screen = camera.project_fine(rotated, level, elevation_near_px);
    let anchor = Depth::new(camera).anchor(screen[1], altitude, level);
    (screen, anchor)
}

/// Draws clean-room directional 2D procedural Sims (and a crisp green diamond
/// plumb bob above the selected Sim) into `canvas` for standalone `open-pack/`
/// lots where no commercial 3D `.skn`/`.cmx` character meshes exist.
///
/// Writes `canvas.color`, `canvas.depth`, and object picking IDs so Sims are
/// visibly rendered at all rotations and zooms, depth-occluded by nearer
/// walls/furniture, and clickable/selectable via `Game::click`.
pub(crate) fn draw_open_pack_sims(
    world: &World,
    lot_shape: &RenderLot,
    camera: &Camera,
    viewport: (u32, u32),
    canvas: &mut Canvas,
    written_rects: &mut Vec<sims_render::canvas::Rect>,
) {
    let vp_w = i32::try_from(viewport.0)
        .unwrap_or(i32::MAX)
        .min(canvas.width as i32);
    let vp_h = i32::try_from(viewport.1)
        .unwrap_or(i32::MAX)
        .min(canvas.height as i32);
    if vp_w <= 0 || vp_h <= 0 {
        return;
    }

    let zoom_idx = camera.zoom.index();
    let active_level = camera.level;
    let selected_id = world.selected();

    struct VisibleOpenSim {
        id: u16,
        guid: u32,
        rel_dir: u8,
        screen: [i32; 2],
        anchor: Anchor,
    }

    // Collect visible Sims sorted back-to-front by screen Y so nearer Sims draw cleanly over farther ones.
    let mut sims: Vec<VisibleOpenSim> = Vec::new();
    for (&id, obj) in &world.objects {
        if obj.person.is_none() {
            continue;
        }
        let [x, y, level] = obj.position;
        if level != active_level || x < 0 || y < 0 {
            continue;
        }
        let (screen, anchor) = project_open_pack_sim(camera, lot_shape, [x, y, level]);
        let facing = obj.data.get(1).copied().unwrap_or(0);
        // Combine world Sim facing (0..8) with camera rotation (0..4) so the
        // character faces consistently relative to the active camera angle.
        let rel_dir =
            ((facing.rem_euclid(8) / 2) + i16::from(camera.rotation & 3)).rem_euclid(4) as u8;
        sims.push(VisibleOpenSim {
            id,
            guid: obj.guid,
            rel_dir,
            screen,
            anchor,
        });
    }
    sims.sort_by_key(|s| s.screen[1]);

    for VisibleOpenSim {
        id,
        guid,
        rel_dir,
        screen,
        anchor,
    } in sims
    {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;

        canvas.set_id(Some(id));
        draw_procedural_sim_figure(
            guid,
            rel_dir,
            screen,
            zoom_idx,
            vp_w,
            vp_h,
            anchor.depth,
            canvas,
            &mut min_x,
            &mut min_y,
            &mut max_x,
            &mut max_y,
        );
        canvas.set_id(None);

        // Draw green diamond plumb bob above the selected Sim.
        if selected_id == Some(id) {
            let bob_y = (if min_y < i32::MAX {
                min_y - (8 << zoom_idx).min(24)
            } else {
                screen[1] - (42 << zoom_idx).min(96)
            })
            .max(12);
            draw_clean_room_plumb_bob(
                screen[0], bob_y, zoom_idx, vp_w, vp_h, canvas, &mut min_x, &mut min_y, &mut max_x,
                &mut max_y,
            );
        }

        if let Some(drawn_rect) = canvas.drawn(id) {
            min_x = min_x.min(drawn_rect[0]);
            min_y = min_y.min(drawn_rect[1]);
            max_x = max_x.max(drawn_rect[2]);
            max_y = max_y.max(drawn_rect[3]);
        }
        if min_x < max_x && min_y < max_y {
            written_rects.push([min_x.max(0), min_y.max(0), max_x.min(vp_w), max_y.min(vp_h)]);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_procedural_sim_figure(
    guid: u32,
    rel_dir: u8,
    screen: [i32; 2],
    zoom_idx: usize,
    vp_w: i32,
    vp_h: i32,
    depth: i32,
    canvas: &mut Canvas,
    min_x: &mut i32,
    min_y: &mut i32,
    max_x: &mut i32,
    max_y: &mut i32,
) {
    let scale = 1i32 << zoom_idx;
    let half_w = 6 * scale;
    let total_w = (half_w * 2 + 1) as usize;
    let total_h = (28 * scale) as usize;
    let (shirt, shirt_hi, shirt_lo, hair, skin, pants) = if guid & 1 == 1 {
        (
            [62u8, 134, 204, 255],
            [92u8, 166, 232, 255],
            [40u8, 92, 150, 255],
            [86u8, 56, 36, 255],
            [236u8, 198, 168, 255],
            [48u8, 66, 98, 255],
        )
    } else {
        (
            [196u8, 86, 114, 255],
            [224u8, 120, 144, 255],
            [142u8, 54, 76, 255],
            [42u8, 34, 48, 255],
            [218u8, 174, 142, 255],
            [56u8, 58, 76, 255],
        )
    };
    let shoe = [38u8, 34, 34, 255];
    let eye = [30u8, 32, 42, 255];
    let shadow = [24u8, 28, 34, 140];

    let mut rgba = vec![0u8; total_w * total_h * 4];
    let cx = half_w;
    let head_r = 3 * scale;
    let head_cy = 4 * scale;
    let torso_top = head_cy + head_r + scale;
    let torso_bot = torso_top + 9 * scale;
    let legs_bot = torso_bot + 8 * scale;
    let feet_bot = (total_h as i32) - scale;

    for y in 0..total_h as i32 {
        for x in 0..total_w as i32 {
            let dx = x - cx;
            let dy_h = y - head_cy;
            let mut px: Option<[u8; 4]> = None;

            // Ground shadow oval under feet.
            let dy_s = y - feet_bot;
            if dy_s.abs() <= scale && dx.abs() <= 5 * scale {
                px = Some(shadow);
            }

            // Two legs + shoes.
            if y >= torso_bot && y <= feet_bot {
                let leg_gap = (scale / 2).max(1);
                let in_left = dx >= -3 * scale && dx <= -leg_gap;
                let in_right = dx >= leg_gap && dx <= 3 * scale;
                if in_left || in_right {
                    px = Some(if y >= legs_bot { shoe } else { pants });
                }
            }

            // Shaded torso + arms.
            if y >= torso_top && y < torso_bot {
                let torso_half = 4 * scale;
                let arm_half = 5 * scale;
                if dx.abs() <= torso_half {
                    px = Some(if dx < -scale {
                        shirt_hi
                    } else if dx > scale {
                        shirt_lo
                    } else {
                        shirt
                    });
                } else if dx.abs() <= arm_half && y >= torso_top + scale && y < torso_bot - scale {
                    px = Some(if y >= torso_bot - 3 * scale {
                        skin
                    } else if dx < 0 {
                        shirt_hi
                    } else {
                        shirt_lo
                    });
                }
            }

            // Neck.
            if y >= head_cy + head_r && y < torso_top && dx.abs() <= scale {
                px = Some(skin);
            }

            // Head, hair crown, and camera-relative directional face/eyes.
            if dx * dx + dy_h * dy_h <= head_r * head_r {
                let back_view = rel_dir == 2;
                let hair_cap = back_view || dy_h <= -scale;
                let mut c = if hair_cap { hair } else { skin };
                if !back_view && dy_h >= 0 && dy_h <= scale {
                    let eye_shift = match rel_dir {
                        1 => -scale,
                        3 => scale,
                        _ => 0,
                    };
                    let ex = dx - eye_shift;
                    if (ex == -scale || ex == scale) && dy_h == 0 {
                        c = eye;
                    }
                }
                px = Some(c);
            }

            if let Some(c) = px {
                let p = (y as usize * total_w + x as usize) * 4;
                rgba[p..p + 4].copy_from_slice(&c);
            }
        }
    }

    let ox = screen[0] - half_w;
    let oy = screen[1] - (total_h as i32) + scale;
    if ox >= vp_w || oy >= vp_h || ox + total_w as i32 <= 0 || oy + total_h as i32 <= 0 {
        return;
    }
    let clipped_h = (vp_h - oy).min(total_h as i32);
    if clipped_h <= 0 {
        return;
    }
    let draw_h = clipped_h as usize;
    let img = CanvasImage {
        width: total_w,
        height: draw_h,
        rgba: if draw_h < total_h {
            rgba[..total_w * draw_h * 4].to_vec()
        } else {
            rgba
        },
        depth: None,
    };
    canvas.draw(&img, [ox, oy], false, |_, _, _| depth);
    *min_x = (*min_x).min(ox);
    *min_y = (*min_y).min(oy);
    *max_x = (*max_x).max(ox + total_w as i32);
    *max_y = (*max_y).max(oy + clipped_h);
}

#[allow(clippy::too_many_arguments)]
fn draw_clean_room_plumb_bob(
    cx: i32,
    cy: i32,
    zoom_idx: usize,
    vp_w: i32,
    vp_h: i32,
    canvas: &mut Canvas,
    min_x: &mut i32,
    min_y: &mut i32,
    max_x: &mut i32,
    max_y: &mut i32,
) {
    let rx = (3 << zoom_idx).min(10);
    let ry = (6 << zoom_idx).min(18);
    for dy in -ry..=ry {
        let y = cy + dy;
        if y < 0 || y >= vp_h {
            continue;
        }
        let span = rx * (ry - dy.abs()) / ry.max(1);
        for dx in -span..=span {
            let x = cx + dx;
            if x < 0 || x >= vp_w {
                continue;
            }
            let rgb = if dx < 0 {
                [96, 244, 126]
            } else {
                [42, 186, 74]
            };
            canvas.plot_color(x, y, rgb);
            *min_x = (*min_x).min(x);
            *min_y = (*min_y).min(y);
            *max_x = (*max_x).max(x + 1);
            *max_y = (*max_y).max(y + 1);
        }
    }
}
