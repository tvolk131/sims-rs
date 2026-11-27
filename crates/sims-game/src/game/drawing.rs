//! Rendering a frame and the damage presented.

use super::*;

impl Game {
    /// Draws the frame: the lot, its people, then the panel. When the
    /// frame shows a new object under the pointer (the object moved, or
    /// the view did), its pie menu is listed, as when the pointer moves
    /// onto it, and the frame drawn again if the listing changed the lot.
    pub fn render(&mut self) -> &Canvas {
        self.draw();
        if self.follow() {
            self.prev_steady_key = None;
            self.present_damage = None;
            self.draw();
        }
        &self.canvas
    }

    /// Clears `lot_cache` while preserving the last drawn lot depth in
    /// `self.lot_depth` so `Game::under` stays exact until the next `draw()`.
    pub(super) fn clear_lot_cache(&mut self) {
        if self.lot_depth_from_cache {
            if let Some(cached) = self.lot_cache.last_lot_depth() {
                self.lot_depth.clear();
                self.lot_depth.extend_from_slice(cached);
            }
            self.lot_depth_from_cache = false;
        }
        self.lot_cache.clear();
        self.prev_steady_key = None;
        self.present_damage = None;
    }

    /// Read-only view of the lot depth buffer for the frame last drawn
    /// (borrowing directly from [`LotCache::last_lot_depth`] on cached draws
    /// with zero per-frame copy, or falling back to `self.lot_depth`).
    pub fn lot_depth_slice(&self) -> &[u16] {
        if self.lot_depth_from_cache
            && let Some(cached) = self
                .lot_cache
                .last_lot_depth()
                .filter(|d| d.len() == self.canvas.width.saturating_mul(self.canvas.height))
        {
            return cached;
        }
        &self.lot_depth
    }

    /// Whether `Game::under` is currently borrowing lot depth directly from
    /// [`LotCache`] without a per-frame clone.
    pub fn lot_depth_from_cache(&self) -> bool {
        self.lot_depth_from_cache
    }

    /// Number of pixels currently stored in the sparse `people_ids` buffer.
    pub fn people_ids_stored_pixels(&self) -> usize {
        self.people_ids.stored_pixels()
    }

    /// Canvas-space dirty rects accumulated since the last successful window
    /// present, or `None` when a full blit/present is required.
    pub fn peek_present_damage(&self) -> Option<&[sims_render::canvas::Rect]> {
        self.present_damage.as_deref()
    }

    /// Marks the current canvas as successfully presented to the window
    /// backbuffer so subsequent steady-state frames can accumulate bounded
    /// partial damage relative to it.
    pub fn on_presented(&mut self) {
        self.present_damage = Some(Vec::new());
    }

    /// Forces the next present of this lot to perform a full blit (e.g. when a
    /// shell-level modal overlay was composited over the lot canvas).
    pub fn invalidate_present_damage(&mut self) {
        self.present_damage = None;
    }

    /// Draws the frame.
    pub(super) fn draw(&mut self) {
        self.draw_frame(false);
    }

    /// Draws the frame, or with `photo` only the house view (the lot, its
    /// people and what they hold) without menus, panel or dialogs: what
    /// Camera Mode photographs.
    pub(super) fn draw_frame(&mut self, photo: bool) {
        let [w, h] = self.view.size.map(|n| n.max(1) as usize);
        if (self.canvas.width, self.canvas.height) != (w, h) {
            self.canvas = Canvas::new(w, h, [0, 0, 0]);
        }
        let sell = self.buy_sell_info();
        let world = self.lot.world();
        // The cutaway also opens the room the local player's selected Sim is in.
        self.view.camera.focus.selected = if self.coop.is_active() {
            self.local_selected_sim()
                .and_then(|id| world.objects.get(&id))
                .map(|o| Selected {
                    tile: [o.position[0].div_euclid(16), o.position[1].div_euclid(16)],
                    level: o.position[2],
                    room: o.data.get(29).copied().unwrap_or(NO_ROOM),
                })
        } else {
            world.selected_whereabouts().map(|w| Selected {
                tile: [i32::from(w.tile[0]), i32::from(w.tile[1])],
                level: w.level,
                room: w.room.unwrap_or(NO_ROOM),
            })
        };
        let viewport = self.house_viewport_size();
        let render_options = sims_render::RenderOptions {
            shadows: self.hud.controls.options.graphics.shadows,
            lighting: self.hud.controls.options.graphics.lighting,
        };
        // The lot, then the people, then the objects that hold someone
        // (a bench a Sim sits on), as the original's frame does.
        let preview = self.buy_cursor.as_ref().map(|c| (c.cursor, c.held));
        let draw_shape = self.build_preview_shape.as_ref().unwrap_or(&self.lot_shape);
        self.lot_cache.draw_lot_with_preview_and_options(
            world,
            draw_shape,
            &mut self.assets,
            &self.view.camera,
            viewport,
            preview,
            render_options,
            &mut self.canvas,
        );
        let lot_hit = self.lot_cache.last_hit();
        self.drawn_camera = self.view.camera;
        if self
            .lot_cache
            .last_lot_depth()
            .is_some_and(|d| d.len() == self.canvas.width.saturating_mul(self.canvas.height))
        {
            self.lot_depth_from_cache = true;
        } else {
            self.lot_depth.clone_from(&self.canvas.depth);
            self.lot_depth_from_cache = false;
        }
        // The plumb bob spins by the game's clock in milliseconds (the
        // original's 0x6e54e0); here the time passed in, which runs on
        // while the lot is paused or a dialog waits (**unverified**
        // whether the original's does).
        self.drawn_us = self.clock_us;
        let milliseconds = (self.clock_us / 1000) as u32;
        self.people.set_clock(milliseconds);
        self.people_rects.clear();
        let mut held_by_people = std::collections::BTreeSet::new();
        // The lot's depth from before the people, which the censor's test
        // reads.
        let pixels = self.canvas.width.saturating_mul(self.canvas.height);
        let lot_depth: &[u16] = self
            .lot_cache
            .last_lot_depth()
            .filter(|d| self.lot_depth_from_cache && d.len() == pixels)
            .unwrap_or(&self.lot_depth);
        // The censor's test also sees the Sims' held objects: the
        // original's software renderer draws them into the depth it
        // reads, and the Sims' meshes (Direct3D) not. So while someone is
        // censored, each Sim's held objects are drawn again into a canvas
        // holding only the lot's depth.
        let censoring = world.objects.values().any(|o| {
            o.person
                .as_ref()
                .is_some_and(|p| p.data.get(30).is_some_and(|&w| w != 0))
        });
        let mut censor_canvas: Option<sims_render::Canvas> = None;
        if self.open_pack_mode {
            crate::open_pack::draw_open_pack_sims(
                world,
                draw_shape,
                &self.view.camera,
                viewport,
                &mut self.canvas,
                &mut self.people_rects,
            );
        } else {
            // What each Sim holds is drawn just before the Sim, at their
            // attach points (a carried plate in the right palm), as the
            // person's own draw does (0x459e45..0x45a027, then the Sim at
            // 0x45a131); depth-tested both ways.
            let camera = self.view.camera;
            let assets = &mut self.assets;
            self.people.draw_in_viewport_tracked_holding(
                world,
                &camera,
                viewport,
                &mut self.canvas,
                &mut self.people_rects,
                |person, attach, canvas| {
                    if censoring
                        && lot_depth.len() == pixels
                        && !sims_render::held_by(world, person).is_empty()
                    {
                        let scratch = censor_canvas.get_or_insert_with(|| {
                            let mut c =
                                sims_render::Canvas::new(canvas.width, canvas.height, [0; 3]);
                            c.depth.copy_from_slice(lot_depth);
                            c
                        });
                        sims_render::draw_held_by(
                            world,
                            draw_shape,
                            assets,
                            &camera,
                            render_options,
                            person,
                            attach,
                            &held_by_people,
                            scratch,
                        );
                    }
                    let drawn = sims_render::draw_held_by(
                        world,
                        draw_shape,
                        assets,
                        &camera,
                        render_options,
                        person,
                        attach,
                        &held_by_people,
                        canvas,
                    );
                    held_by_people.extend(drawn);
                },
            );
        }
        let bobs = self.people.bobs(world);
        for (&id, obj) in &world.objects {
            if (obj.person.is_some() || bobs.contains(&id))
                && let Some(rect) = self.canvas.drawn(id)
            {
                self.people_rects.push(rect);
            }
        }
        // Who the pick notes: the people as drawn, before a bench they sit
        // on draws over them, captured only within their tight drawn rects.
        self.people_ids.take_rects(&self.canvas, &self.people_rects);
        // The censor mosaic over the people whose censor word is set,
        // after the people and before the objects that hold someone, as
        // the original's frame does (0x4b3510: 0x4b017b, 0x4ac707, then
        // 0x4aa1d7). Open-pack Sims have no censor boxes.
        // Its depth test reads the lot's depth from before the people,
        // with the Sims' held objects.
        let mut censor_rects = Vec::new();
        if !self.open_pack_mode {
            let censor_depth = censor_canvas.as_ref().map_or(lot_depth, |c| &c.depth[..]);
            self.people.draw_censor(
                world,
                draw_shape,
                &self.view.camera,
                viewport,
                censor_depth,
                self.restore_generation,
                &mut self.canvas,
                &mut censor_rects,
            );
        }
        // Then the objects that hold someone (a bench a Sim sits on), and
        // what the people's draws left: what a Sim without attach points
        // holds stays at its own position.
        let attach = self.held_attach();
        sims_render::draw_held_except(
            world,
            draw_shape,
            &mut self.assets,
            &self.view.camera,
            render_options,
            &attach,
            &held_by_people,
            &mut self.canvas,
        );
        let mut cur_dynamic_rects = self.people_rects.clone();
        cur_dynamic_rects.extend_from_slice(&censor_rects);
        for id in sims_render::holds_person(world) {
            if let Some(rect) = self.canvas.drawn(id) {
                cur_dynamic_rects.push(rect);
            }
        }
        // Build Mode Part 2 previews (Terrain overlay, Water/Pool digging preview
        // with coping border, and isometric Roof preview). Never drawn during static
        // Live Mode frames when `roof_view` is false, keeping ratchet frames bit-exact.
        if self.hud.controls.mode == Mode::Build {
            if let Some(overlay) = self.terrain_hover_overlay {
                let _ = render_build::draw_terrain_preview(
                    &self.lot_shape,
                    Some(world),
                    &self.view.camera,
                    &overlay,
                    &mut self.canvas,
                );
            }
            if let Some(overlay) = self.water_hover_overlay {
                let _ = render_build::draw_water_preview(
                    &self.lot_shape,
                    Some(world),
                    &self.view.camera,
                    &overlay,
                    &mut self.canvas,
                );
            }
        }
        if self.roof_view
            || (self.hud.controls.mode == Mode::Build
                && self.hud.build_state.tool == Some(BuildTool::Roof))
        {
            self.draw_roof_preview();
        }
        if photo {
            // The next frame presents in full: this one never reaches the
            // window.
            self.prev_steady_key = None;
            self.present_damage = None;
            self.prev_dynamic_rects = cur_dynamic_rects;
            return;
        }
        if self.coop.is_active() {
            self.draw_coop_remote_indicators(viewport);
            self.draw_coop_buy_preview(viewport);
        }
        let selected_override = if self.coop.is_active() {
            let local_sel = self.local_selected_sim();
            (local_sel != self.lot.world().selected()).then_some(local_sel)
        } else {
            None
        };
        self.log_debug_note_if_new();
        self.hud.set_clock_us(self.clock_us);
        self.sync_special_panel();
        let status = self.debug.then(|| self.status()).flatten();
        let world = self.lot.world();
        // A dialog the lot waits on is in the lot as the tick left it when
        // it was shown. Read it before the pie menu so a newly opened dialog
        // suppresses the pie's dark backdrop and head on the frame it closes
        // the menu.
        let dialogs = self.lot.asking().map_or(world, |a| &a.world);
        self.hud.read_dialog(dialogs, &self.assets);
        // The pie menu shades the house viewport behind it (0x4a94ec →
        // 0x4ac8ff) and renders the Sim's 3D head tracking the pointer
        // (0x4a953e → 0x462290 → 0x462985) directly into the house viewport
        // before the 2D HUD draws, so oversized headwear is clipped only by
        // the house viewport (not the 201×201 face bitmap), the bottom control
        // strip hides anything below the viewport, and pie labels draw on top.
        if !self.hud.will_show_dialog(self.view.size) {
            self.hud.resize_menu(self.view.size);
            let pointer = self.hud.pointer();
            if let Some(menu) = &mut self.hud.menu {
                if let Some(backdrop) = menu.backdrop_at(milliseconds) {
                    crate::people::shade_pie_face_with_radius(
                        &mut self.canvas,
                        backdrop.center,
                        backdrop.radius,
                        viewport,
                    );
                }
                if let Some(center) = menu.main_center()
                    && !self.open_pack_mode
                {
                    let rendered = if let Some(pose) = menu.head_pose_at(pointer, milliseconds) {
                        self.people.head_in_viewport_pose(
                            world,
                            menu.person,
                            viewport,
                            center,
                            self.view.camera.zoom,
                            pose,
                        )
                    } else {
                        let look = menu.head_look(pointer);
                        self.people.head_in_viewport(
                            world,
                            menu.person,
                            viewport,
                            center,
                            self.view.camera.zoom,
                            look,
                        )
                    };
                    match rendered {
                        Ok((origin, head, depth)) => {
                            if head.width > 0 && head.height > 0 {
                                crate::people::composite_pie_head(
                                    &mut self.canvas,
                                    origin,
                                    &head,
                                    &depth,
                                    viewport,
                                );
                            }
                            menu.head_origin = origin;
                            menu.head = Some(head);
                            menu.head_depth = depth;
                        }
                        Err(e) => {
                            self.people
                                .note(vec![format!("the pie menu has no head: {e}")]);
                            menu.head_origin = [0, 0];
                            menu.head = None;
                            menu.head_depth.clear();
                        }
                    }
                }
            }
        }
        let was_ui_animating = self.ui_animating_prev;
        let blink_changed = self.hud.sync_clock_blink(world, self.clock_us);
        self.hud.sync_character_names(self.lot.character_names());
        self.blink_dirty = false;
        if self.hud.controls.mode == Mode::Build && self.hud.controls.expanded {
            self.hud.layout_build(
                self.view.size,
                world,
                &self.assets,
                self.lot.can_undo(),
                self.lot.can_redo(),
            );
            self.hud.ensure_build_swatches(&mut self.assets);
        }
        self.hud.draw_for_lot_at_with_selection(
            world,
            self.lot.residential(),
            self.lot.lot_zoning(),
            self.lot_visibility(),
            &self.assets,
            &self.view,
            &self.pace,
            status.as_deref(),
            sell.as_ref(),
            self.clock_us,
            selected_override,
            &mut self.canvas,
        );
        self.ui_animating_prev = self.hud.is_ui_animating(self.clock_us);
        draw_ui_effect_highlights(
            self.hud.items(),
            &self.highlighted_buttons,
            &self.highlighted_portraits,
            !self.highlighted_relationships.is_empty(),
            &mut self.canvas,
        );
        self.draw_build_cost_label();
        self.draw_shop_dialog();
        self.draw_book_dialog();
        self.draw_phonebook_dialog();
        self.draw_destination_picker();
        self.draw_coop_hud_and_lobby();
        // The help balloon closed by the selection's going.
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        let problems = self.hud.take_problems();
        self.people.note(problems);

        let steady_eligible = lot_hit == sims_render::CacheHit::Full
            && self.hud.controls.mode == Mode::Live
            && self.hud.controls.help.is_none()
            && !blink_changed
            && !was_ui_animating
            && !self.ui_animating_prev
            && !self.debug
            && !self.roof_view
            && self.terrain_hover_overlay.is_none()
            && self.water_hover_overlay.is_none()
            && self.build_preview_shape.is_none()
            && self.buy_cursor.is_none()
            && self.buy_selected.is_none()
            && !self.coop.is_active_or_starting()
            && !self.coop_lobby.is_open()
            && self.hud.menu.is_none()
            && self.hud.dialog().is_none()
            && !self.hud.has_ui_dialog()
            && self.lot.asking().is_none()
            && self.lot.asking_special_panel().is_none()
            && !self.hud.camera_strip_open()
            && !self.cheat_console_open()
            && !self.shop_dialog.is_open()
            && !self.book_dialog.is_open()
            && !self.phonebook_dialog.is_open()
            && self.destination_picker.is_none()
            && self.highlighted_buttons.is_empty()
            && self.highlighted_portraits.is_empty()
            && self.highlighted_relationships.is_empty()
            && sell.is_none();
        let cur_steady_key = if steady_eligible {
            let world = self.lot.world();
            Some(SteadyFrameKey {
                width: self.canvas.width,
                height: self.canvas.height,
                viewport,
                camera: self.view.camera,
                ticks: self.lot.ticks(),
                restore_generation: self.restore_generation,
                world_sound_rev: self.world_sound_rev,
                controls: self.hud.controls,
                selected: world.selected(),
                funds: world.budget.funds,
                paused: world.paused(),
                speed: world.speed(),
                return_speed: world.return_speed(),
                clock_nonzero: self.clock_us > 0,
                queue: self.hud.queue().clone(),
                hud_items: self.hud.items().to_vec(),
            })
        } else {
            None
        };
        if cur_steady_key.is_some() && self.prev_steady_key == cur_steady_key {
            if let Some(dmg) = &mut self.present_damage {
                dmg.extend_from_slice(&self.prev_dynamic_rects);
                dmg.extend_from_slice(&cur_dynamic_rects);
                if dmg.len() > 32 {
                    self.present_damage = None;
                }
            }
        } else {
            self.present_damage = None;
        }
        self.prev_dynamic_rects = cur_dynamic_rects;
        self.prev_steady_key = cur_steady_key;
    }

    /// The canvas last drawn.
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// Where each person the last [`Game::render`] drew draws what they
    /// hold: their attach points ([`sims_render::PersonAttach`]), as the
    /// held pass took them. Empty in open-pack mode, whose sprite Sims have
    /// no hands.
    pub fn held_attach(&self) -> BTreeMap<u16, sims_render::PersonAttach> {
        if self.open_pack_mode {
            return BTreeMap::new();
        }
        self.people.held_attach(&self.drawn_camera)
    }

    /// Passes the house view's screen-centre pick (`0x4a7b58` over the active
    /// house viewport [`Game::house_viewport_size`], at the original's origin
    /// column `camera.origin[0] + pick::ORIGIN_COLUMN`, cut to a tile via
    /// `0x433067`) into the simulation.
    pub(super) fn sync_view_centre(&mut self) {
        if self.coop.is_active() {
            return;
        }
        let (w, h) = self.house_viewport_size();
        let mut camera = self.view.camera;
        camera.origin[0] = camera.origin[0].saturating_add(pick::ORIGIN_COLUMN);
        let centre =
            sims_render::screen_centre_pick(&self.lot_shape, &camera, [w as i32, h as i32])
                .map(|[x, y]| [(x >> 4) as i8, (y >> 4) as i8]);
        self.lot.set_view_centre(centre);
    }

    /// Passes the house view's rotation and screen-centre pick into the
    /// simulation.
    pub(super) fn sync_view(&mut self) {
        if self.coop.is_active() {
            return;
        }
        self.lot.set_view_rotation(self.view.camera.rotation);
        self.sync_view_centre();
    }
}
