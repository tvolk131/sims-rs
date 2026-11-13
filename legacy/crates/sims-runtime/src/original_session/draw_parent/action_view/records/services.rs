//! Retained resident Bookcase thumbnail path. Captured inputs and native
//! packing/key flags feed the shared CPU compositor; native output pixels and
//! DirectDraw allocation bookkeeping are not qualified by this adapter.
use super::*;
use anyhow::Context;
use sims_content::dialog::portrait::{
    images::{Raster, thumbnail},
    resize::{Format, Image},
};
use sims_vm::scheduler::clock::draw::actions::record;
const FIRST: u32 = 0xe0000000;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Services {
    background: Raster,
    picture: Raster,
    next: u32,
    generated: BTreeMap<u32, Raster>,
    entry_backing: Vec<u8>,
}
pub(super) fn import(
    code: &[u8],
    object: &[u8],
    pixels: &[u8],
    backing: &[u8],
) -> Result<Services> {
    ensure!(
        sims_fixtures::digest(code)
            == "bea48ccffbf20eda27a988a9dd10c0c642d979cb5a93a1189b9ec4be11f30b40",
        "record services instruction source differs"
    );
    ensure!(
        sims_fixtures::digest(object)
            == "b8cf9f6be283efdf8780b49adc2fa0cf3667059aaed726f65ec24e3535a121fe",
        "record services object source differs"
    );
    ensure!(
        sims_fixtures::digest(pixels)
            == "1eb27d713a1521343ba4fabf9133271f61b57862931de3dc96ee4d46987bbc19",
        "record services image source differs"
    );
    let code: serde_json::Value = serde_json::from_slice(code)?;
    let memory = Memory::new(&code)?;
    let owner = word(&memory.read(0x6e48bc, 4)?, 0);
    let format = word(&memory.read(owner + 0x34, 4)?, 0);
    ensure!(format == 2, "record services display packing differs");
    let object: serde_json::Value = serde_json::from_slice(object)?;
    let object = Memory::new(&object)?;
    let resource = object.read(0x0872c500, 0x90)?;
    ensure!(
        word(&resource, 0x8c) == 0x0872c500 && word(&resource, 0x84) == 0x0be10948,
        "record services warm master/descriptor differs"
    );
    let image: serde_json::Value = serde_json::from_slice(pixels)?;
    let memory = Memory::new(&image)?;
    let raster = |address| -> Result<Raster> {
        let header = memory.read(address, 100)?;
        ensure!(word(&header,0xc)==16,"record source is not a16-bit bitmap");
        let surface = word(&header, 0x48);
        let layout = memory.read(surface + 0x178, 8)?;
        let (width, height) = (word(&header, 0x1c) as usize, word(&header, 0x20) as usize);
        let stride = word(&layout, 4) as usize;
        ensure!(
            width <= 180 && height <= 45 && stride >= 2 * width,
            "record services source dimensions differ"
        );
        let raw = memory.read(word(&layout, 0), stride * height)?;
        Ok(Raster {
            format: Format::Rgb565,
            key: (word(&header, 0x28) & 2 != 0).then_some(word(&header, 0x24) as u16),
            image: Image {
                width,
                height,
                alpha: None,
                pixels: (0..height)
                    .flat_map(|y| (0..width).map(move |x| (x, y)))
                    .map(|(x, y)| {
                        u16::from_le_bytes(raw[y * stride + 2 * x..][..2].try_into().unwrap())
                    })
                    .collect(),
            },
        })
    };
    let result = Services {
        background: raster(0x09e27418)?,
        picture: raster(0x0b7c9d58)?,
        next: FIRST,
        generated: BTreeMap::new(),
        entry_backing: backing.to_vec(),
    };
    result.validate()?;
    Ok(result)
}
impl Services {
    pub(super) fn sizes(&self)->BTreeMap<u32,[i32;2]> {self.generated.iter().map(|(&id,r)|(id,[r.image.width as i32,r.image.height as i32])).collect()}

    pub(super) fn validate(&self) -> Result<()> {
        for (r, size, key, hash) in [
            (
                &self.background,
                [180, 45],
                Some(0xf81f),
                "c13a6e247e9ccb96e5b3c3000a7c6fde4928a00bf28831107c8bd1396a1ce3e4",
            ),
            (
                &self.picture,
                [74, 37],
                None,
                "2c3f7ebe25cd044106d625e37907ada2fe5d7bad87bf63e1bf6e0636c33f6b9e",
            ),
        ] {
            r.validate()?;
            ensure!(
                r.format == Format::Rgb565
                    && [r.image.width, r.image.height] == size
                    && r.key == key
                    && r.image.alpha.is_none(),
                "record services resident raster metadata differs"
            );
            let raw: Vec<u8> = r
                .image
                .pixels
                .iter()
                .flat_map(|p| p.to_le_bytes())
                .collect();
            ensure!(
                sims_fixtures::digest(&raw) == hash,
                "record services resident pixels differ"
            );
        }
        ensure!(
            self.next >= FIRST
                && (self.next - FIRST) % 4 == 0
                && self.generated.len() as u32 == (self.next - FIRST) / 4,
            "record services image handle ownership differs"
        );
        for (&p, r) in &self.generated {
            r.validate()?;
            ensure!(
                p >= FIRST
                    && p < self.next
                    && p % 4 == 0
                    && [r.image.width, r.image.height] == [45, 45]
                    && r.format == Format::Rgb565,
                "record services generated raster differs"
            );
        }
        Ok(())
    }
    pub(super) fn backing(&self) -> &[u8] {
        &self.entry_backing
    }
    fn thumbnail(&mut self, counts: &mut BTreeMap<u32, u32>) -> Result<(u32, bool)> {
        let rendered = thumbnail::compose(&self.background, &self.picture, None)?;
        let Some(image) = rendered.image else {
            return Ok((0, rendered.success));
        };
        let id = self.next;
        self.next = self
            .next
            .checked_add(4)
            .context("record image handle range exhausted")?;
        self.generated.insert(id, image);
        ensure!(counts.insert(id, 1).is_none(), "record image handle reused");
        Ok((id, rendered.success))
    }
    pub(super) fn build(
        &mut self,
        counts: &mut BTreeMap<u32, u32>,
        simulation: &sims_sim::Simulation,
        actor: sims_vm::ObjectId,
        request: &actions::collect::Request,
    ) -> Result<(actions::Record, bool)> {
        let actions::collect::Request::Build {
            index,
            person,
            node,
            current,
            record,
        } = request
        else {
            anyhow::bail!("record builder request absent");
        };
        ensure!(
            *node == person.wrapping_add(0xa48),
            "queued action pointer binding needs an original source owner"
        );
        let mut raw = *record;
        let mut host = Builder {
            services: self,
            counts,
            simulation,
            actor,
            node: *node,
        };
        let success = record::build(&mut raw, *index, *person, *node, *current as u8, &mut host)
            .map_err(anyhow::Error::msg)?;
        Ok((raw, success))
    }
}
pub(super) fn assign(
    counts: &mut BTreeMap<u32, u32>,
    slot: &mut u32,
    image: u32,
) -> Result<(), String> {
    if image != 0 {
        let count = counts
            .get_mut(&image)
            .ok_or("record incoming image absent")?;
        *count = count.wrapping_add(1);
    }
    if *slot != 0 {
        release(counts, *slot)?;
    }
    *slot = image;
    Ok(())
}
pub(super) fn release(counts: &mut BTreeMap<u32, u32>, image: u32) -> Result<(), String> {
    let count = counts
        .get_mut(&image)
        .ok_or("record released image absent")?;
    if *count <= 1 {
        return Err("record image deleting destructor needs its retained owner".into());
    }
    *count -= 1;
    Ok(())
}
struct Builder<'a> {
    services: &'a mut Services,
    counts: &'a mut BTreeMap<u32, u32>,
    simulation: &'a sims_sim::Simulation,
    actor: sims_vm::ObjectId,
    node: u32,
}
impl record::Images for Builder<'_> {
    fn assign(&mut self, slot: &mut u32, image: u32) -> Result<(), String> {
        assign(self.counts, slot, image)
    }
    fn release(&mut self, image: u32) -> Result<(), String> {
        release(self.counts, image)
    }
}
impl record::Build for Builder<'_> {
    fn icon_owner(&self, node: u32) -> Result<u32, String> {
        if node != self.node {
            return Err("record builder action node differs".into());
        }
        let actor = self
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .and_then(|s| s.actors.get(&self.actor))
            .ok_or("record builder action owner absent")?;
        let id = actor.icon_owner();
        if id == 0 {
            return Ok(0);
        }
        if id != 132
            || self
                .simulation
                .vm
                .world
                .objects
                .get(&id)
                .and_then(|o| o.definition.as_ref())
                .is_none_or(|d| d.guid != 0x5cdc712f)
        {
            return Err("record icon requires an original object/resource binding".into());
        }
        Ok(0x0c3dcff0)
    }
    fn kind(&self, object: u32) -> Result<u16, String> {
        if object != 0x0c3dcff0 {
            return Err("record icon object differs".into());
        }
        Ok(4)
    }
    fn resource(&self, object: u32) -> Result<u32, String> {
        if object != 0x0c3dcff0 {
            return Err("record icon resource differs".into());
        }
        Ok(0x0872c500)
    }
    fn guid(&self, _: u32) -> Result<u32, String> {
        Err("person icon resource not bound".into())
    }
    fn cache_index(&self, guid: u32) -> i32 {
        sims_vm::interface::person_selection::cache_index(
            &self.simulation.vm.world.image_cache,
            guid,
        )
    }
    fn portrait(&mut self, _: i32, _: &mut u32) -> Result<bool, String> {
        Err("person icon factory not bound".into())
    }
    fn master(&mut self, resource: u32) -> Result<Option<u32>, String> {
        if resource != 0x0872c500 {
            return Err("record icon master resource differs".into());
        }
        Ok(Some(resource))
    }
    fn descriptor(&self, master: u32) -> Result<Option<u32>, String> {
        if master != 0x0872c500 {
            return Err("record icon master differs".into());
        }
        Ok(Some(0x0be10948))
    }
    fn thumbnail(&mut self, descriptor: u32, temporary: &mut u32) -> Result<bool, String> {
        if descriptor != 0x0be10948 {
            return Err("record thumbnail descriptor differs".into());
        }
        let (image, success) = self
            .services
            .thumbnail(self.counts)
            .map_err(|e| e.to_string())?;
        *temporary = image;
        Ok(success)
    }
    fn active_counter(&self, _: u32) -> Result<u16, String> {
        self.simulation
            .vm
            .world
            .objects
            .get(&self.actor)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.get(&39))
            .copied()
            .map(|v| v as u16)
            .ok_or_else(|| "record cancelability person39 absent".into())
    }
}
pub(super) struct Copy<'a> {
    pub strings: &'a mut constructor::Storage,
    pub counts: &'a mut BTreeMap<u32, u32>,
    pub simulation: &'a mut sims_sim::Simulation,
    pub program: &'a sims_vm::Program,
    pub actor: sims_vm::ObjectId,
    pub node: u32,
}
impl record::Strings for Copy<'_> {
    fn allocate(&mut self, n: u32) -> Result<u32, String> {
        record::Strings::allocate(self.strings, n)
    }
    fn write_nul(&mut self, p: u32) -> Result<(), String> {
        record::Strings::write_nul(self.strings, p)
    }
}
impl record::Images for Copy<'_> {
    fn assign(&mut self, slot: &mut u32, image: u32) -> Result<(), String> {
        assign(self.counts, slot, image)
    }
    fn release(&mut self, image: u32) -> Result<(), String> {
        release(self.counts, image)
    }
}
impl record::Copy for Copy<'_> {
    fn action_label(&mut self, node: u32) -> Result<Vec<u8>, String> {
        if node != self.node {
            return Err("record copy action node differs".into());
        }
        let label = self.simulation.vm.displayed_action_label(
            self.program,
            self.actor,
            sims_vm::action::DisplaySlot::Active,
        )?;
        if !label.is_ascii() {
            return Err("native displayed caption requires its captured code page".into());
        }
        Ok(label.into_bytes())
    }
    fn caption(&mut self, header: &mut [u32; 3], bytes: &[u8]) -> Result<(), String> {
        self.strings
            .assign(header, bytes)
            .map_err(|e| e.to_string())
    }
}
