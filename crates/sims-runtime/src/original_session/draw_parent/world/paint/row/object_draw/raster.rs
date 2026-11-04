//! B-resident SPR2 payloads and shared scalar blitter owners. Observed later
//! returns are comparisons only; missing branches are never acknowledged.
use super::*;
use sims_sim::lot_presenter::{sprite2_clipped as blit, terrain_raster::Data};
const INPUT: &str = "14087b98c0c94359504568c25eaddf9057150c3a278ca6e2511209eac1396c13";
const FAMILY_INPUT:&str="d9c0c7367219f9563e3489cc06e29464f472f7d77e2ec36ac7fed8b1ab4f983a";
const BATCH_INPUT: &str = "0e4761e0c038e90ffc2534ff9452d88abc3f24dc9fc917838873a17848528ada";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BatchOwner {
    identity: String,
    scratch: Vec<u8>,
    alphas: Vec<u8>,
    copy: blit::CopyProfile,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MirrorOwner {
    identity: String,
    targets: [u32; 2],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Image {
    palette: u32,
    frames: Vec<Vec<u8>>,
    addresses: Vec<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Palette {
    header: Vec<u8>,
    unit: Vec<u8>,
    working: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::original_session::draw_parent::world::paint::row) struct Source {
    identity: String,
    images: BTreeMap<u32, Image>,
    palettes: BTreeMap<u32, Palette>,
    unit: u32,
    epsilon: u32,
    generation: u32,
    base_offsets: [i16; 3],
    z: Vec<u8>,
    mix: Vec<u8>,
    old: Vec<u8>,
    colors: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    batch: Option<BatchOwner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mirror: Option<MirrorOwner>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    family:Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Job {
    resource: u32,
    index: i32,
    origin: [i32; 2],
    rectangle: [i32; 4],
    margins: [i32; 4],
    base: i32,
    palette: Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    selector:Option<sims_sim::lot_presenter::palette_select::Selection>,
    prepared: bool,
    writes: Vec<blit::Write>,
    next: usize,
    complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    flags: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    returned_argument: Option<blit::ArgumentCursor>,
}
impl Source {
    pub fn import(input: &[u8]) -> Result<Self> {
        ensure!(
            matches!(sims_fixtures::digest(input).as_str(),INPUT|FAMILY_INPUT),
            "SPR2 raster B source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        ensure!(
            v["tick"] == 89073
                && v["executes_target_code"] == false
                && v["catalog_sha256"] == if sims_fixtures::digest(input)==INPUT{RESIDENT_CATALOG_SOURCE}else{OVERRIDE_FAMILY_SOURCE},
            "SPR2 raster epoch/catalog differs"
        );
        let m = Memory::with_union_limit(&v, 4 << 20)?;
        let mut images = BTreeMap::new();
        let mut palettes = BTreeMap::new();
        for e in v["sprites"]
            .as_array()
            .context("SPR2 raster entries absent")?
        {
            let p = e["pointer"].as_u64().context("SPR2 entry absent")? as u32;
            let raw = m.read(p, 44)?;
            let data = word(&raw, 12);
            let size = word(&raw, 40) as usize;
            let palette = word(&raw, 8);
            let encoded = m.read(data, size)?;
            let count = word(&raw, 4) as usize;
            ensure!(
                e["data"] == data
                    && e["bytes"] == size
                    && e["palette"] == palette
                    && word(&raw, 32) == 1000
                    && word(&encoded, 0) == 1000
                    && word(&encoded, 4) as usize == count
                    && count <= 1024,
                "SPR2 payload alias differs"
            );
            let mut frames = Vec::new();
            let mut addresses = Vec::new();
            for i in 0..count {
                let start = word(&encoded, 12 + 4 * i) as usize;
                let end = if i + 1 < count {
                    word(&encoded, 16 + 4 * i) as usize
                } else {
                    size
                };
                let f = encoded
                    .get(start..end)
                    .context("SPR2 frame extent invalid")?;
                ensure!(f.len() >= 18, "SPR2 frame header truncated");
                frames.push(f.to_vec());
                addresses.push(
                    data.checked_add(u32::try_from(start)?)
                        .context("SPR2 address overflow")?,
                );
            }
            ensure!(
                images
                    .insert(
                        p,
                        Image {
                            palette,
                            frames,
                            addresses
                        }
                    )
                    .is_none(),
                "SPR2 duplicate entry"
            );
            if !palettes.contains_key(&palette) {
                palettes.insert(
                    palette,
                    Palette {
                        header: m.read(palette, 32)?,
                        unit: m.read(palette + 0x420, 512)?,
                        working: m.read(palette + 0xc20, 512)?,
                    },
                );
            }
        }
        let depth = m.read(0x09e88960 + 0xb8, 0xb0)?;
        let mut base_offsets = [0; 3];
        for zoom in 1..=3 {
            let ptr = word(&depth, 4 + (3 - zoom) * 4);
            base_offsets[zoom - 1] =
                i16::from_le_bytes(m.read(ptr + 2 * (8 << zoom), 2)?.try_into().unwrap());
        }
        ensure!(
            word(&m.read(0x6f0ab0, 4)?, 0) == 0x4bc606,
            "SPR2 clipped blitter target differs"
        );
        let s = Self {
            identity: sims_fixtures::digest(input),
            images,
            palettes,
            unit: word(&m.read(0x666748, 4)?, 0),
            epsilon: word(&m.read(0x67343c, 4)?, 0),
            generation: word(&m.read(0x6f0180, 12)?, 4),
            base_offsets,
            z: m.read(0x6f0cb0, 512)?,
            mix: m.read(0x6e5eac, 32768)?,
            old: m.read(0x6f09dc, 64)?,
            colors: m.read(0x6f0a5c, 64)?,
            batch: None,
            mirror: None,
            family:None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.identity.as_str(),INPUT|FAMILY_INPUT)
                && self.images.len() == if self.identity==FAMILY_INPUT{24}else if self.family.is_some(){124}else{100}
                && !self.palettes.is_empty()
                && self.z.len() == 512
                && self.mix.len() == 32768
                && self.old.len() == 64
                && self.colors.len() == 64,
            "SPR2 raster owner differs"
        );
        if let Some(f)=&self.family{ensure!(self.identity==INPUT&&f==FAMILY_INPUT,"SPR2 family provenance differs");}
        if let Some(b)=&self.batch {
            ensure!(b.identity==BATCH_INPUT && b.scratch.len()==0x314 && b.alphas.len()==64
                && b.copy.dispatch==[1;6] && b.copy.plane_mod32==[16;2],"SPR2 batched owner differs");
        }
        if let Some(m)=&self.mirror {
            ensure!(m.identity==INPUT && m.targets==[0x4bca26,0x4bd590],"SPR2 mirrored dispatch differs");
        }
        Ok(())
    }
    pub fn bind_family(&mut self,input:&[u8])->Result<()>{
        ensure!(self.identity==INPUT&&sims_fixtures::digest(input)==FAMILY_INPUT,"SPR2 family source differs");
        if self.family.is_some(){return self.validate();}
        let value=Self::import(input)?;
        ensure!(self.unit==value.unit&&self.epsilon==value.epsilon&&self.generation==value.generation&&self.base_offsets==value.base_offsets&&self.z==value.z&&self.mix==value.mix,"SPR2 family global owners differ");
        for p in value.images.keys(){ensure!(!self.images.contains_key(p),"SPR2 family replaces existing image");}
        for(p,palette)in &value.palettes{if let Some(old)=self.palettes.get(p){ensure!(old==palette,"overlapping SPR2 palette differs");}}
        //Keep current old/source colors, MMX scratch, copy dispatch and mirrors.
        self.images.extend(value.images);self.palettes.extend(value.palettes);self.family=Some(FAMILY_INPUT.into());self.validate()
    }
    pub fn bind_mirror(&mut self, encoded: &[u8]) -> Result<()> {
        ensure!(self.mirror.is_none() && sims_fixtures::digest(encoded)==INPUT,"SPR2 mirrored B source differs/already bound");
        let v:serde_json::Value=serde_json::from_slice(encoded)?;
        ensure!(v["tick"]==89073 && v["executes_target_code"]==false,"SPR2 mirrored B epoch differs");
        let m=Memory::with_union_limit(&v,4<<20)?;
        let raw=m.read(0x6f0ab4,8)?;
        self.mirror=Some(MirrorOwner {identity:INPUT.into(),targets:[word(&raw,0),word(&raw,4)]});
        self.validate()
    }
    pub fn bind_batch(&mut self, encoded: &[u8], targets: sims_vm::scheduler::clock::draw::targets::Buffers) -> Result<()> {
        ensure!(self.batch.is_none(),"SPR2 batched owner already bound");
        ensure!(sims_fixtures::digest(encoded)==BATCH_INPUT,"SPR2 batched B source differs");
        let v:serde_json::Value=serde_json::from_slice(encoded)?;
        ensure!(v["tick"]==89073 && v["executes_target_code"]==false && v["native_parity_credit"]==0,"SPR2 batched B epoch differs");
        ensure!(targets.color==0x0f380030 && targets.depth==0x0f560030 && targets.color_pitch==1600 && targets.depth_pitch==1600,"SPR2 batched backing planes differ");
        let m=Memory::with_union_limit(&v,4096)?;
        let raw=m.read(0x6d21e8,24)?;
        let b=BatchOwner {identity:BATCH_INPUT.into(),scratch:m.read(0x6f06c8,0x314)?,alphas:m.read(0x6f0a1c,64)?,
            copy:blit::CopyProfile {dispatch:std::array::from_fn(|n|word(&raw,n*4)),plane_mod32:[(targets.color&31) as u8,(targets.depth&31) as u8]}};
        ensure!(b.copy.dispatch==[1;6],"SPR2 original CRT feature dispatch differs");
        self.batch=Some(b);self.validate()
    }
    pub fn compare_batch(&self,q:&serde_json::Value)->Result<()> {
        let v=&q["spr2_owners"];
        ensure!(v.is_object(),"native SPR2 owner readback absent");
        let b=self.batch.as_ref().context("SPR2 batched owner unbound")?;
        for (key,raw) in [("batch",&b.scratch),("alphas",&b.alphas),("old",&self.old),("colors",&self.colors)] {
            ensure!(crate::original_session::completed_animations::bytes(v[key].as_str().context("native SPR2 owner missing")?)?==*raw,"native SPR2 {key} owner differs");
        }
        let flags=crate::original_session::completed_animations::bytes(v["copy_dispatch"].as_str().context("native CRT flags absent")?)?;
        ensure!(flags.len()==24 && b.copy.dispatch==std::array::from_fn(|n|word(&flags,n*4)),"native SPR2 CRT dispatch differs");
        Ok(())
    }
    pub fn begin(
        &self,
        frames: &frames::Source,
        id: u32,
        index: i32,
        origin: [i32; 2],
        clip: [i16; 4],
        context: &[u32; 29],
        mask: u8,
    ) -> Result<Option<Job>> {
        self.validate()?;
        // Other selection/transform/auxiliary branches still require their owners.
        if mask & !1 != 0
            || mask == 1 && self.mirror.is_none()
            || context[13] & 0xffffff00 != 0
            || context[14] & 255 != 0
            || context[23] & 8 != 0
            || context[22] & 3 != 3
        {
            return Ok(None);
        }
        let resource = frames
            .resident_pointer(id)
            .context("SPR2 resident entry absent")?;
        let Some(image) = self.images.get(&resource) else {
            return Ok(None);
        };
        let frame = image
            .frames
            .get(usize::try_from(index)?)
            .context("SPR2 raster frame index invalid")?;
        ensure!(
            frame[..16] == frames.frame(id, index)?[..],
            "SPR2 metadata/payload disagree"
        );
        let light = f64::from(f32::from_bits(
            sims_sim::lot_presenter::floor_prep::brightness(context),
        ));
        if !light.is_finite(){return Ok(None);}
        let p=self.palettes.get(&image.palette).context("palette owner absent")?;
        let selector=sims_sim::lot_presenter::palette_select::Selection::new(image.palette,sims_sim::lot_presenter::floor_prep::brightness(context),self.unit,self.epsilon,word(&p.header,0x18),word(&p.header,0x1c),self.generation).map_err(anyhow::Error::msg)?;
        let width = i32::from(u16::from_le_bytes(frame[..2].try_into()?));
        let height = i32::from(u16::from_le_bytes(frame[2..4].try_into()?));
        let r = [
            origin[0],
            origin[1],
            origin[0].wrapping_add(width),
            origin[1].wrapping_add(height),
        ];
        let rectangle = sims_sim::lot_presenter::room_masks::clip_rectangle(
            r,
            [
                clip[1] as i32,
                clip[0] as i32,
                clip[3] as i32,
                clip[2] as i32,
            ],
        );
        let margins = [
            rectangle[0] - r[0],
            rectangle[1] - r[1],
            r[2] - rectangle[2],
            r[3] - rectangle[3],
        ];
        let zoom = context[8] as usize;
        ensure!((1..=3).contains(&zoom), "SPR2 context zoom differs");
        let base = (context[10] as i32).wrapping_add(i32::from(self.base_offsets[zoom - 1]));
        Ok(Some(Job {
            resource,
            index,
            origin,
            //4c2071 returns the original -60 rectangle after rasterization;
            //the clamped copy only supplies the blitter margins.
            rectangle: r,
            margins,
            base,
            palette:None,
            selector:Some(selector),
            prepared: false,
            writes: vec![],
            next: 0,
            complete: false,
            flags: (mask != 0).then_some(mask),
            returned_argument: None,
        }))
    }
}
impl Job {
    pub fn advance(&mut self, source: &mut Source, data: &mut Data) -> Result<bool> {
        if self.complete {
            return Ok(false);
        }
        if let Some(selector)=&mut self.selector{
            if !selector.complete(){
                let before=selector.cache_write();let progress=selector.advance().map_err(anyhow::Error::msg)?;
                if before.is_none(){if let Some([old,value])=selector.cache_write(){let owner=source.images[&self.resource].palette;let p=source.palettes.get_mut(&owner).context("mutable palette absent")?;ensure!(word(&p.header,0x18)==old,"palette cache store source differs");p.header[0x18..0x1c].copy_from_slice(&value.to_le_bytes());}}
                if selector.complete(){self.palette=selector.result();}
                return Ok(progress);
            }
        }
        if !self.prepared {
            let image = &source.images[&self.resource];
            let p = &source.palettes[&image.palette];
            let palette = if self.palette.context("selected palette absent")? == image.palette + 0x420 {
                &p.unit
            } else {
                &p.working
            };
            let input=blit::Input {
                frame: &image.frames[self.index as usize],
                palette,
                z: &source.z,
                mix: &source.mix,
                color: &data.color,
                depth: &data.depth,
                pitch: [data.color_pitch, data.depth_pitch],
                origin: self.origin,
                margins: self.margins,
                base: self.base,
            };
            self.writes=if self.flags.unwrap_or(0)&1!=0 {
                let (writes,argument)=blit::plan_mirrored_with_argument(input,source.batch.as_ref().map(|b|b.copy)).map_err(anyhow::Error::msg)?;
                self.returned_argument=(self.margins==[0;4]).then_some(argument);
                writes
            } else if let Some(batch)=&source.batch { blit::plan_batched(input,batch.copy).map_err(anyhow::Error::msg)? }
            else { blit::plan(input).map_err(anyhow::Error::msg)? };
            self.prepared = true;
            return Ok(true);
        }
        if let Some(w) = self.writes.get(self.next) {
            if let Some(b)=&mut source.batch {
                blit::apply_batched(w,&mut data.color,&mut data.depth,&mut source.old,&mut source.colors,&mut b.scratch,&mut b.alphas)
            } else { blit::apply(
                w,
                &mut data.color,
                &mut data.depth,
                &mut source.old,
                &mut source.colors,
            ) }
            .map_err(anyhow::Error::msg)?;
            self.next += 1;
        } else {
            self.complete = true;
        }
        Ok(true)
    }
    pub fn at_entry(&self)->bool{!self.prepared}
    pub fn entry_pc(&self)->u32 {
        if let Some(s)=&self.selector{if !s.complete(){return s.pc();}}
        if self.flags.unwrap_or(0)&1==0 {0x4bc606}
        else if self.margins==[0;4] {0x4bca26} else {0x4bd590}
    }
    pub fn return_pc(&self)->u32 {
        if self.flags.unwrap_or(0)&1==0 {0x4c1ee5}
        else if self.margins==[0;4] {0x4c1d14} else {0x4c1d6b}
    }
    pub fn store_progress(&self)->Option<(usize,usize)>{self.prepared.then_some((self.next,self.writes.len()))}
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn rectangle(&self) -> [i32; 4] {
        self.rectangle
    }
    pub fn compare(&self, source: &Source, q: &serde_json::Value, data: &Data) -> Result<()> {
        if q["palette_selector"].is_object(){
            let n=&q["palette_selector"];let image=&source.images[&self.resource];let p=&source.palettes[&image.palette];
            let selector=self.selector.as_ref().context("palette selector child absent")?;
            ensure!(selector.pc()==q["pc"].as_u64().context("palette PC absent")?as u32,"palette selector boundary differs");
            ensure!(n["owner"]==image.palette&&n["generation"]==source.generation&&n["input_bits"]==selector.light_bits(),"palette owner/live epoch differs");
            let header=crate::original_session::completed_animations::bytes(n["header"].as_str().context("palette header absent")?)?;
            let unit=crate::original_session::completed_animations::bytes(n["unit"].as_str().context("unit palette absent")?)?;
            let working=crate::original_session::completed_animations::bytes(n["working"].as_str().context("working palette absent")?)?;
            ensure!(header==p.header&&unit==p.unit&&working==p.working,"retained palette bytes differ");
            let sp=n["entry_esp"].as_u64().context("palette entry ESP absent")?as i64;
            ensure!(q["registers"]["esp"].as_i64()==Some(sp+i64::from(selector.esp_delta())),"palette selector stack lifetime differs");
            if let Some(eax)=selector.eax(){ensure!(q["registers"]["eax"]==eax,"palette selector EAX differs");}
            if selector.returned(){let pointer=selector.eax().context("palette result absent")?;let bytes=if pointer==image.palette+0x420{&p.unit}else{&p.working};ensure!(n["returned"]["pointer"]==pointer&&crate::original_session::completed_animations::bytes(n["returned"]["raw"].as_str().context("palette result bytes absent")?)?==*bytes,"palette returned bytes differ");}
            return Ok(());
        }
        if q["spr2_owners"].is_object() {source.compare_batch(q)?;}
        let call = &q["pixel_call"];
        if self.flags.unwrap_or(0)&1!=0 {
            let raw=crate::original_session::completed_animations::bytes(q["raster_functions"]["raw"].as_str().context("native SPR2 dispatch absent")?)?;
            let m=source.mirror.as_ref().context("SPR2 mirror owner absent")?;
            ensure!(word(&raw,0x34)==m.targets[0] && word(&raw,0x38)==m.targets[1],"native mirrored dispatch differs");
        }
        let a = call["arguments"]
            .as_array()
            .context("native SPR2 call absent")?;
        let image = &source.images[&self.resource];
        let expected_argument=if self.complete {
            match self.returned_argument {
                Some(blit::ArgumentCursor::ColorOffset(offset))=> {
                    let base=q["buffers"][0]["pointer"].as_u64().context("native SPR2 plane base absent")?;
                    (base as i64+offset) as u32
                }
                Some(blit::ArgumentCursor::FrameOffset(offset))=>image.addresses[self.index as usize].wrapping_add(offset as u32),
                None=>image.addresses[self.index as usize],
            }
        } else {image.addresses[self.index as usize]};
        let p = &source.palettes[&image.palette];
        let palette_address=self.palette.context("palette not yet selected")?;
        let palette = if palette_address == image.palette + 0x420 {
            &p.unit
        } else {
            &p.working
        };
        ensure!(
            a[2] == expected_argument
                && a[3] == palette_address
                && a[4] == self.base as u32
                && a[5] == 0x6f0cb0u32
                && call["margins"] == serde_json::to_value(self.margins)?
                && crate::original_session::completed_animations::bytes(
                    call["palette"].as_str().context("native palette absent")?
                )? == *palette
                && crate::original_session::completed_animations::bytes(
                    call["z_table"].as_str().context("native SPR2 z absent")?
                )? == source.z,
            "native SPR2 scalar call differs"
        );
        ensure!(
            crate::original_session::completed_animations::bytes(
                call["old_colors"]
                    .as_str()
                    .context("native old colors absent")?
            )? == source.old
                && crate::original_session::completed_animations::bytes(
                    call["source_colors"]
                        .as_str()
                        .context("native source colors absent")?
                )? == source.colors,
            "native SPR2 scratch differs"
        );
        let targets = q["buffers"]
            .as_array()
            .context("native SPR2 planes absent")?;
        for (index, pitch) in [data.color_pitch, data.depth_pitch].into_iter().enumerate() {
            let plane = targets.get(index).context("SPR2 plane missing")?["pointer"]
                .as_u64()
                .context("SPR2 plane pointer absent")? as i64;
            let pair = call[if index == 0 { "color" } else { "depth" }]
                .as_array()
                .context("SPR2 plane arguments absent")?;
            ensure!(
                pair[0].as_i64()
                    == Some(
                        plane
                            + i64::from(self.origin[1]) * i64::from(pitch)
                            + i64::from(self.origin[0]) * 2
                    )
                    && pair[1] == pitch / 2,
                "SPR2 plane origin/pitch differs"
            );
        }
        ensure!(
            targets.len() == 2
                && targets[0]["sha256"] == sims_fixtures::digest(&data.color)
                && targets[1]["sha256"] == sims_fixtures::digest(&data.depth),
            "native SPR2 planes differ"
        );
        Ok(())
    }
}
