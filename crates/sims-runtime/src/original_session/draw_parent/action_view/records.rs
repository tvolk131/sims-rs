//! Owned caption allocations and bitmap wrappers for full action-record erase.
//! Rust releases its own caption storage. Native allocator freelist bytes and
//! image deleting-destructor internals are not substituted with inferred state.
use super::*;
use sims_vm::scheduler::clock::draw::actions::clear;
mod constructor;
mod services;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Resources {
    backing: Vec<u8>,
    image_header: Vec<u8>,
    pub(super) images: BTreeMap<u32, u32>,
    allocations: BTreeMap<u32, Vec<u8>>,
    freed: BTreeMap<u32, u32>,
    clear: clear::Frame,
    cleared: bool,
    #[serde(default)]
    constructor: Option<constructor::Storage>,
    #[serde(default)]
    services:Option<services::Services>,
}
pub(super) fn import(source: &[u8], records: &[actions::Record]) -> Result<Resources> {
    ensure!(
        sims_fixtures::digest(source)
            == "f5379ff55bd3148d0bd123bbc1e78911c7bdea2ccab86640053a5cd297426961",
        "record resource source differs"
    );
    let v: serde_json::Value = serde_json::from_slice(source)?;
    ensure!(
        v["schema"] == 1
            && v["tick"] == 89073
            && v["executes_target_code"] == false
            && v["native_parity_credit"] == 0,
        "record resource source epoch differs"
    );
    let m = Memory::new(&v)?;
    for (p, n, h) in [
        (
            0x4d4032,
            72,
            "e53715fde1b757da6a3d75306e10bab528161077e8583602b73dcdaffc5299c2",
        ),
        (
            0x4e8835,
            54,
            "2cf84ae20dee59e5c8ae59bdd27f14e6f4576638af65637ce471c5e6a8b46b75",
        ),
        (
            0x4016f8,
            25,
            "cb9dea9d985b385e9a1537751e7f58c10537c659283041dd6088a26160610e2d",
        ),
        (
            0x5a5f3a,
            19,
            "d1f7a8bb87198fbb49f97436c061efbabf421f0d95bfcd1c440aaf7d40e5e9cb",
        ),
        (
            0x5e369b,
            21,
            "9b07365b8d4f9fa361765150104dd530d2d667346b780ec30924d52ad66fb76e",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(p, n)?) == h,
            "record resource instruction source differs"
        );
    }
    let h = m.read(0x6f0f18, 12)?;
    let begin = word(&h, 0);
    let end = word(&h, 4);
    let cap = word(&h, 8);
    ensure!(
        [begin, end, cap] == [0x0ebdd378, 0x0ebdd394, 0x0ebdd3b0],
        "record resource vector differs"
    );
    let backing = m.read(begin, (cap - begin) as usize)?;
    ensure!(
        records.concat() == backing[..(end - begin) as usize],
        "record resource rows differ from caller owners"
    );
    let record = &backing[..28];
    let image = word(record, 4);
    let allocation = word(record, 16);
    let limit = word(record, 24);
    let image_header = m.read(image, 100)?;
    let resources = Resources {
        backing,
        image_header: image_header.clone(),
        images: BTreeMap::from([(image, word(&image_header, 0x58))]),
        allocations: BTreeMap::from([(
            allocation,
            m.read(allocation, (limit - allocation) as usize)?,
        )]),
        freed: BTreeMap::new(),
        clear: clear::Frame::at_entry(),
        cleared: false,
        constructor: None,
        services:None,
    };
    resources.validate(records)?;
    Ok(resources)
}
impl Resources {
    #[cfg(test)]
    pub(super) fn image_counts(&self)->&BTreeMap<u32,u32> {&self.images}
    pub(super) fn bind_services(&mut self,code:&[u8],object:&[u8],pixels:&[u8])->Result<()> {
        ensure!(self.services.is_none() && self.constructor.is_some() && !self.cleared,"record service binding already executing or constructor absent");
        self.services=Some(services::import(code,object,pixels,&self.backing)?);Ok(())
    }
    pub(super) fn build(&mut self,simulation:&sims_sim::Simulation,actor:sims_vm::ObjectId,request:&actions::collect::Request)->Result<Option<(actions::Record,bool)>> {
        self.services.as_mut().map(|services|services.build(&mut self.images,simulation,actor,request)).transpose()
    }
    pub(super) fn copy_record(&mut self,simulation:&mut sims_sim::Simulation,program:&sims_vm::Program,actor:sims_vm::ObjectId,node:u32,source:&actions::Record,index:usize)->Result<Option<actions::Record>> {
        if self.services.is_none(){return Ok(None);}
        let start=index.checked_mul(28).ok_or_else(||anyhow::anyhow!("record output extent overflow"))?;
        ensure!(start+28<=self.backing.len(),"record vector growth requires its original allocation caller");
        let mut raw:actions::Record=self.backing[start..start+28].try_into().unwrap();
        let mut host=services::Copy{strings:self.constructor.as_mut().ok_or_else(||anyhow::anyhow!("record caption storage absent"))?,counts:&mut self.images,simulation,program,actor,node};
        actions::record::copy(&mut raw,source,&mut host).map_err(anyhow::Error::msg)?;
        self.backing[start..start+28].copy_from_slice(&raw);Ok(Some(raw))
    }
    pub(super) fn destroy_temporary(&mut self,request:&actions::collect::Request)->Result<Option<actions::Record>> {
        if self.services.is_none(){return Ok(None);}
        let mut record=match request {actions::collect::Request::DestroyString{record}|actions::collect::Request::DestroyImage{record}=>*record,_=>return Ok(None)};
        if matches!(request,actions::collect::Request::DestroyString{..}) {
            let p=actions::record::word(&record,16);let cap=actions::record::word(&record,24);
            self.constructor.as_mut().ok_or_else(||anyhow::anyhow!("temporary caption storage absent"))?.free(p,cap.wrapping_sub(p))?;
        } else {
            let image=actions::record::word(&record,4);if image!=0 {services::release(&mut self.images,image).map_err(anyhow::Error::msg)?;}
            record[4..8].fill(0);
        }
        Ok(Some(record))
    }
    pub(super) fn bind_constructor(&mut self, source: &[u8]) -> Result<()> {
        ensure!(self.constructor.is_none() && !self.cleared, "record constructor binding already started");
        self.constructor = Some(constructor::import(source)?);
        Ok(())
    }
    pub(super) fn construct(&mut self) -> Result<Option<actions::Record>> {
        self.constructor.as_mut().map(constructor::Storage::construct).transpose()
    }
    pub(super) fn image_sizes(&self)->Result<BTreeMap<u32,[i32;2]>> {
        let mut sizes=self.services.as_ref().ok_or_else(||anyhow::anyhow!("publication image service absent"))?.sizes();
        sizes.insert(0x0bb6d5e0,[word(&self.image_header,0x1c)as i32,word(&self.image_header,0x20)as i32]);Ok(sizes)
    }
    pub(super) fn validate_temporary(&self, temporary:Option<&actions::Record>,output:&[actions::Record],window_image:u32,adjustments:&[(u32,i32)])->Result<()> {
        if let Some(constructor)=&self.constructor {
            constructor.validate_record(temporary,self.services.is_none())?;
            constructor.validate_links(output,temporary)?;
        }
        if self.services.is_some() {
            if self.cleared {ensure!(output.iter().all(|r|actions::record::word(r,16)>=0xf0000000),"new record retains an unowned source caption");}
            let mut expected=BTreeMap::<u32,u32>::new();
            if window_image!=0 {expected.insert(window_image,1);}
            for row in output.iter().chain(temporary) {
                let image=actions::record::word(row,4);
                if image!=0 {*expected.entry(image).or_default()+=1;}
            }
            for &(image,delta) in adjustments {if image!=0 {let count=expected.entry(image).or_default();*count=count.checked_add_signed(delta).ok_or_else(||anyhow::anyhow!("image wrapper adjustment underflow"))?;}}
            expected.retain(|_,n|*n!=0);
            let actual:BTreeMap<_,_>=self.images.iter().filter_map(|(&p,&n)|(n!=0).then_some((p,n))).collect();
            ensure!(expected==actual,"record bitmap counts differ from retained wrappers");
        }
        Ok(())
    }
    pub(super) fn cleared(&self) -> bool {
        self.cleared
    }
    pub(super) fn pending(&self) -> Option<clear::Request> {
        self.clear.pending()
    }
    pub(super) fn validate(&self, records: &[actions::Record]) -> Result<()> {
        if let Some(constructor) = &self.constructor {constructor.validate()?;}
        if let Some(services)=&self.services {
            services.validate()?;
            let mut expected=services.backing().to_vec();
            let rows=records.concat();ensure!(rows.len()<=expected.len(),"record vector exceeds source allocation");
            if self.cleared && rows.is_empty() {expected[4..8].fill(0);} else {expected[..rows.len()].copy_from_slice(&rows);}
            ensure!(expected==self.backing,"retained record backing differs from live rows");
        }
        ensure!(
            self.backing.len() == 56
                && self.image_header.len() == 100
                && word(&self.image_header, 0) == 0x673d4c,
            "record resource backing/class differs"
        );
        let mut original_backing = self.services.as_ref().map_or_else(||self.backing.clone(),|s|s.backing().to_vec());
        original_backing[4..8].copy_from_slice(&0x0bb6d5e0u32.to_le_bytes());
        ensure!(
            sims_fixtures::digest(&original_backing)
                == "caedb54a2919e60c94a7ad199e585586dd9248b16c2a8ff1bc6b2fa9b3f07bc7"
                && sims_fixtures::digest(&self.image_header)
                    == "9c79b1741d94750d258862d914c81db71b0c41ee6d5d00cee485ff7eefa67fb4",
            "unaccessed record resource bytes differ"
        );
        ensure!(
            (self.services.is_some() || self.images.len() == 1)
                && self
                    .images
                    .get(&0x0bb6d5e0)
                    .is_some_and(|n| matches!(*n, 1 | 2) || *n==0&&self.services.is_some()),
            "record resource reference profile differs"
        );
        let active = self.allocations.get(&0x0ed9a8d0);
        ensure!(
            self.allocations.len() == usize::from(active.is_some())
                && self.freed.len() == usize::from(active.is_none()),
            "record caption allocation owner differs"
        );
        if let Some(bytes) = active {
            ensure!(
                bytes == b"Read a Book\0" && self.freed.is_empty(),
                "record caption source bytes differ"
            );
        } else {
            ensure!(
                self.freed.get(&0x0ed9a8d0) == Some(&12),
                "record caption free receipt differs"
            );
        }
        if self.cleared {
            ensure!(
                self.clear.complete()
                    && (self.images[&0x0bb6d5e0] == 1 || self.services.is_some() && self.images[&0x0bb6d5e0]==0)
                    && active.is_none()
                    && (records.is_empty() && word(&self.backing,4)==0 || self.services.is_some() && !records.is_empty()),
                "completed record erase retains resources/rows"
            );
        } else {
            self.clear
                .validate(records, &self.images)
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    pub(super) fn advance(&mut self, records: &mut [actions::Record]) -> Result<bool> {
        for _ in 0..8194 {
            let Some(request) = self
                .clear
                .advance(records, &mut self.images)
                .map_err(anyhow::Error::msg)?
            else {
                let raw = records.concat();
                ensure!(
                    raw.len() <= self.backing.len(),
                    "record erase exceeded backing"
                );
                self.backing[..raw.len()].copy_from_slice(&raw);
                self.cleared = true;
                return Ok(true);
            };
            match request {
                clear::Request::FreeString { pointer, bytes, .. } => {
                    let storage = self.allocations.get(&pointer).ok_or_else(|| {
                        anyhow::anyhow!("record caption allocation absent or already freed")
                    })?;
                    ensure!(
                        storage.len() == bytes as usize,
                        "record caption free size differs"
                    );
                    // Equivalent owned Rust allocation lifetime. This is not
                    // an invented result for the native small-block freelist.
                    self.allocations.remove(&pointer);
                    self.freed.insert(pointer, bytes);
                }
                clear::Request::DestroyImage { .. } => return Ok(false),
            }
            self.clear
                .acknowledge(records, &self.images, request)
                .map_err(anyhow::Error::msg)?;
        }
        anyhow::bail!("record resource work bound exceeded")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    #[test]
    #[ignore = "private frozen resources and original C8 image headers"]
    fn native_clear_resources_and_pending_free_survive_binary_restore() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source =
            std::fs::read(root.join(
                "fixtures/private/goth-action-refresh-20261031-first/action-source-first.json",
            ))?;
        let resources =
            std::fs::read(root.join(
                "fixtures/private/goth-action-records-20261031-first/record-source-first.json",
            ))?;
        let mut owner = super::super::import(&source)?;
        owner.bind_records(&resources)?;
        let mut controls = state::State::constructed(0);
        controls.mode = 2;
        // Stop the actual nested destructor before freeing its owned string.
        for _ in 0..3 {
            let request = owner
                .frame
                .advance(&mut owner.state)
                .map_err(anyhow::Error::msg)?
                .unwrap();
            let reply = match request {
                actions::Request::Mode => actions::Reply::Value(2),
                actions::Request::Parent { window } => {
                    actions::Reply::Value(word(&owner.state.headers[&window], 0x40))
                }
                actions::Request::ClearRecords => break,
                _ => anyhow::bail!("unexpected original prefix"),
            };
            owner
                .frame
                .acknowledge(&mut owner.state, request, reply)
                .map_err(anyhow::Error::msg)?;
        }
        assert_eq!(owner.pending(), Some(&actions::Request::ClearRecords));
        let resources = owner.resources.as_mut().unwrap();
        let pending = resources
            .clear
            .advance(&mut owner.state.records, &mut resources.images)
            .map_err(anyhow::Error::msg)?;
        assert_eq!(
            pending,
            Some(clear::Request::FreeString {
                record: 0,
                pointer: 0x0ed9a8d0,
                bytes: 12
            })
        );
        assert_eq!(resources.images[&0x0bb6d5e0], 2);
        let wire = postcard::to_allocvec(&owner)?;
        let mut restored: super::super::Owner = postcard::from_bytes(&wire)?;
        assert_eq!(owner, restored);
        assert_eq!(
            owner.advance(&controls)?,
            Some(actions::Request::SelectedPerson)
        );
        assert_eq!(
            restored.advance(&controls)?,
            Some(actions::Request::SelectedPerson)
        );
        assert_eq!(owner, restored);
        assert!(restored.state.records.is_empty());
        let resources = restored.resources.as_ref().unwrap();
        assert!(resources.allocations.is_empty());
        assert_eq!(resources.freed[&0x0ed9a8d0], 12);
        assert_eq!(resources.images[&0x0bb6d5e0], 1);
        assert_eq!(word(&resources.backing, 4), 0);
        assert_eq!(restored.vector_headers[2][0], restored.vector_headers[2][1]);
        let encoded = std::fs::read(root.join(
            "fixtures/private/goth-action-image-witness-20261031-first/action-image-trace.jsonl.gz",
        ))?;
        let mut raw = String::new();
        flate2::read::GzDecoder::new(encoded.as_slice()).read_to_string(&mut raw)?;
        let rows: Vec<serde_json::Value> = raw
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let after = rows
            .iter()
            .find(|r| r["kind"] == "same_stop_outer_readback" && r["pc"] == 0x4d29d0u32)
            .unwrap();
        let bitmap = after["action_image_headers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["address"] == 0x0bb6d5e0u32)
            .unwrap();
        let h =
            crate::original_session::completed_animations::bytes(bitmap["raw"].as_str().unwrap())?;
        assert_eq!(resources.images[&0x0bb6d5e0], word(&h, 0x58));
        let mut actual = resources.image_header.clone();
        actual[0x58..0x5c].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(actual, h[..100]);
        let saved = postcard::to_allocvec(&restored)?;
        assert_eq!(
            restored.advance(&controls)?,
            Some(actions::Request::SelectedPerson)
        );
        assert_eq!(saved, postcard::to_allocvec(&restored)?);
        Ok(())
    }
}
