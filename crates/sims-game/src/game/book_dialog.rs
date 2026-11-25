//! The paginated spellbook and cookbook (`sims_ui::book`).

use super::*;

impl Game {
    // -------------------------------------------------------------------------
    // Paginated Spellbook & Cookbook Reference Modal (`sims_ui::book`)
    // -------------------------------------------------------------------------

    /// Whether the paginated Spellbook / Cookbook [`ReferenceBookDialog`] is open.
    pub fn book_open(&self) -> bool {
        self.book_dialog.is_open()
    }

    /// Active [`ReferenceBookDialog`] controller.
    pub fn book_dialog(&self) -> &ReferenceBookDialog {
        &self.book_dialog
    }

    /// Active [`ReferenceBookSnapshot`], when the reference book modal is open.
    pub fn book_snapshot(&self) -> Option<&ReferenceBookSnapshot> {
        self.book_snapshot.as_ref()
    }

    /// Navigates to the previous page of the open reference book.
    pub fn book_prev_page(&mut self) -> bool {
        let changed = self
            .book_snapshot
            .as_ref()
            .and_then(|snap| self.book_dialog.prev_page(snap))
            .is_some();
        if changed {
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        }
        changed
    }

    /// Navigates to the next page of the open reference book.
    pub fn book_next_page(&mut self) -> bool {
        let changed = self
            .book_snapshot
            .as_ref()
            .and_then(|snap| self.book_dialog.next_page(snap))
            .is_some();
        if changed {
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        }
        changed
    }

    /// Scrolls vertically within the active page of the open reference book
    /// (clamped to the measured wrapped line overflow for the current viewport).
    pub fn book_scroll_lines(&mut self, delta_lines: i32) -> bool {
        if !self.book_dialog.is_open() {
            return false;
        }
        self.ensure_book_assets();
        let size = [
            self.view.size[0].max(1) as u32,
            self.view.size[1].max(1) as u32,
        ];
        let art = self.book_art.as_ref().unwrap_or(&self.hud.art);
        self.book_snapshot.as_ref().is_some_and(|snap| {
            self.book_dialog
                .scroll_vertical(delta_lines, size, snap, art)
        })
    }

    /// Scrolls within a tall reference book page via mouse wheel, turning pages
    /// once at the top/bottom of the current page.
    pub fn book_wheel(&mut self, delta_y: f64) -> bool {
        let dir = if delta_y > 0.0 {
            -1
        } else if delta_y < 0.0 {
            1
        } else {
            return false;
        };
        if self.book_scroll_lines(dir * 3) {
            return true;
        }
        let changed = self
            .book_snapshot
            .as_ref()
            .and_then(|snap| self.book_dialog.scroll(dir, snap))
            .is_some();
        if changed {
            self.hud.play(crate::hud::CLICK);
            let sounds = self.hud.take_sounds();
            self.keep_sounds(sounds);
        }
        changed
    }

    /// Opens the paginated Spellbook (`is_cookbook == false`) or Cookbook
    /// (`is_cookbook == true`) reference modal directly for `caller`.
    pub fn open_reference_book(&mut self, caller: Option<u16>, is_cookbook: bool) {
        self.ensure_book_assets();
        let snapshot = self.build_reference_book_snapshot(self.lot.world(), caller, is_cookbook);
        self.book_snapshot = Some(snapshot);
        self.book_dialog.open();
        self.close_menu();
        self.hud.close_camera_strip();
    }

    /// Closes the open reference book modal and delivers
    /// [`SpecialPanelResponse::Dismissed`] to the VM when backing a special panel.
    pub fn book_close(&mut self) -> Outcome {
        let is_book_panel = self.active_special_panel.as_ref().is_some_and(|p| {
            matches!(
                p.request,
                SpecialPanelRequest::SpellBook { .. } | SpecialPanelRequest::CookBook { .. }
            )
        });
        if !self.book_dialog.is_open() && !is_book_panel {
            return Outcome::default();
        }
        self.book_dialog.close();
        if is_book_panel {
            if self.answer_special_panel(SpecialPanelResponse::Dismissed) {
                self.hud.play(crate::hud::CLICK);
            } else {
                self.hud.play(crate::hud::ERROR);
                if self.book_snapshot.is_some() {
                    self.book_dialog.open();
                } else {
                    self.active_special_panel = None;
                    self.sync_special_panel();
                }
            }
        } else {
            self.book_snapshot = None;
            self.hud.play(crate::hud::CLICK);
        }
        let sounds = self.hud.take_sounds();
        self.keep_sounds(sounds);
        Outcome::redraw()
    }

    pub(super) fn ensure_book_assets(&mut self) {
        if self.book_strings.is_none() {
            let strings = BookStrings::load(self.hud.install(), US_ENGLISH).unwrap_or_default();
            self.book_strings = Some(strings);
        }
        if self.book_art.is_none()
            && let Ok(ui) = UiGraphics::open(self.hud.install())
            && let Ok(art) = sims_ui::book::load_art(&ui, US_ENGLISH)
        {
            self.book_art = Some(art);
        }
    }

    pub(super) fn draw_book_dialog(&mut self) {
        if !self.book_dialog.is_open() {
            return;
        }
        self.ensure_book_assets();
        let Some(snapshot) = self.book_snapshot.as_ref() else {
            return;
        };
        let strings = self.book_strings.clone().unwrap_or_default();
        let art = self.book_art.as_ref().unwrap_or(&self.hud.art);
        let (w, h) = (
            u32::try_from(self.canvas.width).unwrap_or(0),
            u32::try_from(self.canvas.height).unwrap_or(0),
        );
        let Ok(mut frame) =
            sims_ui::frame::Frame::from_packed(w, h, std::mem::take(&mut self.canvas.color))
        else {
            return;
        };
        self.book_dialog.draw(&mut frame, snapshot, &strings, art);
        self.canvas.color = frame.into_packed();
    }
}
