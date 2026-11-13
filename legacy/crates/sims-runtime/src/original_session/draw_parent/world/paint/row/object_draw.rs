//! Resident 4378bc drawing-group selection. Sources are B inputs; later sprite
//! services remain explicit children. The caller borrows the rectangle owner.
use super::*;
use std::collections::BTreeMap;
mod frames;
mod contained;
mod auxiliary;
pub(super) mod raster;
const CATALOG_SOURCE: &str = "0f543e9a84902158f9b0a31457aa73b5a12855bd458f9b11085471add8a72e71";
fn signed(raw:&[u8],at:usize)->i16{i16::from_le_bytes(raw[at..at+2].try_into().unwrap())}
const SOURCE: &str = "3039e6efaf988f5d147fa92cda28524c0e4da63f1475e4b85781a4a5411ff061";
const RESIDENT_CATALOG_SOURCE: &str =
    "b74de110c855f35638f0ce9f1fba9cd0c66b6cf0a7bbb987295aa9e276ccb49c";
const OVERRIDE_CATALOG_SOURCE:&str="36deee3bd8f3a5f980450c18bb34c0484ab951666b5fe5b74600d25d8676d4db";
const OVERRIDE_FAMILY_SOURCE:&str="e84dd53e6defad921aedbde153910a68e10ccf335bf38d352d9434af4b2595b3";
fn is_override_catalog(identity:&str)->bool{identity==OVERRIDE_CATALOG_SOURCE||identity==OVERRIDE_FAMILY_SOURCE}
const DIRECTORY: &str = "f042f68b9f32d7fe96351320812492bc731df7ecee450af9d10c864781b813a9";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct View {
    pointer: u32,
    mask: u32,
    zoom: i32,
    sprites: Vec<(u32, [u32; 8])>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source {
    identity: String,
    pub(super) id: i16,
    pointer: u32,
    definition: sims_vm::Definition,
    graphic_marker: i16,
    dynamic_start: i16,
    #[serde(default,skip_serializing_if="Option::is_none")]
    native_kind:Option<i16>,
    groups: BTreeMap<i32, (u32, Vec<View>)>,
    pub bounds: [i32; 4],
    ui_mode: u32,
    ui_tool: u32,
    view: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frames: Option<frames::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot_vector: Option<(u32,usize,u32)>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    override_flags:Option<[i16;2]>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    visibility:Option<sims_sim::lot_presenter::visibility::State>,
}
impl Source {
    pub fn import(encoded: &[u8], directory: &[u8], sim: &sims_sim::Simulation) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE
                && sims_fixtures::digest(directory) == DIRECTORY,
            "object drawing sources differ"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "object drawing epoch differs"
        );
        let d: serde_json::Value = serde_json::from_slice(directory)?;
        let m = Memory::new(&v)?;
        let dm = Memory::new(&d)?;
        let pointer = v["object_pointer"]
            .as_u64()
            .context("object pointer absent")? as u32;
        Self::from_memory(&m, &dm, pointer, SOURCE, sim)
    }
    fn from_memory(
        m: &Memory,
        dm: &Memory,
        pointer: u32,
        identity: &str,
        sim: &sims_sim::Simulation,
    ) -> Result<Self> {
        let obj = m.read(pointer, 0x18c)?;
        ensure!(dm.read(pointer, 0x18c)? == obj, "object sources disagree");
        let id = i16::from_le_bytes(obj[0xf0..0xf2].try_into()?);
        let definition = sim
            .vm
            .world
            .objects
            .get(&id)
            .and_then(|o| o.definition.as_ref())
            .context("drawing definition unobserved")?
            .clone();
        let resource = m.read(word(&obj, 0x11c), 0xb8)?;
        ensure!(
            definition.source == format!("native-provider:{:08x}", word(&resource, 0)),
            "drawing resource identity differs"
        );
        let def = m.read(word(&obj, 0x118), 0xc0)?;
        ensure!(
            definition.guid == word(&def, 0x1c),
            "drawing definition GUID differs"
        );
        let signed = |b: &[u8], o: usize| i16::from_le_bytes(b[o..o + 2].try_into().unwrap());
        let mut groups = BTreeMap::new();
        ensure!(
            resource[0xb4] & 1 != 0,
            "drawing groups require their loading child"
        );
        let begin = word(&resource, 0x94);
        let end = word(&resource, 0x98);
        ensure!(
            end >= begin && (end - begin) % 4 == 0,
            "drawing group vector invalid"
        );
        for (i, g) in m
            .read(begin, (end - begin) as usize)?
            .chunks_exact(4)
            .enumerate()
        {
            let p = word(g, 0);
            if p == 0 {
                continue;
            }
            let gr = m.read(p, 12)?;
            let b = word(&gr, 0);
            let e = word(&gr, 4);
            ensure!(e >= b && (e - b) % 20 == 0, "drawing view vector invalid");
            let mut views = Vec::new();
            for (n, r) in m.read(b, (e - b) as usize)?.chunks_exact(20).enumerate() {
                let sb = word(r, 0);
                let se = word(r, 4);
                ensure!(se >= sb && (se - sb) % 32 == 0, "sprite vector invalid");
                let sprites = m
                    .read(sb, (se - sb) as usize)?
                    .chunks_exact(32)
                    .enumerate()
                    .map(|(j, s)| (sb + j as u32 * 32, std::array::from_fn(|k| word(s, k * 4))))
                    .collect();
                views.push(View {
                    pointer: b + n as u32 * 20,
                    mask: word(r, 12),
                    zoom: word(r, 16) as i32,
                    sprites,
                });
            }
            groups.insert(i as i32, (p, views));
        }
        let scene = dm.read(word(&dm.read(0x6e5528, 4)?, 0), 0x90)?;
        let ui = word(&dm.read(word(&dm.read(0x6f2a58, 4)?, 0) + 0xb0, 4)?, 0);
        let u = dm.read(ui, 0x6c)?;
        Ok(Self {
            identity: identity.into(),
            id,
            pointer,
            definition,
            graphic_marker: signed(&def, 6),
            dynamic_start: signed(&def, 0x80),
            native_kind:Some(signed(&def,0x12)),
            groups,
            bounds: std::array::from_fn(|i| word(&scene, 0x70 + 4 * i) as i32),
            ui_mode: word(&u, 8),
            ui_tool: word(&u, 0x68),
            view: dm.read(0x09e88960, 0x520)?,
            frames: None,
            slot_vector: None,
            override_flags:if is_override_catalog(identity){let r=m.read(0x6e52d0,4)?;Some([signed(&r,0),signed(&r,2)])}else{None},
            visibility:None,
        })
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (self.identity == SOURCE
                || self.identity == CATALOG_SOURCE
                || self.identity == RESIDENT_CATALOG_SOURCE
                || is_override_catalog(&self.identity))
                && self.id > 0
                && self.pointer != 0
                && self.view.len() == 0x520,
            "object sprite provenance/owner differs"
        );
        ensure!(self.override_flags.is_some()==(is_override_catalog(&self.identity)),"drawing override provenance differs");
        if let Some(v)=&self.visibility{ensure!(v.mask.first_row==0&&v.mask.rows.len()==64,"drawing visibility mask extent differs");}
        if let Some(f) = &self.frames {
            f.validate()?;
        }
        if let Some((begin,count,owner))=self.slot_vector{ensure!(begin!=0&&count>0&&count<=256&&owner!=0,"drawing slot vector metadata differs");}
        Ok(())
    }
    pub fn bind_frames(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(self.frames.is_none(), "sprite frames already bound");
        self.frames = Some(frames::Source::import(bytes)?);
        self.validate()
    }
    pub fn owns(&self, id: i16, pointer: u32, sim: &sims_sim::Simulation) -> bool {
        id == self.id
            && pointer == self.pointer
            && sim
                .vm
                .world
                .objects
                .get(&id)
                .and_then(|o| o.definition.as_ref())
                == Some(&self.definition)
    }
    pub fn compare_visibility_inputs(&self,q:&serde_json::Value)->Result<()>{
        let v=&q["visibility_inputs"];if !v.is_object(){return Ok(());}
        let raw=crate::original_session::completed_animations::bytes(v["view_raw"].as_str().context("visibility view absent")?)?;
        let ui=crate::original_session::completed_animations::bytes(v["ui_raw"].as_str().context("visibility UI absent")?)?;
        ensure!(v["view_pointer"]==0x09e88960u32&&raw.len()==0x520&&ui.len()==0x6c,"visibility input owner differs");
        for range in [0x18..0x1c,0x48..0x49,0x51..0x52]{ensure!(raw[range.clone()]==self.view[range],"visibility control differs from retained view");}
        let mask:Vec<u8>=if let Some(v)=&self.visibility{ensure!(raw[0x49]==v.enabled,"retained visibility enabled differs");v.mask.rows.iter().flat_map(|v|v.to_le_bytes()).collect()}else{ensure!(raw[0x49]==self.view[0x49],"baseline visibility enabled differs");self.view[0x208..0x408].to_vec()};
        ensure!(raw[0x208..0x408]==mask,"visibility mask differs from retained composition");
        ensure!(word(&ui,8)==self.ui_mode&&word(&ui,0x68)==self.ui_tool,"visibility UI mode/tool differs");Ok(())
    }
    pub fn hidden(&self,mut point:[i8;3],rotation:u8,sim:&sims_sim::Simulation)->Result<bool>{
        if self.view[0x51]!=0||self.ui_mode==0&&matches!(self.ui_tool,8|9){return Ok(false);}
        let obj=sim.vm.world.objects.get(&self.id).context("draw object absent")?;
        let data=|n|obj.data.get(&n).copied().context("visibility object variable absent");
        if data(8)?as u16&0x400==0{return Ok(false);}
        ensure!(self.visibility.is_some()||self.view[0x48]!=0||self.view[0x49]==0,"current composed visibility is unbound; baseline mask cannot substitute");
        let level=sim.vm.world.positions.get(&self.id).context("visibility physical level absent")?.level;
        point[2]=i8::try_from(level)?;
        let mask;let(enabled,mask)=if let Some(v)=&self.visibility{(v.enabled!=0,&v.mask)}else{mask=sims_sim::lot_presenter::walls::cutaway::Mask{first_row:0,rows:(0..64).map(|n|u64::from(word(&self.view,0x208+n*8))|u64::from(word(&self.view,0x20c+n*8))<<32).collect()};(self.view[0x49]!=0,&mask)};
        sims_sim::lot_presenter::object_cutaway::hidden(rotation,word(&self.view,0x18)as i32,self.view[0x48]!=0,enabled,mask,point,data(8)?as u16,data(13)?as u16,data(1)?,self.ui_mode==0&&matches!(self.ui_tool,8|9)).map_err(anyhow::Error::msg)
    }
    pub fn new_draw(&self) -> Cursor {
        Cursor {
            stage: if self.override_flags.is_some(){Stage::OverrideEntry}else{Stage::Entry},
            raster: None,
            rectangle: None,
            group: None,
            view: None,
            sprite: None,
            enabled: None,
            frame_count: None,
            metadata: None,
            saved_flags: None,
            saved_light: None,
            light: None,
            box_out: None,
            temp: None,
            index: None,
            slot_index: None,
            slot_child: None,
            override_copied:self.override_flags.map(|_|0),
            positive_slot:None,
            auxiliary_scan:None,
            return_pc:None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Catalog {
    identity: String,
    entries: BTreeMap<i16, Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot_metadata:Option<String>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    slot_records:Option<contained::Slots>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    auxiliary:Option<auxiliary::Source>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    override_catalog:Option<String>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    visibility:Option<sims_sim::lot_presenter::visibility::State>,
}
impl Catalog {
    pub fn import(encoded: &[u8], directory: &[u8], sim: &sims_sim::Simulation) -> Result<Self> {
        ensure!(
            (sims_fixtures::digest(encoded) == CATALOG_SOURCE
                || sims_fixtures::digest(encoded) == RESIDENT_CATALOG_SOURCE
                || is_override_catalog(&sims_fixtures::digest(encoded)))
                && sims_fixtures::digest(directory) == DIRECTORY,
            "tree catalog inputs differ"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        let d: serde_json::Value = serde_json::from_slice(directory)?;
        ensure!(
            v["tick"] == 89073
                && v["executes_target_code"] == false
                && v["directory_sha256"] == DIRECTORY,
            "tree catalog epoch differs"
        );
        let m = Memory::new(&v)?;
        let dm = Memory::new(&d)?;
        let identity=sims_fixtures::digest(encoded);
        let mut entries = BTreeMap::new();
        let mut definitions: BTreeMap<(u32, u32), Source> = BTreeMap::new();
        let mut wrappers: BTreeMap<u32, frames::Source> = BTreeMap::new();
        for o in v["objects"].as_array().context("catalog objects absent")? {
            let pointer = o["pointer"].as_u64().context("catalog pointer absent")? as u32;
            let raw = m.read(pointer, 0x18c)?;
            ensure!(dm.read(pointer, 0x18c)? == raw, "catalog instance differs");
            let id = i16::from_le_bytes(raw[0xf0..0xf2].try_into()?);
            let resource = word(&raw, 0x11c);
            let definition_pointer = word(&raw, 0x118);
            let key = (resource, definition_pointer);
            let live = sim
                .vm
                .world
                .objects
                .get(&id)
                .and_then(|o| o.definition.as_ref())
                .context("catalog live definition absent")?;
            let mut source =
                if let Some(cached) = definitions.get(&key).filter(|s| &s.definition == live) {
                    let mut s = cached.clone();
                    s.id = id;
                    s.pointer = pointer;
                    s
                } else {
                    let s = Source::from_memory(&m, &dm, pointer, &identity, sim)?;
                    definitions.insert(key, s.clone());
                    s
                };
            ensure!(o["id"] == source.id, "catalog ID differs");
            if identity == RESIDENT_CATALOG_SOURCE || is_override_catalog(&identity) {
                ensure!(
                    word(&m.read(word(&raw, 0) + 0x30, 4)?, 0) == if is_override_catalog(&identity) {0x463f4c}else{0x4378bc},
                    "catalog virtual drawing target differs"
                );
            }
            let wrapper = word(&m.read(resource, 0xb8)?, 8);
            let f = if let Some(cached) = wrappers.get(&wrapper) {
                cached.for_instance(&m, pointer)?
            } else {
                let f = frames::Source::from_memory(&v, &m, pointer, &identity)?;
                wrappers.insert(wrapper, f.clone());
                f
            };
            source.frames = Some(f);
            ensure!(
                entries.insert(source.id, source).is_none(),
                "duplicate catalog identity"
            );
        }
        let state = Self {
            identity,
            entries,
            slot_metadata:None,
            slot_records:None,
            auxiliary:None,
            override_catalog:None,
            visibility:None,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.identity == CATALOG_SOURCE || self.identity == RESIDENT_CATALOG_SOURCE ||is_override_catalog(&self.identity),
            "tree catalog provenance differs"
        );
        for s in self.entries.values() {
            s.validate()?;
        }
        if let Some(records)=&self.slot_records{records.validate()?;}
        if let Some(aux)=&self.auxiliary{aux.validate()?;}
        if let Some(id)=&self.slot_metadata{ensure!(id==DIRECTORY,"slot metadata provenance differs");}
        if let Some(id)=&self.override_catalog{ensure!(is_override_catalog(id)&&self.entries.get(&102).is_some_and(|s|&s.identity==id),"drawing override catalog provenance differs");}
        if let Some(v)=&self.visibility{ensure!(v.mask.first_row==0&&v.mask.rows.len()==64,"catalog visibility mask extent differs");}
        Ok(())
    }
    pub fn bind_slots(&mut self,directory:&[u8])->Result<()> {
        ensure!(self.slot_metadata.is_none()&&sims_fixtures::digest(directory)==DIRECTORY,"drawing slot directory differs/already bound");
        let v:serde_json::Value=serde_json::from_slice(directory)?;let m=Memory::new(&v)?;
        for s in self.entries.values_mut(){
            let obj=m.read(s.pointer,0x18c)?;ensure!(i16::from_le_bytes(obj[0xf0..0xf2].try_into()?)==s.id,"slot directory ID differs");
            let begin=word(&obj,0x130);let end=word(&obj,0x134);let cap=word(&obj,0x138);
            ensure!(begin<=end&&end<=cap&&(end-begin)%36==0,"slot directory extent differs");
            let count=(end-begin)/36;
            s.slot_vector=if count>1 {Some((begin,(count-1)as usize,word(&obj,0xdc)))}else{None};s.validate()?;
        }
        self.slot_metadata=Some(DIRECTORY.into());self.validate()
    }
    pub fn bind_override_catalog(&mut self,encoded:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()> {
        let incoming=sims_fixtures::digest(encoded);
        ensure!(is_override_catalog(&incoming),"override catalog differs");
        let already=self.override_catalog.as_ref()==Some(&incoming);
        if already&&self.entries.values().filter(|s|s.identity==incoming).all(|s|s.native_kind.is_some()){return self.validate();}
        ensure!(already||self.override_catalog.is_none()||self.override_catalog.as_deref()==Some(OVERRIDE_CATALOG_SOURCE)&&incoming==OVERRIDE_FAMILY_SOURCE,"override catalog replacement is not a qualified B extension");
        let mut part=Self::import(encoded,directory,sim)?;
        if self.slot_metadata.is_some(){part.bind_slots(directory)?;}
        //Both independently recovered inputs may describe the same B instance.
        //Only provenance changes; reject a differing field before any mutation.
        for(id,entry)in &part.entries{if let Some(old)=self.entries.get(id){let mut old=old.clone();old.identity=entry.identity.clone();if old.native_kind.is_none(){old.native_kind=entry.native_kind;}if let(Some(a),Some(b))=(&old.frames,&entry.frames){ensure!(a.same_snapshot_data(b),"overlapping B sprite cache differs");old.frames=entry.frames.clone();}ensure!(old==*entry,"overlapping B drawing instance differs");}}
        if already{
            //Enrich only previously unknown immutable definition metadata.
            //Retain every hot cache and every other saved field.
            ensure!(part.entries.keys().all(|id|self.entries.contains_key(id)),"retained catalog entry absent");
            for(id,entry)in part.entries{self.entries.get_mut(&id).context("retained catalog entry absent")?.native_kind=entry.native_kind;}
            return self.validate();
        }
        self.entries.extend(part.entries);
        self.override_catalog=Some(incoming);self.validate()
    }
    pub fn compare_slot_call(&self,q:&serde_json::Value)->Result<()>{
        let call=&q["slot_call"];if !call.is_object(){return Ok(());}
        let parent=self.entries.values().find(|s|call["parent"]==s.pointer).context("contained call parent absent")?;
        if let Some(records)=&self.slot_records{
            let (begin,count,_)=parent.slot_vector.context("contained call vector absent")?;
            let address=call["slot"].as_u64().context("contained call slot absent")?as u32;
            ensure!(address>=begin+36&&(address-begin)%36==0,"contained call record offset differs");let index=((address-begin)/36-1)as usize;ensure!(index<count,"contained call index differs");
            records.compare_child(parent.id,index,call["child"].as_i64().context("contained call child absent")?as i16)?;
        }Ok(())
    }
    pub fn bind_auxiliary(&mut self,encoded:&[u8],directory:&[u8])->Result<()>{ensure!(self.auxiliary.is_none(),"auxiliary source already bound");self.auxiliary=Some(auxiliary::Source::import(encoded,directory)?);self.validate()}
    pub fn bind_slot_records(&mut self,encoded:&[u8],directory:&[u8])->Result<()>{ensure!(self.slot_records.is_none(),"contained records already bound");self.slot_records=Some(contained::Slots::import(encoded,directory)?);self.validate()}
    pub fn select(&self, id: i16, pointer: u32, sim: &sims_sim::Simulation) -> Option<Source> {
        self.entries
            .get(&id)
            .filter(|s| s.owns(id, pointer, sim))
            .cloned().map(|mut s|{s.visibility=self.visibility.clone();s})
    }
    pub fn bind_visibility(&mut self,value:sims_sim::lot_presenter::visibility::State)->Result<()>{ensure!(value.mask.first_row==0&&value.mask.rows.len()==64,"current visibility mask extent differs");self.visibility=Some(value);self.validate()}
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    BoundsCall,
    BoundsReturned,
    Visibility,
    Definition,
    GroupCall,
    GroupReturned,
    ViewCall,
    ViewReturned,
    Loop,
    SpriteCall,
    SpriteReturned,
    EnabledCall,
    EnabledReturned,
    FrameCountCall,
    FrameCountReturned,
    MetadataCall,
    MetadataReturned,
    LightGetterCall,
    LightGetterReturned,
    LightingCall,
    LightingReturned,
    MasterCall,
    MasterReturned,
    Transform,
    WrapperCall,
    SpriteRaster,
    SpriteRasterReturned,
    WrapperReturned,
    UnionCall,
    UnionReturned,
    RestoreEntry,
    RestoreCall,
    Restoring,
    RestoreReturned,
    NextSprite,
    AfterSprites,
    LayerCall,
    LayerReturned,
    LayerUnionCall,
    LayerUnionReturned,
    CacheCall,
    CacheReturned,
    ReturnPrep,
    Returned,
    SlotCall,
    SlotExecuting,
    SlotReturned,
    SlotUnionCall,
    SlotUnionReturned,
    AuxiliaryEntry,AuxiliaryScan,AuxiliaryEmptyLoop,AuxiliaryEmptyFinished,
    OverrideEntry,OverrideCall,OverrideAfterBase,OverrideSecond,OverrideCopySetup,OverrideCopy,OverrideReturn,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Cursor {
    stage: Stage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    raster: Option<raster::Job>,
    rectangle: Option<[i32; 4]>,
    group: Option<u32>,
    view: Option<u32>,
    sprite: Option<(u32, [u32; 8])>,
    enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frame_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<[i16; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    saved_flags: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    saved_light: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    light: Option<LightUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    box_out: Option<[i32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    temp: Option<[i32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot_index:Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot_child:Option<sims_sim::lot_presenter::contained_slot::Empty>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    override_copied:Option<u8>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    positive_slot:Option<Box<contained::Positive>>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    return_pc:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    auxiliary_scan:Option<auxiliary::Scan>,
}
impl Cursor {
    pub fn advance(
        &mut self,
        source: &Source,
        sim: &sims_sim::Simulation,
        context: &mut Frame,
        classify: &mut classification::State,
        clip: [i16; 4],
        raster: Option<(&mut raster::Source,&mut sims_sim::lot_presenter::terrain_raster::Data)>,
        catalog:Option<&Catalog>,
        directory:Option<&ordinary_objects::Source>,
        placement:Option<&mut placement::Source>,
        rotation:u8,
    ) -> Result<bool> {
        use self::Stage::*;
        source.validate()?;
        let obj = sim
            .vm
            .world
            .objects
            .get(&source.id)
            .context("draw object absent")?;
        ensure!(
            obj.definition.as_ref() == Some(&source.definition),
            "drawing definition changed while cached"
        );
        let data = |n| {
            obj.data
                .get(&n)
                .copied()
                .context("draw object variable absent")
        };
        self.stage = match self.stage {
            Entry => BoundsCall,
            BoundsCall => {
                self.rectangle = Some(classify.cached_rectangles(source.id)?[0]);
                BoundsReturned
            }
            BoundsReturned => {
                ensure!(
                    !sim.vm
                        .world
                        .refresh
                        .as_ref()
                        .and_then(|r| r.draw.get(&source.id))
                        .context("draw refresh unobserved")?
                        .dynamic,
                    "dynamic draw timer child pending"
                );
                Visibility
            }
            Visibility => if data(34)?!=0 {AfterSprites}else{Definition},
            Definition => {
                if source.graphic_marker==0 {AfterSprites}else{GroupCall}
            }
            GroupCall => {
                self.group = source.groups.get(&i32::from(data(0)?)).map(|g| g.0);
                GroupReturned
            }
            GroupReturned => {
                if self.group.is_none() {
                    //437949 reads the retained definition's native type.
                    //2,8,11 join437c56; other missing-view drawing remains
                    //an explicit child. An old snapshot's absent type is
                    //unknown, not an invisible-object default.
                    if matches!(source.native_kind,Some(2|8|11)){AfterSprites}
                    else{return Ok(false);}
                }else{
                    ViewCall
                }
            }
            ViewCall => {
                let (_, views) = source
                    .groups
                    .values()
                    .find(|g| Some(g.0) == self.group)
                    .context("draw group absent")?;
                let facing =
                    (i32::from(data(1)?).wrapping_add(context.state().words()[7] as i32)) & 7;
                let bit = 1_u32 << facing;
                self.view = views
                    .iter()
                    .find(|v| v.mask & bit != 0 && v.zoom == context.state().words()[8] as i32)
                    .map(|v| v.pointer);
                ViewReturned
            }
            ViewReturned => {
                if self.view.is_none() {
                    if matches!(source.native_kind,Some(2|8|11)){AfterSprites}
                    else{return Ok(false);}
                }else{
                    Loop
                }
            }
            Loop => SpriteCall,
            SpriteCall => {
                self.sprite = source
                    .groups
                    .values()
                    .flat_map(|g| &g.1)
                    .find(|v| Some(v.pointer) == self.view)
                    .and_then(|v| v.sprites.get(self.index.unwrap_or(0)))
                    .copied();
                if self.sprite.is_none() {
                    return Ok(false);
                }
                SpriteReturned
            }
            SpriteReturned => EnabledCall,
            EnabledCall => {
                let id = self.sprite.context("sprite absent")?.1[0] as i16;
                let i = i32::from(id) - i32::from(source.dynamic_start);
                self.enabled = Some(if i >= 0 && (i as usize) < obj.dynamic_sprite_flags.len() {
                    obj.dynamic_sprite_flags[i as usize] != 0
                } else {
                    true
                });
                EnabledReturned
            }
            EnabledReturned => {
                if self.enabled==Some(false){NextSprite}else{ensure!(self.enabled==Some(true),"sprite enable query absent");FrameCountCall}
            }
            FrameCountCall => {
                let Some(f) = &source.frames else {
                    return Ok(false);
                };
                let Some(count) = f.count(self.sprite.context("sprite absent")?.1[0]) else {
                    return Ok(false);
                };
                self.frame_count = Some(count);
                FrameCountReturned
            }
            FrameCountReturned => {
                if self.sprite.context("sprite absent")?.1[1] as i32
                    >= self.frame_count.context("frame count absent")?
                {
                    return Ok(false);
                }
                MetadataCall
            }
            MetadataCall => {
                let (_, s) = self.sprite.context("sprite absent")?;
                self.metadata = Some(
                    source
                        .frames
                        .as_ref()
                        .context("sprite frame source absent")?
                        .metadata(s[0] as i16 as i32 as u32, s[1] as i32)?,
                );
                MetadataReturned
            }
            MetadataReturned => {
                self.saved_flags =
                    Some([0x35, 0x36, 0x38].map(|o| context.render_byte(o).unwrap()));
                let flags = obj.native_flags.context("object native flags unknown")?;
                context
                    .set_render_byte(0x35, (flags & 1) as u8)
                    .map_err(anyhow::Error::msg)?;
                if flags & 8 == 0 {
                    context
                        .set_render_byte(0x36, 0)
                        .map_err(anyhow::Error::msg)?;
                }
                if flags & 16 != 0 {
                    context
                        .set_render_byte(0x38, 1)
                        .map_err(anyhow::Error::msg)?;
                }
                LightGetterCall
            }
            LightGetterCall => {
                if context.render_byte(0x34).map_err(anyhow::Error::msg)? != 0 {
                    return Ok(false);
                }
                self.saved_light = Some(context.state().words()[0x48 / 4]);
                LightGetterReturned
            }
            LightGetterReturned => LightingCall,
            LightingCall => LightingReturned,
            LightingReturned => {
                if data(51)? != 0
                    && source
                        .frames
                        .as_ref()
                        .context("sprite frame source absent")?
                        .kind
                        != 8
                {
                    return Ok(false);
                }
                MasterCall
            }
            MasterCall => MasterReturned,
            MasterReturned => {
                if source
                    .frames
                    .as_ref()
                    .context("sprite frame source absent")?
                    .category
                    == 7
                {
                    return Ok(false);
                }
                Transform
            }
            Transform => {
                let s = self.sprite.context("sprite absent")?.1;
                if s[4..7].iter().any(|b| f32::from_bits(*b) != 0.0) {
                    return Ok(false);
                }
                WrapperCall
            }
            WrapperCall => {
                let s = self.sprite.context("sprite absent")?.1;
                let w = context.state().words();
                let h = i32::from(self.metadata.context("sprite metadata absent")?[2]);
                let origin = [
                    (s[2] as i32).wrapping_add(w[4] as i32),
                    (s[3] as i32).wrapping_add(w[5] as i32).wrapping_sub(h),
                ];
                let Some(r) = source
                    .frames
                    .as_ref()
                    .context("sprite frame source absent")?
                    .clipped_return(s[0] as i16 as i32 as u32, s[1] as i32, origin, clip)?
                else {
                    let Some((raster,_))=raster else{return Ok(false);};
                    let Some(job)=raster.begin(source.frames.as_ref().context("sprite frame source absent")?,s[0]as i16 as i32 as u32,s[1]as i32,origin,clip,w,s[7]as u8)?else{return Ok(false);};self.raster=Some(job);self.stage=SpriteRaster;return Ok(true);
                };
                self.box_out = Some(r);
                self.temp = Some(r);
                WrapperReturned
            }
            SpriteRaster=>{
                let Some((source,data))=raster else{return Ok(false);};let job=self.raster.as_mut().context("SPR2 job absent")?;if !job.advance(source,data)?{return Ok(false);}if job.complete(){SpriteRasterReturned}else{SpriteRaster}
            }
            SpriteRasterReturned=>{let r=self.raster.as_ref().context("SPR2 job absent")?.rectangle();self.box_out=Some(r);self.temp=Some(r);WrapperReturned}
            WrapperReturned => UnionCall,
            UnionCall => {
                self.union(context)?;
                UnionReturned
            }
            UnionReturned => RestoreEntry,
            RestoreEntry => RestoreCall,
            RestoreCall => {
                self.light = Some(LightUpdate::new(
                    self.saved_light.context("saved light absent")?,
                ));
                Restoring
            }
            Restoring => {
                let l = self.light.as_mut().context("light restore child absent")?;
                if !context.advance_light(l).map_err(anyhow::Error::msg)? {
                    return Ok(false);
                }
                if l.complete() {
                    RestoreReturned
                } else {
                    Restoring
                }
            }
            RestoreReturned => {
                for (o, b) in [0x35, 0x36, 0x38]
                    .into_iter()
                    .zip(self.saved_flags.context("saved sprite flags absent")?)
                {
                    context.set_render_byte(o, b).map_err(anyhow::Error::msg)?;
                }
                NextSprite
            }
            NextSprite=>{
                let next = self.index.unwrap_or(0) + 1;
                self.index = Some(next);
                let len = source
                    .groups
                    .values()
                    .flat_map(|g| &g.1)
                    .find(|v| Some(v.pointer) == self.view)
                    .context("drawing view absent")?
                    .sprites
                    .len();
                if next < len { SpriteCall } else { AfterSprites }
            }
            AfterSprites => {
                let f=source.frames.as_ref().context("sprite frame source absent")?;
                if f.kind!=2&&!obj.slots.is_empty(){
                    let (_,count,_)=source.slot_vector.context("contained-slot vector owner is unbound")?;
                    ensure!(count==obj.slots.len(),"contained-slot resize requires its current native vector owner");
                    self.slot_index=Some(0);SlotCall
                }else if f.kind!=2{AuxiliaryEntry}else{LayerCall}
            }
            SlotCall=>{
                let at=self.slot_index.context("contained-slot index absent")?;
                if let Some(id)=obj.slots.get(at).context("contained slot absent")?{
                    let Some(catalog)=catalog else{return Ok(false);};
                    let Some(records)=&catalog.slot_records else{return Ok(false);};
                    let local=records.get(source.id,at,*id)?;
                    self.positive_slot=Some(Box::new(contained::Positive::new(*id,local)));self.slot_child=None;
                }else{self.slot_child=Some(sims_sim::lot_presenter::contained_slot::Empty::new(0).map_err(anyhow::Error::msg)?);self.positive_slot=None;}
                SlotExecuting
            }
            SlotExecuting=>{
                if let Some(c)=&mut self.positive_slot {
                    if !c.advance(source,sim,context,classify,clip,raster,catalog,directory,placement,rotation)?{return Ok(false);}
                    self.stage=if c.complete(){SlotReturned}else{SlotExecuting};return Ok(true);
                }
                let c=self.slot_child.as_mut().context("contained child absent")?;
                ensure!(c.advance().map_err(anyhow::Error::msg)?,"contained child already returned");
                if c.complete(){SlotReturned}else{SlotExecuting}
            }
            SlotReturned=>{self.temp=if let Some(c)=&self.positive_slot{c.result()}else{self.slot_child.as_ref().and_then(|c|c.result())};SlotUnionCall}
            SlotUnionCall=>{let r=self.temp.context("contained union input absent")?;let old=self.rectangle.context("contained union owner absent")?;self.rectangle=Some([old[0].min(r[0]),old[1].min(r[1]),old[2].max(r[2]),old[3].max(r[3])]);SlotUnionReturned}
            SlotUnionReturned=>{
                let next=self.slot_index.context("contained-slot index absent")?+1;
                if next<obj.slots.len(){self.slot_index=Some(next);self.slot_child=None;self.positive_slot=None;SlotCall}
                else{self.slot_index=None;self.slot_child=None;self.positive_slot=None;AuxiliaryEntry}
            }
            AuxiliaryEntry=>{if let Some(aux)=catalog.and_then(|c|c.auxiliary.as_ref()){let owner=aux.owner(source.id,source.pointer)?;self.auxiliary_scan=Some(auxiliary::Scan::new(owner));AuxiliaryScan}else if source.frames.as_ref().context("auxiliary frame source absent")?.auxiliary_empty{AuxiliaryEmptyLoop}else{return Ok(false);}},
            AuxiliaryEmptyLoop=>AuxiliaryEmptyFinished,AuxiliaryEmptyFinished=>LayerCall,
            AuxiliaryScan=>{let aux=catalog.and_then(|c|c.auxiliary.as_ref()).context("auxiliary source absent")?;let owner=aux.owner(source.id,source.pointer)?;let scan=self.auxiliary_scan.as_mut().context("auxiliary scan absent")?;if !scan.advance(owner)?{return Ok(false);}if scan.complete(){self.auxiliary_scan=None;LayerCall}else{AuxiliaryScan}},
            LayerCall => {
                if source
                    .frames
                    .as_ref()
                    .context("sprite frame source absent")?
                    .layer_flags
                    != [0; 4]
                {
                    return Ok(false);
                }
                self.temp = Some([i32::MAX, i32::MAX, i32::MIN, i32::MIN]);
                LayerReturned
            }
            LayerReturned => LayerUnionCall,
            LayerUnionCall => {
                self.union(context)?;
                LayerUnionReturned
            }
            LayerUnionReturned => {
                let r = self.rectangle.context("draw rectangle absent")?;
                if r[0] < r[2] && r[1] < r[3] {
                    CacheCall
                } else {
                    ReturnPrep
                }
            }
            CacheCall => {
                classify.store_cached_rectangle(
                    source.id,
                    1,
                    self.rectangle.context("draw rectangle absent")?,
                )?;
                CacheReturned
            }
            CacheReturned => {
                if obj.native_keep_dynamic != Some(0)
                    || data(19)? == 2
                    || source
                        .frames
                        .as_ref()
                        .context("sprite frame source absent")?
                        .kind
                        == 2
                {
                    return Ok(false);
                }
                ReturnPrep
            }
            ReturnPrep => if self.override_copied.is_some(){OverrideAfterBase}else{Returned},
            OverrideEntry=>OverrideCall,
            OverrideCall=>Entry,
            OverrideAfterBase=>{if source.override_flags.context("override flags absent")?[0]!=0{return Ok(false);}OverrideSecond},
            OverrideSecond=>{if source.override_flags.context("override flags absent")?[1]!=0{return Ok(false);}OverrideCopySetup},
            OverrideCopySetup=>OverrideCopy,
            OverrideCopy=>{let n=self.override_copied.context("override copy absent")?+1;ensure!(n<=4,"override copy overflow");self.override_copied=Some(n);if n==4{OverrideReturn}else{OverrideCopy}},
            OverrideReturn=>Returned,
            Returned => return Ok(false),
        };
        Ok(true)
    }
    fn union(&mut self, context: &Frame) -> Result<()> {
        let mut r = self.temp.context("draw union input absent")?;
        if r[0] < r[2] && r[1] < r[3] {
            let w = context.state().words();
            for i in 0..4 {
                r[i] = r[i].wrapping_sub(w[0x3c / 4 + i % 2] as i32);
            }
            self.temp = Some(r);
            let old = self.rectangle.context("draw union owner absent")?;
            self.rectangle = Some([
                old[0].min(r[0]),
                old[1].min(r[1]),
                old[2].max(r[2]),
                old[3].max(r[3]),
            ]);
        }
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Returned
    }
    pub fn result(&self) -> Option<[i32; 4]> {
        self.complete().then_some(self.rectangle).flatten()
    }
    pub fn pc(&self) -> u32 {
        use self::Stage::*;
        match self.stage {
            Entry => 0x4378bc,
            BoundsCall => 0x4378d3,
            BoundsReturned => 0x4378d8,
            Visibility => 0x4378f5,
            Definition => 0x437907,
            GroupCall => 0x437922,
            GroupReturned => 0x437927,
            ViewCall => 0x43793d,
            ViewReturned => 0x437942,
            Loop => 0x4379b7,
            SpriteCall => 0x4379d5,
            SpriteReturned => 0x4379da,
            EnabledCall => 0x4379ea,
            EnabledReturned => 0x4379ef,
            FrameCountCall => 0x437a0b,
            FrameCountReturned => 0x437a10,
            MetadataCall => 0x437a76,
            MetadataReturned => 0x437a7b,
            LightGetterCall => 0x437ab3,
            LightGetterReturned => 0x437ab8,
            LightingCall => 0x437abd,
            LightingReturned => 0x437ac2,
            MasterCall => 0x437ad3,
            MasterReturned => 0x437ad8,
            Transform => 0x437af2,
            WrapperCall => 0x437b50,
            SpriteRaster=>self.raster.as_ref().filter(|r|r.at_entry()).map_or(0,|r|r.entry_pc()),
            SpriteRasterReturned=>self.raster.as_ref().map_or(0,|r|r.return_pc()),
            WrapperReturned => 0x437b55,
            UnionCall => 0x437b68,
            UnionReturned => 0x437b6d,
            RestoreEntry => 0x437c1c,
            RestoreCall => 0x437c25,
            Restoring => 0x000000,
            RestoreReturned => 0x437c2a,
            NextSprite=>0x437c3f,
            AfterSprites => 0x437c56,
            AuxiliaryEntry=>0x437ce8,AuxiliaryEmptyLoop=>0x437cee,AuxiliaryEmptyFinished=>0x437d2b,AuxiliaryScan=>self.auxiliary_scan.as_ref().map_or(0,|c|c.pc()),
            LayerCall => 0x437d3d,
            LayerReturned => 0x437d40,
            LayerUnionCall => 0x437d53,
            LayerUnionReturned => 0x437d58,
            CacheCall => 0x437d73,
            CacheReturned => 0x437d78,
            ReturnPrep => 0x437f6a,
            Returned => self.return_pc.unwrap_or(0x4aebbb),
            SlotCall=>0x437cbd,
            SlotExecuting=>if let Some(c)=&self.positive_slot{c.pc()}else{self.slot_child.as_ref().map_or(0,|c|c.pc())},
            SlotReturned=>0x437cc2,
            SlotUnionCall=>0x437cd1,
            SlotUnionReturned=>0x437cd6,
            OverrideEntry=>0x463f4c,OverrideCall=>0x463f5f,OverrideAfterBase=>0x463f64,OverrideSecond=>0x463fc3,OverrideCopySetup=>0x46409c,
            OverrideCopy=>0x4640a4+u32::from(self.override_copied.unwrap_or(0)),OverrideReturn=>0x4640ac,
        }
    }
    pub fn compare_raster(&self,source:&raster::Source,q:&serde_json::Value,data:&sims_sim::lot_presenter::terrain_raster::Data)->Result<()>{if let Some(c)=&self.positive_slot{if let Some(child)=c.drawing(){return child.compare_raster(source,q,data);}}self.raster.as_ref().context("SPR2 child absent")?.compare(source,q,data)}
    pub fn matches_observation(&self,q:&serde_json::Value)->Result<bool>{if self.stage==Stage::SlotExecuting{if let Some(c)=&self.positive_slot{return c.matches_observation(q);}}Ok(self.pc()==q["pc"].as_u64().unwrap_or(0)as u32)}
    pub(super) fn raster_store_progress(&self)->Option<(usize,usize)>{self.raster.as_ref()?.store_progress()}
    pub fn compare_auxiliary(&self,q:&serde_json::Value,source:&Source,catalog:&Catalog)->Result<()>{
        if self.stage==Stage::SlotExecuting{if let Some(c)=&self.positive_slot{c.compare_auxiliary(q,catalog)?;}}
        if matches!(self.stage,Stage::AuxiliaryEntry|Stage::AuxiliaryScan|Stage::AuxiliaryEmptyLoop|Stage::AuxiliaryEmptyFinished){
            // Old retained contexts prove emptiness, but not native vector
            // addresses. They cannot satisfy a newly requested live readback.
            if let Some(aux)=&catalog.auxiliary{let owner=aux.owner(source.id,source.pointer)?;if let Some(scan)=&self.auxiliary_scan{scan.compare(q,owner,source.id)?;}else{auxiliary::compare_owner(q,owner,source.id)?;}}
            else{ensure!(source.frames.as_ref().is_some_and(|f|f.auxiliary_empty)&&!q["auxiliary_vector"].is_object(),"native auxiliary vector owner unobserved");}
        }Ok(())
    }
    pub fn compare(&self, q: &serde_json::Value,source:&Source) -> Result<()> {
        if let Some(c)=&self.positive_slot{if self.stage==Stage::SlotExecuting{return c.compare(q,source);}}
        let pc = self.pc();
        ensure!(q["pc"] == pc, "object draw boundary differs");
        let eax = q["registers"]["eax"]
            .as_u64()
            .context("native EAX absent")? as u32;
        match self.stage {
            Stage::GroupReturned => {
                ensure!(eax == self.group.unwrap_or(0), "drawing group differs")
            }
            Stage::ViewReturned => ensure!(eax == self.view.unwrap_or(0), "drawing view differs"),
            Stage::SpriteReturned => ensure!(
                eax == self.sprite.map_or(0, |s| s.0),
                "drawing sprite differs"
            ),
            Stage::EnabledReturned => ensure!(
                Some(eax & 255 != 0) == self.enabled,
                "sprite enabled differs"
            ),
            Stage::FrameCountReturned => ensure!(
                Some(eax as i32) == self.frame_count,
                "sprite frame count differs"
            ),
            _ => {}
        }
        if self.stage == Stage::Returned {
            return Ok(());
        }
        if matches!(self.stage,Stage::OverrideEntry|Stage::OverrideCall|Stage::OverrideAfterBase|Stage::OverrideSecond|Stage::OverrideCopySetup|Stage::OverrideCopy|Stage::OverrideReturn){
            let row_bp=q["ordinary_frame"]["bp"].as_u64().context("override parent BP absent")?as u32;
            let regs=&q["registers"];let wrapper_bp=row_bp-0x244;
            let outer=matches!(self.stage,Stage::OverrideEntry|Stage::OverrideReturn);
            ensure!(regs["ebp"]==if outer{row_bp}else{wrapper_bp}&&regs["esp"]==if outer{row_bp-0x240}else{wrapper_bp-if self.stage==Stage::OverrideCall{0x54}else{0x4c}},"drawing override stack lifetime differs");
            let flags=crate::original_session::completed_animations::bytes(q["override_flags"].as_str().context("drawing override flags absent")?)?;
            ensure!(flags.len()==4&&source.override_flags==Some([signed(&flags,0),signed(&flags,2)]),"drawing override flags differ");
            if self.stage==Stage::OverrideEntry{
                let c=&q["override_call"];ensure!(c["parent"]==source.pointer&&c["context"]==q["context"]["pointer"]&&c["caller"]==0x4aebbb_u32,"drawing override call differs");
                ensure!(c["output"]==row_bp-0x1cc,"drawing override output address differs");
            }else{
                let f=&q["override_frame"];let bp=f["bp"].as_u64().context("drawing override BP absent")?as u32;
                let raw=crate::original_session::completed_animations::bytes(f["raw"].as_str().context("drawing override frame absent")?)?;
                ensure!(f["address"]==bp-0x40&&raw.len()==0x54&&word(&raw,0x48)==f["output"]&&word(&raw,0x4c)==q["context"]["pointer"].as_u64().context("drawing override context absent")?as u32,"drawing override frame arguments differ");
                ensure!(bp==wrapper_bp&&word(&raw,0x40)==row_bp&&word(&raw,0x44)==0x4aebbb&&word(&raw,0x48)==row_bp-0x1cc,"drawing override frame owner differs");
                if self.stage!=Stage::OverrideCall{
                    let local=std::array::from_fn(|n|word(&raw,0x30+n*4)as i32);
                    ensure!(self.rectangle==Some(local),"drawing override base result differs");
                    let output=crate::original_session::completed_animations::bytes(f["output_raw"].as_str().context("drawing override output absent")?)?;
                    for n in 0..usize::from(self.override_copied.context("drawing override copy cursor absent")?){ensure!(word(&output,n*4)as i32==local[n],"drawing override ordered output copy differs");}
                }
                let copied=u32::from(self.override_copied.context("override copy absent")?);
                if matches!(self.stage,Stage::OverrideCopy|Stage::OverrideReturn){ensure!(regs["eax"]==word(&raw,0x48),"override return pointer differs");}
                else{ensure!(regs["eax"]==bp-0x10,"override base output pointer differs");}
                if self.stage==Stage::OverrideCopy{ensure!(regs["esi"]==bp-0x10+copied*4&&regs["edi"]==word(&raw,0x48)+copied*4,"override copy cursors differ");}
            }
            return Ok(());
        }
        if !matches!(self.stage, Stage::Entry | Stage::BoundsCall) {
            let f = &q["draw_frame"];
            let bp = f["bp"].as_u64().context("draw BP absent")? as u32;
            let raw = crate::original_session::completed_animations::bytes(
                f["raw"].as_str().context("draw frame bytes absent")?,
            )?;
            ensure!(
                f["address"] == bp - 0x128 && raw.len() == 0x150,
                "draw frame extent differs"
            );
            let rectangle = std::array::from_fn(|i| word(&raw, 0x128 - 0x38 + 4 * i) as i32);
            ensure!(
                self.rectangle == Some(rectangle),
                "draw retained rectangle differs"
            );
            if matches!(self.stage,Stage::SlotCall|Stage::SlotExecuting|Stage::SlotReturned){
                let (begin,count,owner)=source.slot_vector.context("contained slot owner absent")?;
                let index=self.slot_index.context("contained slot index absent")?;
                ensure!(index<count,"contained index outside original vector");
                let slot=begin+36*(index as u32+1);let output=bp-0x58;
                let call=&q["slot_call"];
                if call.is_object(){ensure!(call["parent"]==source.pointer&&call["output"]==output&&call["context"]==q["context"]["pointer"]&&call["slot"]==slot,"original contained-slot arguments differ");}
                if q["contained_vector"].is_object(){let v=&q["contained_vector"];ensure!(v["parent"]==source.pointer&&v["begin"]==begin&&v["end"]==begin+36*(count as u32+1),"original contained-slot vector differs");}
                if let Some(c)=&self.slot_child {
                    c.validate().map_err(anyhow::Error::msg)?;
                    if q["slot_frame"].is_object()&&c.initialized_locals(){
                        let sf=&q["slot_frame"];let sr=crate::original_session::completed_animations::bytes(sf["raw"].as_str().context("contained frame absent")?)?;
                        ensure!(sr.len()==0x70&&sf["address"]==sf["bp"].as_u64().context("contained BP absent")?-0x58,"contained frame extent differs");
                        ensure!(word(&sr,0x60)==output&&word(&sr,0x64)==q["context"]["pointer"].as_u64().context("contained context absent")?as u32&&word(&sr,0x68)==slot,"contained frame arguments differ");
                        ensure!(std::array::from_fn::<_,4,_>(|n|word(&sr,0x20+n*4)as i32)==sims_sim::lot_presenter::contained_slot::EMPTY,"contained local rectangle differs");
                    }
                    if q["slot_lookup"].is_object(){ensure!(q["slot_lookup"]["owner"]==owner&&q["slot_lookup"]["id"]==c.child()&&q["slot_lookup"]["caller"]==0x43800du32,"contained lookup differs");}
                    if c.lookup_returned(){ensure!(eax==0,"original contained lookup was non-null");}
                    for(n,value)in c.output_words().iter().enumerate(){if let Some(value)=value{ensure!(word(&raw,0xd0+n*4)as i32==*value,"contained output store differs");}}
                }
            }
            if matches!(self.stage,Stage::SlotUnionCall|Stage::SlotUnionReturned){ensure!(self.temp==Some(std::array::from_fn(|n|word(&raw,0x104+n*4)as i32)),"contained caller result copy differs");}
            if matches!(
                self.stage,
                Stage::EnabledCall | Stage::EnabledReturned | Stage::FrameCountCall
            ) {
                let (_, sprite) = self.sprite.context("sprite absent")?;
                ensure!(
                    word(&raw, 0x128 + 0xc) == sprite[0] && word(&raw, 0x128 - 0x28) == sprite[1],
                    "draw sprite/frame locals differ"
                );
            }
        }
        Ok(())
    }
}
