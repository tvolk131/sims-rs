//! Live prefix inputs and its borrowed selected-object scan. OS replies are
//! consumed by the shared external-input owner, not stored as game-state facts.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fields {
    pub drawing_all: u8,
    pub dirty_region: u32,
    pub rectangle: [i32; 4],
    pub overlay: u32,
    pub overlay_guard: u32,
    pub scan_enabled: u8,
    pub include_visible: u8,
    pub matched: Option<sims_vm::ObjectId>,
    pub scanned: bool,
}
impl Fields {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.drawing_all == 0
                && self.dirty_region == 0
                && self.rectangle == [0, 0, 800, 600]
                && self.overlay == 0
                && self.overlay_guard == 0
                && self.scan_enabled == 1
                && self.include_visible == 1,
            "World prefix source footprint differs"
        );
        ensure!(
            self.scanned || self.matched.is_none(),
            "World redraw match precedes scan"
        );
        Ok(())
    }
}
impl Owner {
    pub(in crate::original_session::draw_parent) fn bind_prefix(
        &mut self,
        source: &[u8],
    ) -> Result<()> {
        self.validate()?;
        ensure!(
            self.at_entry() && self.prefix.is_none(),
            "World prefix binding already started"
        );
        ensure!(
            sims_fixtures::digest(source)
                == "f81d36af5488c0acafa0ebb73bcf9b4bbee767907e83726d8a6dfdded952d212",
            "World prefix source differs"
        );
        let value: serde_json::Value = serde_json::from_slice(source)?;
        ensure!(
            value["tick"] == 89073
                && value["executes_target_code"] == false
                && value["native_parity_credit"] == 0,
            "World prefix epoch differs"
        );
        let m = Memory::new(&value)?;
        let h = m.read(self.owner, 0x4c0)?;
        let fields = Fields {
            drawing_all: m.read(0x6f00e4, 1)?[0],
            dirty_region: word(&h, 0x4bc),
            rectangle: std::array::from_fn(|i| word(&h, 0x28 + i * 4) as i32),
            overlay: word(&m.read(0x6f6bac, 4)?, 0),
            overlay_guard: word(&h, 0x1a4),
            scan_enabled: h[0x49],
            include_visible: h[0x1f8],
            matched: None,
            scanned: false,
        };
        fields.validate()?;
        self.prefix = Some(fields);
        self.validate()
    }
    pub(in crate::original_session::draw_parent) fn client_reply(
        &mut self,
        reply: super::super::windows::ClientReply,
    ) -> Result<()> {
        self.validate()?;
        ensure!(
            self.phase == Phase::Window && self.prefix.is_some() && reply.window == self.window,
            "World client rectangle call is not bound/pending"
        );
        self.scratch = Some(reply.rectangle);
        self.client_result = Some(reply.result);
        self.phase = Phase::Rectangle;
        self.validate()
    }
    pub(in crate::original_session::draw_parent) fn scan(
        &mut self,
        simulation: &sims_sim::Simulation,
    ) -> Result<()> {
        self.validate()?;
        ensure!(
            self.phase == Phase::Scan,
            "World redraw scan is not pending"
        );
        let placement = &simulation
            .native_lot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("World redraw lot owner absent"))?
            .placement;
        let view = sims_sim::lot_presenter::redraw::Live {
            world: &simulation.vm.world,
            placement,
        };
        let prefix = self.prefix.as_mut().unwrap();
        let matched = sims_vm::presenter::redraw::first(&view, prefix.scan_enabled)
            .map_err(anyhow::Error::msg)?;
        prefix.matched = matched;
        prefix.scanned = true;
        self.phase = if matched.is_some() {
            Phase::Compose
        } else {
            Phase::Prepare
        };
        self.validate()
    }
    pub(in crate::original_session::draw_parent) fn matched(&self) -> Option<sims_vm::ObjectId> {
        self.prefix.as_ref().and_then(|p| p.matched)
    }
    pub(in crate::original_session::draw_parent) fn compare_native_prefix(
        &self,
        encoded: &[u8],
        simulation: &sims_sim::Simulation,
    ) -> Result<()> {
        use anyhow::Context as _;
        use std::io::Read;
        self.validate()?;
        ensure!(
            sims_fixtures::digest(encoded)
                == "f75dd16f738a78c4ddb2b3b4f05fd51e927de6a0ccfe50db8d07af815e6e7b26",
            "native World prefix witness differs"
        );
        let mut raw = String::new();
        flate2::read::GzDecoder::new(encoded).read_to_string(&mut raw)?;
        let rows: Vec<serde_json::Value> = raw
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let entry = rows
            .iter()
            .find(|r| r["kind"] == "same_stop_outer_readback" && r["pc"] == 0x4a8a32)
            .context("native World scan inputs absent")?;
        ensure!(
            entry["tick"] == simulation.vm.tick,
            "native redraw tick differs"
        );
        let source = &entry["redraw_inputs"];
        let objects = source["objects"]
            .as_array()
            .context("native redraw registry absent")?;
        let ids: Vec<sims_vm::ObjectId> = objects
            .iter()
            .map(|o| {
                o["id"]
                    .as_i64()
                    .context("native redraw identity absent")?
                    .try_into()
                    .map_err(Into::into)
            })
            .collect::<Result<_>>()?;
        let person = simulation
            .vm
            .world
            .person
            .as_ref()
            .context("retained person registry absent")?;
        ensure!(
            person.person_order == ids,
            "retained redraw person registry differs"
        );
        let placement = &simulation
            .native_lot
            .as_ref()
            .context("retained redraw placement absent")?
            .placement;
        let view = sims_sim::lot_presenter::redraw::Live {
            world: &simulation.vm.world,
            placement,
        };
        use sims_vm::presenter::redraw::View as _;
        ensure!(
            source["size"].as_i64()
                == Some(i64::from(view.active_size().map_err(anyhow::Error::msg)?)),
            "retained redraw active extent differs"
        );
        for (row, &id) in objects.iter().zip(&ids) {
            let b = crate::original_session::completed_animations::bytes(
                row["raw"].as_str().context("native redraw object absent")?,
            )?;
            ensure!(b.len() == 40, "native redraw object extent differs");
            ensure!(
                view.cached_tile(id).map_err(anyhow::Error::msg)?
                    == [b[12] as i8, b[13] as i8, b[14] as i8]
                    && view.native_flags(id).map_err(anyhow::Error::msg)? == word(&b, 36),
                "retained redraw tile/flags differ for{id}"
            );
        }
        let active = view.active_size().map_err(anyhow::Error::msg)? as usize;
        for floor in source["floors"]
            .as_array()
            .context("native redraw floor flags absent")?
        {
            let level = floor["level"]
                .as_u64()
                .context("native redraw level absent")? as usize;
            let b = crate::original_session::completed_animations::bytes(
                floor["raw"]
                    .as_str()
                    .context("native redraw flags absent")?,
            )?;
            ensure!(
                b.len() == active * active,
                "native redraw flag extent differs"
            );
            for x in 0..active {
                for y in 0..active {
                    ensure!(
                        placement.flags[level - 1][x * placement.links.size + y]
                            == b[x * active + y],
                        "retained live redraw tile flags differ at{x},{y},{level}"
                    );
                }
            }
        }
        let matched = rows
            .iter()
            .find(|r| r["kind"] == "same_stop_outer_readback" && r["pc"] == 0x4a8aef)
            .and_then(|r| r["registers"]["edi"].as_u64())
            .and_then(|pointer| objects.iter().find(|o| o["pointer"] == pointer))
            .map(|o| o["id"].as_i64().unwrap() as sims_vm::ObjectId);
        ensure!(
            self.prefix.as_ref().is_some_and(|p| p.scanned) && self.matched() == matched,
            "World redraw branch differs from original"
        );
        Ok(())
    }
}
