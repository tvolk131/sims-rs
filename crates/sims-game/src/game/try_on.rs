//! The live 3D Sim preview for wardrobe and charm try-on (`sims_people::caf_preview`).

use super::*;

impl Game {
    // -------------------------------------------------------------------------
    // Live 3D Sim Preview for Wardrobe / Charm Try-On (`sims_people::caf_preview`)
    // -------------------------------------------------------------------------

    pub(super) fn ensure_shop_preview_assets(&mut self) -> Option<&sims_people::Assets> {
        if self.shop_preview_assets.is_none() {
            self.shop_preview_assets = sims_people::Assets::scan(&self.hud.install().root).ok();
        }
        self.shop_preview_assets.as_ref()
    }

    pub(super) fn build_boutique_preview_targets(
        &mut self,
        kind: ShopKind,
        items: &[ShopItem],
    ) -> BTreeMap<String, ShopPreviewTarget> {
        let mut targets = BTreeMap::new();
        let category_prefix: Option<&str> = match kind {
            ShopKind::DowntownBoutique(WardrobeCategory::Everyday) => Some("b"),
            ShopKind::DowntownBoutique(WardrobeCategory::Swimwear) => Some("s"),
            ShopKind::DowntownBoutique(WardrobeCategory::Formal) => Some("f"),
            ShopKind::DowntownBoutique(WardrobeCategory::Pajamas) => Some("l"),
            ShopKind::DowntownBoutique(WardrobeCategory::Winterwear) => Some("w"),
            ShopKind::StudioWardrobe => Some("h"),
            _ => None,
        };
        let Some(prefix) = category_prefix else {
            return targets;
        };
        let try_on_items: Vec<&ShopItem> = items.iter().filter(|i| i.supports_try_on).collect();
        if try_on_items.is_empty() {
            return targets;
        }
        let world = self.lot.world();
        let Some(person) = world.selected().or_else(|| {
            world
                .objects
                .iter()
                .find(|(_, o)| o.person.is_some())
                .map(|(&id, _)| id)
        }) else {
            return targets;
        };
        let (female, child) = world
            .objects
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .map(|p| {
                let kind_word = p.data.get(sims_sim::neighbors::KIND).copied().unwrap_or(0);
                let age = p.data.get(58).copied().unwrap_or(27);
                ((kind_word & 1) == 1, age < 18)
            })
            .unwrap_or((false, false));
        let skin = self.person_skin_tone(world, person);

        if prefix == "b" {
            let mut everyday_rows: Vec<(String, String)> = Vec::new();
            if let Some(wardrobe) = self.ensure_ordered_wardrobe() {
                let count = wardrobe.count(false, female, child, &skin).min(32);
                for idx in 0..count {
                    if let Some((suit, row)) = wardrobe.pick(false, female, child, &skin, idx) {
                        everyday_rows.push((suit, row));
                    }
                }
            }
            for (idx, item) in try_on_items.iter().enumerate() {
                if let Some((suit, row)) = everyday_rows.get(idx) {
                    targets.insert(
                        item.id.clone(),
                        ShopPreviewTarget::human(person, suit.clone(), Some(row.clone())),
                    );
                }
            }
            return targets;
        }

        let gender_age = match (female, child) {
            (true, false) => "fa",
            (false, false) => "ma",
            (true, true) => "fc",
            (false, true) => "mc",
        };
        let mut category_suits: Vec<String> = Vec::new();
        if let Some(assets) = self.ensure_shop_preview_assets() {
            let mut matched: Vec<String> = sims_people::appearance::appearance_names(assets)
                .filter(|name| {
                    let lower = name.to_ascii_lowercase();
                    lower.starts_with(prefix)
                        && lower.len() >= 6
                        && lower.as_bytes()[1..4].iter().all(u8::is_ascii_digit)
                        && lower.as_bytes().get(4..6) == Some(gender_age.as_bytes())
                })
                .map(str::to_owned)
                .collect();
            matched.sort();
            matched.dedup();
            category_suits = matched;
        }
        for (idx, item) in try_on_items.iter().enumerate() {
            if let Some(suit) = category_suits.get(idx) {
                targets.insert(
                    item.id.clone(),
                    ShopPreviewTarget::human(person, suit.clone(), None),
                );
            }
        }
        targets
    }

    pub(super) fn apply_shop_try_on(&mut self, item_id: &str, target: Option<&ShopPreviewTarget>) {
        let mut tried_name = item_id.to_owned();
        let is_new_item = self
            .shop_snapshot
            .as_ref()
            .and_then(|s| s.tried_on_item_id.as_deref())
            != Some(item_id);
        if is_new_item {
            self.shop_preview_yaw_deg = 0;
            self.shop_preview_body = None;
        }
        if let Some(snap) = &self.shop_snapshot
            && let Some(item) = snap.items.iter().find(|i| i.id == item_id)
        {
            tried_name = item.name.clone();
        }
        let image = self.render_shop_try_on_image(item_id, target);
        if let Some(snap) = &mut self.shop_snapshot {
            snap.tried_on_item_id = Some(item_id.to_owned());
            if image.is_some() {
                snap.preview_image = image;
                snap.preview_label = Some(tried_name.clone());
            } else {
                snap.preview_image = None;
                snap.preview_label = Some("Preview unavailable".into());
            }
        }
        self.note = Some(format!("Previewing {tried_name}."));
    }

    pub(super) fn apply_shop_rotate_preview(&mut self, delta_deg: i16) {
        self.shop_preview_yaw_deg =
            (i32::from(self.shop_preview_yaw_deg) + i32::from(delta_deg)).rem_euclid(360) as i16;
        let yaw = sims_people::caf_preview::TURNTABLE_BASE_YAW
            + f32::from(self.shop_preview_yaw_deg).to_radians();
        let params = self.shop_preview_params(yaw);
        let rendered = self
            .shop_preview_body
            .as_ref()
            .and_then(|(_, body)| {
                sims_people::caf_preview::render_turntable(body, None, &params).ok()
            })
            .map(|canvas| sims_formats::bitmap::Image {
                width: canvas.width,
                height: canvas.height,
                rgba: canvas.rgba,
            });
        if let (Some(img), Some(snap)) = (rendered, &mut self.shop_snapshot) {
            snap.preview_image = Some(img);
        }
    }

    pub(super) fn person_head_binding(
        &self,
        world: &sims_sim::World,
        person: u16,
    ) -> Option<(String, String)> {
        let obj = world.objects.get(&person)?;
        if let Some(outfits) = world.made_outfits.get(&obj.guid)
            && let Some(raw) = outfits.get(&2)
        {
            let (app, tex) = sims_people::appearance::body_string(raw);
            if !app.is_empty() {
                return Some((app, tex));
            }
        }
        let catalog = self.assets.catalog();
        if let Some(entry) = catalog
            .by_guid
            .get(&obj.guid)
            .and_then(|&idx| catalog.objects.get(idx))
            && let Some(res) = self.assets.index().resources.get(entry.resource)
            && let Ok(bytes) = res.read()
            && let Ok(iff) = sims_formats::iff::Iff::parse(bytes)
            && let Some(chunk) = iff.get(b"STR#", 200)
            && let Ok(table) = sims_formats::strings::StringTable::parse(iff.data(chunk))
            && let Some(raw) = table.get(2, US_ENGLISH).or_else(|| table.get(2, 1))
        {
            let (app, tex) = sims_people::appearance::body_string(raw);
            if !app.is_empty() {
                return Some((app, tex));
            }
        }
        None
    }

    pub(super) fn render_shop_try_on_image(
        &mut self,
        item_id: &str,
        target: Option<&ShopPreviewTarget>,
    ) -> Option<sims_formats::bitmap::Image> {
        let target = target?;
        if let Some(skeleton) = target.pet_skeleton.clone() {
            return self.render_pet_try_on_image(item_id, target, &skeleton);
        }
        let world = self
            .lot
            .asking_special_panel()
            .map(|a| &a.world)
            .unwrap_or_else(|| self.lot.world());
        let person = target.person;
        let (female, child) = world
            .objects
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .map(|p| {
                let kind = p.data.get(sims_sim::neighbors::KIND).copied().unwrap_or(0);
                let age = p.data.get(58).copied().unwrap_or(27);
                ((kind & 1) == 1, age < 18)
            })
            .unwrap_or((false, false));
        let skin_code = self.person_skin_tone(world, person);
        let head_override = self.person_head_binding(world, person);
        let p_age = if child {
            sims_people::caf_preview::Age::Child
        } else {
            sims_people::caf_preview::Age::Adult
        };
        let p_gender = if female {
            sims_people::caf_preview::Gender::Female
        } else {
            sims_people::caf_preview::Gender::Male
        };
        let p_skin = match skin_code.as_str() {
            "med" => sims_people::caf_preview::SkinTone::Medium,
            "drk" => sims_people::caf_preview::SkinTone::Dark,
            _ => sims_people::caf_preview::SkinTone::Light,
        };

        if self
            .shop_preview_body
            .as_ref()
            .is_none_or(|(id, _)| id != item_id)
        {
            let assets = self.ensure_shop_preview_assets()?;
            let wardrobe = sims_people::suit::Wardrobe::of(assets);
            let catalog = sims_people::caf_preview::enumerate_catalog_from_wardrobe(
                &wardrobe, p_age, p_gender, p_skin,
            );
            let head = head_override
                .filter(|(app, _)| sims_people::appearance::load_appearance(assets, app).is_ok())
                .or_else(|| {
                    catalog
                        .heads
                        .first()
                        .map(|h| (h.appearance.clone(), h.texture_binding.clone()))
                })?;

            let mut candidate_bodies: Vec<(String, String)> = Vec::new();
            if let Some(row) = &target.row {
                let (app, tex) = sims_people::appearance::body_string(row);
                if !app.is_empty() {
                    candidate_bodies.push((app, tex));
                }
            }
            if !target.suit.is_empty() {
                if sims_people::appearance::load_appearance(assets, &target.suit).is_ok() {
                    candidate_bodies.push((target.suit.clone(), String::new()));
                } else {
                    // `scan_install_skin_names` may return a `.bmp` texture stem
                    // such as `s001fafitlgt_bikini`; match its appearance prefix.
                    let lower_suit = target.suit.to_ascii_lowercase();
                    let prefix_len = lower_suit.len().min(9);
                    let matched_app = sims_people::appearance::appearance_names(assets)
                        .find(|name| {
                            let ln = name.to_ascii_lowercase();
                            ln == lower_suit
                                || (prefix_len >= 6
                                    && ln.as_bytes().get(..prefix_len)
                                        == lower_suit.as_bytes().get(..prefix_len))
                        })
                        .map(str::to_owned);
                    if let Some(app) = matched_app {
                        candidate_bodies.push((app, format!("BODY={}", target.suit)));
                    }
                }
            }

            let mut loaded_body = None;
            for (body_app, body_tex) in candidate_bodies {
                let cfg = sims_people::caf_preview::SimConfig {
                    age: p_age,
                    gender: p_gender,
                    skin_tone: p_skin,
                    head: head.clone(),
                    body: (body_app, body_tex),
                    hands: None,
                };
                if let Ok(body) = cfg.load_body(assets) {
                    loaded_body = Some(body);
                    break;
                }
            }
            let body = loaded_body?;
            self.shop_preview_body = Some((item_id.to_owned(), body));
        }

        let (_, body) = self.shop_preview_body.as_ref()?;
        let yaw = sims_people::caf_preview::TURNTABLE_BASE_YAW
            + f32::from(self.shop_preview_yaw_deg).to_radians();
        let params = self.shop_preview_params(yaw);
        let canvas = sims_people::caf_preview::render_turntable(body, None, &params).ok()?;
        Some(sims_formats::bitmap::Image {
            width: canvas.width,
            height: canvas.height,
            rgba: canvas.rgba,
        })
    }

    /// Renders a pet adoption candidate on the pet's own skeleton: the
    /// picked body (`BODY=` row) alone, as pets wear no head or hands.
    pub(super) fn render_pet_try_on_image(
        &mut self,
        item_id: &str,
        target: &ShopPreviewTarget,
        skeleton: &str,
    ) -> Option<sims_formats::bitmap::Image> {
        if self
            .shop_preview_body
            .as_ref()
            .is_none_or(|(id, _)| id != item_id)
        {
            let row = target.row.as_deref()?;
            let (app, tex) = sims_people::appearance::body_string(row);
            let app = if app.is_empty() {
                target.suit.clone()
            } else {
                app
            };
            let assets = self.ensure_shop_preview_assets()?;
            let outfit = sims_people::appearance::Outfit {
                parts: vec![(app, tex)],
            };
            let body = sims_people::Body::load(assets, skeleton, &outfit).ok()?;
            self.shop_preview_body = Some((item_id.to_owned(), body));
        }
        let (_, body) = self.shop_preview_body.as_ref()?;
        let yaw = sims_people::caf_preview::TURNTABLE_BASE_YAW
            + f32::from(self.shop_preview_yaw_deg).to_radians();
        let params = self.shop_preview_params(yaw);
        let canvas = sims_people::caf_preview::render_turntable(body, None, &params).ok()?;
        Some(sims_formats::bitmap::Image {
            width: canvas.width,
            height: canvas.height,
            rgba: canvas.rgba,
        })
    }
}
