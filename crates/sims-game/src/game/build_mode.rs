//! Build mode: tools, roofs, terrain, water, walls and floors.

use super::*;

impl Game {
    /// The Build Mode panel state (active tool, page, selected item).
    pub fn build_state(&self) -> sims_ui::build::BuildState {
        self.hud.build_state
    }

    /// The Build Mode panel layout as last built.
    pub fn build_layout(&self) -> &sims_ui::build::BuildLayout {
        self.hud.build_layout()
    }

    /// Rebuilds the cached Build Mode panel layout for the current viewport and
    /// Undo/Redo availability.
    pub(super) fn layout_build(&mut self) {
        let can_undo = self.lot.can_undo();
        let can_redo = self.lot.can_redo();
        self.hud.layout_build(
            self.view.size,
            self.lot.world(),
            &self.assets,
            can_undo,
            can_redo,
        );
    }

    /// Selects one of the 12 Build Mode tools (`cWinArch`, `0x501197`),
    /// managing `"Wall Cursor"` (`0x00000439`) activation/deactivation
    /// (`0x48b68b`, `0x48a66a`, `0x48a6e5`).
    pub fn select_build_tool(&mut self, tool: BuildTool) {
        self.cancel_buy();
        self.build_drag = None;
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        let catalog = self.hud.ensure_build_catalog(&self.assets).clone();
        self.hud.build_state.select_tool(tool, &catalog);
        if self.coop.is_active() {
            // Never open or close the VM `"Wall Cursor"` object out-of-band in
            // co-op mode; Build previews remain purely local until lockstep commit.
            if matches!(
                tool,
                BuildTool::Terrain | BuildTool::Water | BuildTool::Roof
            ) {
                self.note = Some(
                    "Terrain, Water/Pool, and Roof tools are not yet supported in co-op mode."
                        .into(),
                );
            } else if tool == BuildTool::Roof {
                self.sync_build_roof_ui();
            }
            self.layout_build();
            return;
        }
        match tool {
            BuildTool::Wall => {
                let mode = self.current_wall_drag_mode().cursor_mode();
                self.open_wall_cursor(mode);
            }
            BuildTool::Wallpaper => {
                self.open_wall_cursor(2);
            }
            BuildTool::Roof => {
                self.close_wall_cursor();
                self.sync_build_roof_ui();
            }
            _ => {
                self.close_wall_cursor();
            }
        }
        // 1.91 opens every object catalog tool with its first item picked up
        // (oracle `build-doors/windows/stairs/fireplace/plants.png` all show
        // slot 0 in the selected state). Skip it when the item is not
        // affordable so a tool click never plays the error sound.
        if matches!(
            tool,
            BuildTool::Doors
                | BuildTool::Windows
                | BuildTool::Stairs
                | BuildTool::Fireplace
                | BuildTool::Plants
        ) && self.hud.build_state.selected.is_none()
            && let Some(first) = catalog.items_for(tool).first()
            && (first.price <= 0 || first.price <= sims_vm::available_funds(self.lot.world()))
        {
            self.select_build_item(first.id);
        }
        self.layout_build();
        self.refresh_world();
    }

    /// Applies a Build Mode Part 2 subtool button or roof pattern/pitch selection
    /// (`Terrain`, `Water`/`Pool`, `RoofPitch`, `RoofPattern`, or narrow-viewport `ToolPage`).
    pub fn apply_build_subtool(&mut self, action: BuildSubtoolAction) -> bool {
        if self.hud.controls.mode != Mode::Build {
            return false;
        }
        let catalog = self.hud.ensure_build_catalog(&self.assets).clone();
        if self.coop.is_active()
            && matches!(
                action,
                BuildSubtoolAction::RoofPitch(_)
                    | BuildSubtoolAction::RoofPattern(_)
                    | BuildSubtoolAction::Terrain(_)
                    | BuildSubtoolAction::Water(_)
            )
        {
            self.note = Some(
                "Terrain, Water/Pool, and Roof tools are not yet supported in co-op mode.".into(),
            );
            self.hud.play(crate::hud::ERROR);
            self.layout_build();
            return false;
        }
        let state_changed = self.hud.build_state.apply_subtool(action, &catalog);
        match action {
            BuildSubtoolAction::Terrain(_) | BuildSubtoolAction::Water(_) => {
                self.cancel_buy();
                self.build_drag = None;
                self.build_preview_shape = None;
                self.terrain_hover_overlay = None;
                self.water_hover_overlay = None;
                self.close_wall_cursor();
                self.hud.play(crate::hud::CLICK);
            }
            BuildSubtoolAction::RoofPitch(preset) => {
                self.cancel_buy();
                self.build_drag = None;
                self.build_preview_shape = None;
                self.terrain_hover_overlay = None;
                self.water_hover_overlay = None;
                self.close_wall_cursor();
                let plan = sims_vm::plan_roof_pitch_with_base(
                    self.lot.world(),
                    &self.base_roof,
                    vm_roof_pitch(preset),
                );
                if plan.valid {
                    match self.lot.build_commit_roof(&plan) {
                        Ok(true) => {
                            self.dirty = true;
                            self.hud.play(crate::hud::CLICK);
                        }
                        Ok(false) => {}
                        Err(e) => {
                            self.note = Some(format!("Roof pitch change failed: {e}"));
                            self.hud.play(crate::hud::ERROR);
                        }
                    }
                } else {
                    self.hud.play(crate::hud::CLICK);
                }
                self.sync_build_roof_ui();
            }
            BuildSubtoolAction::RoofPattern(idx) => {
                self.cancel_buy();
                self.build_drag = None;
                self.build_preview_shape = None;
                self.terrain_hover_overlay = None;
                self.water_hover_overlay = None;
                self.close_wall_cursor();
                if let Some(pat) = catalog.roof_patterns().get(idx) {
                    let plan = sims_vm::plan_roof_pattern_with_base(
                        self.lot.world(),
                        &self.base_roof,
                        &pat.stem,
                    );
                    if plan.valid {
                        match self.lot.build_commit_roof(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                self.hud.play(crate::hud::CLICK);
                            }
                            Ok(false) => {}
                            Err(e) => {
                                self.note = Some(format!("Roof pattern change failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else {
                        self.hud.play(crate::hud::CLICK);
                    }
                }
                self.sync_build_roof_ui();
            }
            BuildSubtoolAction::ToolPage(_) => {
                self.hud.play(crate::hud::CLICK);
            }
        }
        self.layout_build();
        state_changed
    }

    /// Synchronizes `hud.build_state`'s Roof pitch & pattern index from the lot's
    /// live roof state (or `HOUS 0` baseline) so the UI and Undo/Redo always match.
    pub(super) fn sync_build_roof_ui(&mut self) {
        let state = self.lot.roof_state_with_base(&self.base_roof);
        self.hud.build_state.roof_pitch = ui_roof_pitch_preset(state.pitch);
        let catalog = self.hud.ensure_build_catalog(&self.assets);
        let target = state.pattern.trim().to_ascii_lowercase();
        let target_stem = target.strip_suffix(".bmp").unwrap_or(target.as_str());
        if let Some(idx) = catalog.roof_patterns().iter().position(|p| {
            let stem = p.stem.trim().to_ascii_lowercase();
            let s = stem.strip_suffix(".bmp").unwrap_or(stem.as_str());
            s == target_stem
        }) {
            self.hud.build_state.roof_pattern = idx;
        }
    }

    pub(super) fn draw_roof_preview(&mut self) {
        if self.roof_catalog.is_none() {
            if let Ok(cat) = RoofCatalog::load(self.hud.install()) {
                self.roof_catalog = Some(cat);
            } else {
                return;
            }
        }
        let Some(catalog) = self.roof_catalog.as_ref() else {
            return;
        };
        let state = self.lot.roof_state_with_base(&self.base_roof);
        let config = RoofPreviewConfig {
            pattern_name: state.pattern,
            style: RoofStyle::Hip,
            pitch: render_roof_pitch(state.pitch),
            custom_slope: None,
            level: None,
            footprint: None,
        };
        let _ = render_build::draw_roof_preview_from_catalog(
            &self.lot_shape,
            &self.view.camera,
            catalog,
            &config,
            &mut self.canvas,
        );
    }

    pub(super) fn current_terrain_mode(&self) -> TerrainMode {
        match self.hud.build_state.terrain_subtool {
            TerrainSubtool::Raise => {
                if self.ctrl_held {
                    TerrainMode::Lower
                } else {
                    TerrainMode::Raise
                }
            }
            TerrainSubtool::Lower => {
                if self.ctrl_held {
                    TerrainMode::Raise
                } else {
                    TerrainMode::Lower
                }
            }
            TerrainSubtool::Level => {
                if self.shift_held {
                    TerrainMode::LevelAverage
                } else {
                    TerrainMode::Level
                }
            }
            TerrainSubtool::Grass => {
                if self.ctrl_held {
                    TerrainMode::RemoveGrass
                } else {
                    TerrainMode::RestoreGrass
                }
            }
        }
    }

    pub(super) fn current_water_mode(&self) -> WaterMode {
        if self.ctrl_held {
            WaterMode::Remove
        } else {
            WaterMode::Paint
        }
    }

    pub(super) fn current_pool_mode(&self) -> PoolMode {
        if self.ctrl_held {
            PoolMode::Demolish
        } else {
            PoolMode::Dig
        }
    }

    /// Steps the Build Mode catalog grid page by `delta`.
    pub fn page_build(&mut self, delta: i32) -> bool {
        if self.hud.controls.mode != Mode::Build || !self.hud.controls.expanded {
            return false;
        }
        let changed = self.hud.build_state.step_page(delta);
        self.layout_build();
        changed
    }

    /// Selects a Build Mode catalog entry (wall/fence style, floor, wallpaper,
    /// or door/window/column/stair/fireplace/plant object).
    pub fn select_build_item(&mut self, item: BuildItemId) {
        self.cancel_buy();
        self.build_drag = None;
        self.build_preview_shape = None;
        self.hud.build_state.select_item(item);
        match item {
            BuildItemId::WallStyle(_) => {
                let mode = self.current_wall_drag_mode().cursor_mode();
                self.open_wall_cursor(mode);
            }
            BuildItemId::Wallpaper(_) => {
                self.open_wall_cursor(2);
            }
            BuildItemId::Floor(_) => {
                self.close_wall_cursor();
            }
            BuildItemId::Object(guid) => {
                self.close_wall_cursor();
                let price = self
                    .hud
                    .ensure_build_catalog(&self.assets)
                    .get(item)
                    .map(|e| e.price);
                let funds = sims_vm::available_funds(self.lot.world());
                match price {
                    Some(p) if p <= 0 || p <= funds => {
                        self.buy_selected = Some(guid);
                    }
                    Some(_) => {
                        self.hud.play(crate::hud::ERROR);
                        self.buy_selected = None;
                        self.hud.build_state.selected = None;
                    }
                    None => {
                        self.buy_selected = None;
                    }
                }
            }
        }
        self.layout_build();
        self.refresh_world();
    }

    pub(super) fn open_wall_cursor(&mut self, mode: i16) {
        if self.coop.is_active_or_starting() {
            return;
        }
        if let Some(mut cur) = self.wall_cursor.take() {
            if cur.mode != mode {
                let pos = cur.position;
                let _ = self.lot.build_cursor_move(&mut cur, pos, None, Some(mode));
                self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
            }
            self.wall_cursor = Some(cur);
            return;
        }
        match self.lot.build_cursor_open(mode) {
            Ok(cursor) => {
                self.wall_cursor = Some(cursor);
                self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
            }
            Err(e) => self.note = Some(format!("Could not open Wall Cursor: {e}")),
        }
    }

    pub(super) fn close_wall_cursor(&mut self) {
        if self.coop.is_active() {
            self.wall_cursor = None;
            return;
        }
        if let Some(cursor) = self.wall_cursor.take() {
            if let Err(e) = self.lot.build_cursor_close(&cursor) {
                self.note = Some(format!("Could not close Wall Cursor: {e}"));
            }
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        } else {
            let _ = self.lot.build_cursor_close_active();
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
    }

    pub(super) fn current_wall_drag_mode(&self) -> WallDragMode {
        match (self.shift_held, self.ctrl_held) {
            (false, false) => WallDragMode::Build,
            (false, true) => WallDragMode::Demolish,
            (true, false) => WallDragMode::RoomBox,
            (true, true) => WallDragMode::RoomBoxDemolish,
        }
    }

    pub(super) fn current_floor_drag_mode(&self) -> FloorDragMode {
        match (self.shift_held, self.ctrl_held) {
            (false, false) => FloorDragMode::Lay,
            (false, true) => FloorDragMode::Remove,
            (true, false) => FloorDragMode::RoomFillLay,
            (true, true) => FloorDragMode::RoomFillRemove,
        }
    }

    pub(super) fn current_wallpaper_drag_mode(&self) -> WallpaperDragMode {
        match (self.shift_held, self.ctrl_held) {
            (false, false) => WallpaperDragMode::Apply,
            (false, true) => WallpaperDragMode::Strip,
            (true, false) => WallpaperDragMode::RoomFillApply,
            (true, true) => WallpaperDragMode::RoomFillStrip,
        }
    }

    pub(super) fn unproject_build_fine(&self, point: [i32; 2]) -> [i32; 2] {
        let at = [point[0] - pick::ORIGIN_COLUMN, point[1]];
        sims_render::unproject_terrain(&self.lot_shape, &self.view.camera, at)
            .unwrap_or_else(|| self.view.camera.unproject_fine(at))
    }

    /// Computes which diagonal half (`0` or `1`) of tile `(tx, ty)` the fine
    /// coordinates `(fx, fy)` fall into for a split diagonal floor tile.
    pub(super) fn split_half_at(&self, fine: [i32; 2], tx: i32, ty: i32, level: u8) -> Option<u8> {
        let level_idx = usize::from(level.saturating_sub(1)).min(1);
        let size = self.lot_shape.size;
        if tx < 0 || ty < 0 || tx as usize >= size || ty as usize >= size {
            return None;
        }
        let segs = self.lot_shape.levels[level_idx].walls
            [sims_render::Lot::cell(tx as usize, ty as usize)]
        .segments;
        let lx = fine[0] - tx * 16;
        let ly = fine[1] - ty * 16;
        if (segs & WALL_DESCENDING) != 0 {
            Some(u8::from(lx + ly >= 16))
        } else if (segs & WALL_ASCENDING) != 0 {
            Some(u8::from(lx > ly))
        } else {
            None
        }
    }

    /// Screen-space front-facing wall quad hit-test for the Wallpaper tool
    /// (`cNewWallpaperTool::HitTest` `0x48a709`), accurate across all 3 zooms
    /// and 4 camera rotations.
    pub(super) fn pick_wall_face(&self, point: [i32; 2]) -> Option<WallFace> {
        let level = u8::try_from(self.view.camera.level)
            .unwrap_or(1)
            .clamp(1, 2);
        let level_idx = usize::from(level - 1);
        let active = self.lot_shape.size;
        if active < 2 {
            return None;
        }
        let wall_h = match (self.view.camera.walls, self.view.camera.zoom) {
            (Walls::Down, sims_render::Zoom::Far) => 6,
            (Walls::Down, sims_render::Zoom::Medium) => 12,
            (Walls::Down, sims_render::Zoom::Near) => 23,
            (_, sims_render::Zoom::Far) => 29,
            (_, sims_render::Zoom::Medium) => 58,
            (_, sims_render::Zoom::Near) => 116,
        };
        let walls = &self.lot_shape.levels[level_idx].walls;
        let lvl = i32::from(level);
        // Gather candidate tiles around the ground unprojections at heights
        // `0..=wall_h` so clicks anywhere on a standing wall are included.
        let mut candidates = BTreeSet::new();
        for dy in [0, wall_h / 3, (wall_h * 2) / 3, wall_h] {
            let fine = self.unproject_build_fine([point[0], point[1] + dy]);
            let cx = fine[0] >> 4;
            let cy = fine[1] >> 4;
            for ox in -2..=2 {
                for oy in -2..=2 {
                    let tx = cx + ox;
                    let ty = cy + oy;
                    if tx >= 1 && ty >= 1 && (tx as usize) < active && (ty as usize) < active {
                        candidates.insert((tx as usize, ty as usize));
                    }
                }
            }
        }

        let px = point[0] as f32;
        let py = point[1] as f32;
        let mut best: Option<(f32, WallFace)> = None;

        let mut test_quad = |c0: [i32; 2],
                             c1: [i32; 2],
                             normal: [i32; 2],
                             face: WallFace,
                             _seg_tile: [i32; 2],
                             _seg_bit: u8| {
            let quad_h = wall_h;
            let mid = [(c0[0] + c1[0]) / 2, (c0[1] + c1[1]) / 2];
            let p_mid = lot_point(&self.view.camera, &self.lot_shape, [mid[0], mid[1], lvl]);
            let p_out = lot_point(
                &self.view.camera,
                &self.lot_shape,
                [mid[0] + normal[0], mid[1] + normal[1], lvl],
            );
            // Front-facing iff outward normal projects downward toward camera (`delta_y > 0`).
            if p_out[1] <= p_mid[1] {
                return;
            }
            let p0 = lot_point(&self.view.camera, &self.lot_shape, [c0[0], c0[1], lvl]);
            let p1 = lot_point(&self.view.camera, &self.lot_shape, [c1[0], c1[1], lvl]);
            let x0 = (p0[0] + pick::ORIGIN_COLUMN) as f32;
            let y0 = p0[1] as f32;
            let x1 = (p1[0] + pick::ORIGIN_COLUMN) as f32;
            let y1 = p1[1] as f32;
            let min_x = x0.min(x1) - 2.0;
            let max_x = x0.max(x1) + 2.0;
            if px < min_x || px > max_x {
                return;
            }
            let span = (x1 - x0).abs();
            let t = if span < 1.0 {
                0.5
            } else {
                ((px - x0) / (x1 - x0)).clamp(0.0, 1.0)
            };
            let y_bot = y0 + t * (y1 - y0);
            if py >= y_bot - quad_h as f32 - 2.0 && py <= y_bot + 3.0 {
                let score = y_bot;
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, face));
                }
            }
        };

        for (x, y) in candidates {
            let segs = walls[sims_render::Lot::cell(x, y)].segments;
            let fx = (x as i32) * 16;
            let fy = (y as i32) * 16;
            let seg_tile = [x as i32, y as i32];
            if (segs & WALL_WEST) != 0 {
                test_quad(
                    [fx, fy],
                    [fx, fy + 16],
                    [4, 0],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_WEST,
                        side: 0,
                    },
                    seg_tile,
                    WALL_WEST,
                );
                if x > 0 {
                    test_quad(
                        [fx, fy],
                        [fx, fy + 16],
                        [-4, 0],
                        WallFace {
                            tile: [(x - 1) as u8, y as u8],
                            bit: WALL_EAST,
                            side: 0,
                        },
                        seg_tile,
                        WALL_WEST,
                    );
                }
            }
            if (segs & WALL_NORTH) != 0 {
                test_quad(
                    [fx, fy],
                    [fx + 16, fy],
                    [0, 4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_NORTH,
                        side: 0,
                    },
                    seg_tile,
                    WALL_NORTH,
                );
                if y > 0 {
                    test_quad(
                        [fx, fy],
                        [fx + 16, fy],
                        [0, -4],
                        WallFace {
                            tile: [x as u8, (y - 1) as u8],
                            bit: WALL_SOUTH,
                            side: 0,
                        },
                        seg_tile,
                        WALL_NORTH,
                    );
                }
            }
            if (segs & WALL_DESCENDING) != 0 {
                test_quad(
                    [fx + 16, fy],
                    [fx, fy + 16],
                    [-4, -4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_DESCENDING,
                        side: 0,
                    },
                    seg_tile,
                    WALL_DESCENDING,
                );
                test_quad(
                    [fx + 16, fy],
                    [fx, fy + 16],
                    [4, 4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_DESCENDING,
                        side: 1,
                    },
                    seg_tile,
                    WALL_DESCENDING,
                );
            }
            if (segs & WALL_ASCENDING) != 0 {
                test_quad(
                    [fx, fy],
                    [fx + 16, fy + 16],
                    [-4, 4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_ASCENDING,
                        side: 0,
                    },
                    seg_tile,
                    WALL_ASCENDING,
                );
                test_quad(
                    [fx, fy],
                    [fx + 16, fy + 16],
                    [4, -4],
                    WallFace {
                        tile: [x as u8, y as u8],
                        bit: WALL_ASCENDING,
                        side: 1,
                    },
                    seg_tile,
                    WALL_ASCENDING,
                );
            }
        }
        best.map(|(_, f)| f)
    }

    pub(super) fn hover_build(&mut self, point: [i32; 2]) -> bool {
        if self.stopped.is_some() {
            return false;
        }
        let level = u8::try_from(self.view.camera.level)
            .unwrap_or(1)
            .clamp(1, 2);
        let active = self.lot_shape.size;
        let tool = self.hud.build_state.tool;

        // 1. Update active drag if one is in progress.
        if let Some(mut drag) = self.build_drag.take() {
            let prices = self.hud.build_prices(&self.assets);
            let tool_prices = self.hud.tool_prices(&self.assets);
            match &mut drag {
                BuildDrag::Wall {
                    start_corner,
                    end_corner,
                    style,
                    mode,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.water_hover_overlay = None;
                    let fine = self.unproject_build_fine(point);
                    let corner = sims_vm::snap_wall_corner(fine, active);
                    *end_corner = corner;
                    *mode = self.current_wall_drag_mode();
                    if !self.coop.is_active()
                        && let Some(mut cur) = self.wall_cursor.take()
                    {
                        let cfine = sims_vm::wall_cursor_fine(corner, level);
                        let _ = self.lot.build_cursor_move(
                            &mut cur,
                            cfine,
                            Some(0),
                            Some(mode.cursor_mode()),
                        );
                        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                        self.wall_cursor = Some(cur);
                    }
                    let computed = sims_vm::plan_wall_drag(
                        self.lot.world(),
                        *start_corner,
                        *end_corner,
                        level,
                        *style,
                        *mode,
                        &prices,
                    );
                    if computed.valid {
                        let mut preview_world = self.lot.world().clone();
                        sims_vm::preview_wall_on_world(&mut preview_world, &computed);
                        self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                            .ok()
                            .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                    } else {
                        self.build_preview_shape = None;
                    }
                    *plan = Some(computed);
                }
                BuildDrag::Floor {
                    start_tile,
                    end_tile,
                    floor_id,
                    mode,
                    split_half,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.water_hover_overlay = None;
                    let fine = self.unproject_build_fine(point);
                    let tile = [fine[0] >> 4, fine[1] >> 4];
                    *end_tile = tile;
                    *mode = self.current_floor_drag_mode();
                    *split_half = self.split_half_at(fine, tile[0], tile[1], level);
                    let computed = sims_vm::plan_floor_edit(
                        self.lot.world(),
                        *start_tile,
                        *end_tile,
                        level,
                        *floor_id,
                        *mode,
                        &prices,
                        *split_half,
                    );
                    if computed.valid {
                        let mut preview_world = self.lot.world().clone();
                        sims_vm::preview_floor_on_world(&mut preview_world, &computed);
                        self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                            .ok()
                            .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                    } else {
                        self.build_preview_shape = None;
                    }
                    *plan = Some(computed);
                }
                BuildDrag::Wallpaper {
                    faces,
                    pattern_id,
                    mode,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.water_hover_overlay = None;
                    *mode = self.current_wallpaper_drag_mode();
                    if let Some(face) = self.pick_wall_face(point) {
                        if !self.coop.is_active()
                            && let Some(mut cur) = self.wall_cursor.take()
                        {
                            let cfine = sims_vm::wallpaper_cursor_fine(face, level);
                            let facing =
                                sims_vm::wallpaper_cursor_facing(face, self.view.camera.rotation);
                            let _ = self.lot.build_cursor_move(&mut cur, cfine, facing, Some(2));
                            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                            self.wall_cursor = Some(cur);
                        }
                        if !faces.contains(&face) {
                            faces.push(face);
                        }
                    }
                    let computed = sims_vm::plan_wallpaper_edit(
                        self.lot.world(),
                        faces,
                        level,
                        *pattern_id,
                        *mode,
                        &prices,
                    );
                    if computed.valid {
                        let mut preview_world = self.lot.world().clone();
                        sims_vm::preview_wallpaper_on_world(&mut preview_world, &computed);
                        self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                            .ok()
                            .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                    } else {
                        self.build_preview_shape = None;
                    }
                    *plan = Some(computed);
                }
                BuildDrag::Terrain {
                    start,
                    end,
                    mode,
                    plan,
                } => {
                    self.water_hover_overlay = None;
                    self.build_preview_shape = None;
                    *mode = self.current_terrain_mode();
                    let fine = self.unproject_build_fine(point);
                    *end = terrain_coord_for_mode(fine, active, *mode);
                    let computed = sims_vm::plan_terrain_edit(
                        self.lot.world(),
                        *start,
                        *end,
                        *mode,
                        &tool_prices,
                    );
                    self.terrain_hover_overlay =
                        Some(terrain_overlay_for_drag(*start, *end, *mode));
                    *plan = Some(computed);
                }
                BuildDrag::Water {
                    start_tile,
                    end_tile,
                    mode,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.build_preview_shape = None;
                    *mode = self.current_water_mode();
                    let fine = self.unproject_build_fine(point);
                    let tile = [fine[0] >> 4, fine[1] >> 4];
                    *end_tile = tile;
                    let computed = sims_vm::plan_water_edit(
                        self.lot.world(),
                        *start_tile,
                        *end_tile,
                        *mode,
                        &tool_prices,
                    );
                    self.water_hover_overlay = Some(WaterOverlay {
                        tool: RenderWaterTool::Pond,
                        action: if *mode == WaterMode::Remove {
                            WaterAction::Remove
                        } else {
                            WaterAction::Dig
                        },
                        start: *start_tile,
                        end: *end_tile,
                    });
                    *plan = Some(computed);
                }
                BuildDrag::Pool {
                    start_tile,
                    end_tile,
                    mode,
                    plan,
                } => {
                    self.terrain_hover_overlay = None;
                    self.build_preview_shape = None;
                    *mode = self.current_pool_mode();
                    let fine = self.unproject_build_fine(point);
                    let tile = [fine[0] >> 4, fine[1] >> 4];
                    *end_tile = tile;
                    let computed = sims_vm::plan_pool_edit(
                        self.lot.world(),
                        *start_tile,
                        *end_tile,
                        *mode,
                        &tool_prices,
                    );
                    self.water_hover_overlay = Some(WaterOverlay {
                        tool: RenderWaterTool::Pool,
                        action: if *mode == PoolMode::Demolish {
                            WaterAction::Remove
                        } else {
                            WaterAction::Dig
                        },
                        start: *start_tile,
                        end: *end_tile,
                    });
                    *plan = Some(computed);
                }
            }
            self.build_drag = Some(drag);
            return true;
        }

        // 2. Non-dragging hover: move `"Wall Cursor"`, Terrain/Water overlays, and Shift room-fills.
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        match tool {
            Some(BuildTool::Wall) => {
                let fine = self.unproject_build_fine(point);
                let corner = sims_vm::snap_wall_corner(fine, active);
                let mode = self.current_wall_drag_mode().cursor_mode();
                if !self.coop.is_active()
                    && let Some(mut cur) = self.wall_cursor.take()
                {
                    let cfine = sims_vm::wall_cursor_fine(corner, level);
                    let _ = self
                        .lot
                        .build_cursor_move(&mut cur, cfine, Some(0), Some(mode));
                    self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                    self.wall_cursor = Some(cur);
                }
                true
            }
            Some(BuildTool::Wallpaper) => {
                if let Some(face) = self.pick_wall_face(point) {
                    if !self.coop.is_active()
                        && let Some(mut cur) = self.wall_cursor.take()
                    {
                        let cfine = sims_vm::wallpaper_cursor_fine(face, level);
                        let facing =
                            sims_vm::wallpaper_cursor_facing(face, self.view.camera.rotation);
                        let _ = self.lot.build_cursor_move(&mut cur, cfine, facing, Some(2));
                        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                        self.wall_cursor = Some(cur);
                    }
                    if self.shift_held {
                        let pat = self.hud.build_state.last_wallpaper.unwrap_or(1);
                        let prices = self.hud.build_prices(&self.assets);
                        let mode = self.current_wallpaper_drag_mode();
                        let plan = sims_vm::plan_wallpaper_edit(
                            self.lot.world(),
                            &[face],
                            level,
                            pat,
                            mode,
                            &prices,
                        );
                        if plan.valid {
                            let mut preview_world = self.lot.world().clone();
                            sims_vm::preview_wallpaper_on_world(&mut preview_world, &plan);
                            self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                                .ok()
                                .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                        }
                    }
                    true
                } else {
                    self.hide_build_cursor()
                }
            }
            Some(BuildTool::Floor) if self.shift_held => {
                let fine = self.unproject_build_fine(point);
                let tile = [fine[0] >> 4, fine[1] >> 4];
                let split_half = self.split_half_at(fine, tile[0], tile[1], level);
                let fid = self.hud.build_state.last_floor.unwrap_or(1);
                let prices = self.hud.build_prices(&self.assets);
                let mode = self.current_floor_drag_mode();
                let plan = sims_vm::plan_floor_edit(
                    self.lot.world(),
                    tile,
                    tile,
                    level,
                    fid,
                    mode,
                    &prices,
                    split_half,
                );
                if plan.valid {
                    let mut preview_world = self.lot.world().clone();
                    sims_vm::preview_floor_on_world(&mut preview_world, &plan);
                    self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                        .ok()
                        .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                    return true;
                }
                false
            }
            Some(BuildTool::Terrain) => {
                let mode = self.current_terrain_mode();
                let fine = self.unproject_build_fine(point);
                let coord = terrain_coord_for_mode(fine, active, mode);
                self.terrain_hover_overlay = Some(terrain_overlay_for_drag(coord, coord, mode));
                true
            }
            Some(BuildTool::Water) => {
                let fine = self.unproject_build_fine(point);
                let tile = [fine[0] >> 4, fine[1] >> 4];
                let (wtool, action) = match self.hud.build_state.water_subtool {
                    WaterSubtool::Pool => (
                        RenderWaterTool::Pool,
                        if self.ctrl_held {
                            WaterAction::Remove
                        } else {
                            WaterAction::Dig
                        },
                    ),
                    WaterSubtool::Water => (
                        RenderWaterTool::Pond,
                        if self.ctrl_held {
                            WaterAction::Remove
                        } else {
                            WaterAction::Dig
                        },
                    ),
                };
                self.water_hover_overlay = Some(WaterOverlay {
                    tool: wtool,
                    action,
                    start: tile,
                    end: tile,
                });
                true
            }
            _ => false,
        }
    }

    pub(super) fn hide_build_cursor(&mut self) -> bool {
        let had_overlay = self.build_preview_shape.take().is_some()
            || self.terrain_hover_overlay.take().is_some()
            || self.water_hover_overlay.take().is_some();
        if self.coop.is_active() {
            self.wall_cursor = None;
            return had_overlay;
        }
        let Some(mut cur) = self.wall_cursor.take() else {
            return had_overlay;
        };
        let was_on_lot = cur.position[0] >= 0;
        if was_on_lot {
            let _ = self.lot.build_cursor_hide(&mut cur);
            self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        }
        self.wall_cursor = Some(cur);
        was_on_lot || had_overlay
    }

    pub(super) fn press_build(&mut self, point: [i32; 2]) -> Outcome {
        let level = u8::try_from(self.view.camera.level)
            .unwrap_or(1)
            .clamp(1, 2);
        let active = self.lot_shape.size;
        self.pointer = Some(point);
        match self.hud.build_state.tool {
            Some(BuildTool::Wall) => {
                let fine = self.unproject_build_fine(point);
                let corner = sims_vm::snap_wall_corner(fine, active);
                let style = WallStyle::from_style_id(self.hud.build_state.last_wall_style)
                    .unwrap_or(WallStyle::Standard);
                let mode = self.current_wall_drag_mode();
                if !self.coop.is_active()
                    && let Some(mut cur) = self.wall_cursor.take()
                {
                    let cfine = sims_vm::wall_cursor_fine(corner, level);
                    let _ = self.lot.build_cursor_move(
                        &mut cur,
                        cfine,
                        Some(0),
                        Some(mode.cursor_mode()),
                    );
                    self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                    self.wall_cursor = Some(cur);
                }
                self.build_drag = Some(BuildDrag::Wall {
                    start_corner: corner,
                    end_corner: corner,
                    style,
                    mode,
                    plan: None,
                });
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            Some(BuildTool::Floor) => {
                let fine = self.unproject_build_fine(point);
                let tile = [fine[0] >> 4, fine[1] >> 4];
                let split_half = self.split_half_at(fine, tile[0], tile[1], level);
                let floor_id = self
                    .hud
                    .build_state
                    .last_floor
                    .or_else(|| self.hud.ensure_build_catalog(&self.assets).default_floor())
                    .unwrap_or(1);
                let mode = self.current_floor_drag_mode();
                let prices = self.hud.build_prices(&self.assets);
                let plan = sims_vm::plan_floor_edit(
                    self.lot.world(),
                    tile,
                    tile,
                    level,
                    floor_id,
                    mode,
                    &prices,
                    split_half,
                );
                if plan.valid {
                    let mut preview_world = self.lot.world().clone();
                    sims_vm::preview_floor_on_world(&mut preview_world, &plan);
                    self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                        .ok()
                        .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                }
                self.build_drag = Some(BuildDrag::Floor {
                    start_tile: tile,
                    end_tile: tile,
                    floor_id,
                    mode,
                    split_half,
                    plan: Some(plan),
                });
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            Some(BuildTool::Wallpaper) => {
                let Some(face) = self.pick_wall_face(point) else {
                    self.hud.play(crate::hud::ERROR);
                    let sounds = self.hud.take_sounds();
                    self.keep_sounds(sounds);
                    return Outcome::redraw();
                };
                if !self.coop.is_active()
                    && let Some(mut cur) = self.wall_cursor.take()
                {
                    let cfine = sims_vm::wallpaper_cursor_fine(face, level);
                    let facing = sims_vm::wallpaper_cursor_facing(face, self.view.camera.rotation);
                    let _ = self.lot.build_cursor_move(&mut cur, cfine, facing, Some(2));
                    self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
                    self.wall_cursor = Some(cur);
                }
                let pattern_id = self
                    .hud
                    .build_state
                    .last_wallpaper
                    .or_else(|| {
                        self.hud
                            .ensure_build_catalog(&self.assets)
                            .default_wallpaper()
                    })
                    .unwrap_or(1);
                let mode = self.current_wallpaper_drag_mode();
                let prices = self.hud.build_prices(&self.assets);
                let plan = sims_vm::plan_wallpaper_edit(
                    self.lot.world(),
                    &[face],
                    level,
                    pattern_id,
                    mode,
                    &prices,
                );
                if plan.valid {
                    let mut preview_world = self.lot.world().clone();
                    sims_vm::preview_wallpaper_on_world(&mut preview_world, &plan);
                    self.build_preview_shape = sims_render::Lot::from_world(&preview_world)
                        .ok()
                        .map(|s| s.with_floor_rotation(self.lot.floor_rotation()));
                }
                self.build_drag = Some(BuildDrag::Wallpaper {
                    faces: vec![face],
                    pattern_id,
                    mode,
                    plan: Some(plan),
                });
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            Some(BuildTool::Terrain) => {
                let mode = self.current_terrain_mode();
                let fine = self.unproject_build_fine(point);
                let coord = terrain_coord_for_mode(fine, active, mode);
                let tool_prices = self.hud.tool_prices(&self.assets);
                let plan =
                    sims_vm::plan_terrain_edit(self.lot.world(), coord, coord, mode, &tool_prices);
                self.terrain_hover_overlay = Some(terrain_overlay_for_drag(coord, coord, mode));
                self.build_drag = Some(BuildDrag::Terrain {
                    start: coord,
                    end: coord,
                    mode,
                    plan: Some(plan),
                });
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            Some(BuildTool::Water) => {
                let fine = self.unproject_build_fine(point);
                let tile = [fine[0] >> 4, fine[1] >> 4];
                let tool_prices = self.hud.tool_prices(&self.assets);
                match self.hud.build_state.water_subtool {
                    WaterSubtool::Pool => {
                        let mode = self.current_pool_mode();
                        let plan = sims_vm::plan_pool_edit(
                            self.lot.world(),
                            tile,
                            tile,
                            mode,
                            &tool_prices,
                        );
                        self.water_hover_overlay = Some(WaterOverlay {
                            tool: RenderWaterTool::Pool,
                            action: if mode == PoolMode::Demolish {
                                WaterAction::Remove
                            } else {
                                WaterAction::Dig
                            },
                            start: tile,
                            end: tile,
                        });
                        self.build_drag = Some(BuildDrag::Pool {
                            start_tile: tile,
                            end_tile: tile,
                            mode,
                            plan: Some(plan),
                        });
                    }
                    WaterSubtool::Water => {
                        let mode = self.current_water_mode();
                        let plan = sims_vm::plan_water_edit(
                            self.lot.world(),
                            tile,
                            tile,
                            mode,
                            &tool_prices,
                        );
                        self.water_hover_overlay = Some(WaterOverlay {
                            tool: RenderWaterTool::Pond,
                            action: if mode == WaterMode::Remove {
                                WaterAction::Remove
                            } else {
                                WaterAction::Dig
                            },
                            start: tile,
                            end: tile,
                        });
                        self.build_drag = Some(BuildDrag::Water {
                            start_tile: tile,
                            end_tile: tile,
                            mode,
                            plan: Some(plan),
                        });
                    }
                }
                self.hud.play(crate::hud::BLD_DRAG_DOWN);
                let sounds = self.hud.take_sounds();
                self.keep_sounds(sounds);
                Outcome::redraw()
            }
            _ => Outcome::default(),
        }
    }

    pub(super) fn release_build(&mut self, point: [i32; 2]) -> Outcome {
        if self.hud.covers(point) {
            self.build_drag = None;
            self.build_preview_shape = None;
            self.terrain_hover_overlay = None;
            self.water_hover_overlay = None;
            self.hide_build_cursor();
            return Outcome::redraw();
        }
        self.pointer = Some(point);
        self.hover_build(point);
        let Some(drag) = self.build_drag.take() else {
            return Outcome::default();
        };
        self.build_preview_shape = None;
        self.terrain_hover_overlay = None;
        self.water_hover_overlay = None;
        if self.coop.is_active() {
            return self.commit_build_coop(drag);
        }
        match drag {
            BuildDrag::Wall {
                start_corner,
                end_corner,
                mode,
                plan,
                ..
            } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_wall(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode.is_demolish() {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Wall edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if start_corner != end_corner {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Floor { mode, plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_floor(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode.is_remove() {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Floor edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Wallpaper { mode, plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_wallpaper(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode.is_strip() {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Wallpaper edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Terrain { plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_terrain(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                self.hud.play(crate::hud::BLD_DRAG_UP);
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Terrain edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Water { mode, plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_water(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode == WaterMode::Remove {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Water edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
            BuildDrag::Pool { mode, plan, .. } => {
                if let Some(plan) = plan {
                    if plan.valid && plan.affordable {
                        match self.lot.build_commit_pool(&plan) {
                            Ok(true) => {
                                self.dirty = true;
                                if mode == PoolMode::Demolish {
                                    self.hud.play(if plan.net_cost < 0 {
                                        crate::hud::MONEYBACK
                                    } else {
                                        crate::hud::ARCHTOOL_DELETE
                                    });
                                } else {
                                    self.hud.play(crate::hud::BLD_DRAG_UP);
                                }
                            }
                            Ok(false) => self.hud.play(crate::hud::ERROR),
                            Err(e) => {
                                self.note = Some(format!("Pool edit failed: {e}"));
                                self.hud.play(crate::hud::ERROR);
                            }
                        }
                    } else if !plan.affordable {
                        self.hud.play(crate::hud::ERROR);
                    }
                }
            }
        }
        self.refresh_world();
        self.layout_build();
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    /// Draws the floating Architecture cost/refund tag next to the cursor while
    /// dragging a wall, floor, wallpaper, terrain, water, or pool edit.
    pub(super) fn draw_build_cost_label(&mut self) {
        let Some([px, py]) = self.pointer else {
            return;
        };
        let (cost, affordable) = match &self.build_drag {
            Some(BuildDrag::Wall { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            Some(BuildDrag::Floor { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            Some(BuildDrag::Wallpaper { plan: Some(p), .. }) if p.valid => {
                (p.net_cost, p.affordable)
            }
            Some(BuildDrag::Terrain { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            Some(BuildDrag::Water { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            Some(BuildDrag::Pool { plan: Some(p), .. }) if p.valid => (p.net_cost, p.affordable),
            _ => return,
        };
        if cost == 0 {
            return;
        }
        let Some(font) = self.hud.art.font(10) else {
            return;
        };
        let label = if cost > 0 {
            format!("${cost}")
        } else {
            format!("+${}", -cost)
        };
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        let text_w = font.width(&label).max(20);
        let box_w = text_w + 10;
        let box_h = font.line_height + 6;
        let bx = (px + 14).clamp(4, (w as i32 - box_w - 4).max(4));
        let by = (py - box_h - 6).clamp(4, (h as i32 - box_h - 4).max(4));
        let bg = if affordable {
            [20, 32, 92]
        } else {
            [110, 24, 24]
        };
        let fg = if cost < 0 {
            [152, 240, 168]
        } else if affordable {
            [248, 240, 180]
        } else {
            [255, 160, 160]
        };
        frame.fill(sims_ui::frame::Rect::new(bx, by, box_w, box_h), bg);
        font.draw(&mut frame, &label, [bx + 5, by + 3], fg);
        self.canvas.color = frame.into_packed();
    }

    /// Refreshes the cached render lot shape and people from `self.lot.world()`.
    pub(super) fn refresh_world(&mut self) {
        self.world_sound_rev = self.world_sound_rev.wrapping_add(1);
        let world = self.lot.world();
        if !self.open_pack_mode {
            self.people.refresh(world);
        }
        match sims_render::Lot::from_world(world) {
            Ok(shape) => self.lot_shape = shape.with_floor_rotation(self.lot.floor_rotation()),
            Err(e) => self
                .people
                .note(vec![format!("the lot is not redrawn: {e}")]),
        }
    }
}
