//! B object-directory identities. Live registration is borrowed from the VM;
//! missing/new identities are not fabricated null registry entries.
use super::*;
use std::collections::BTreeMap;
const SOURCE: &str = "f042f68b9f32d7fe96351320812492bc731df7ecee450af9d10c864781b813a9";
fn decode(s:&str)->Result<Vec<u8>>{crate::original_session::completed_animations::bytes(s)}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source {
    identity: String,
    registry: u32,
    begin: u32,
    end: u32,
    pointers: BTreeMap<i16, u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    shadow_codes:Option<BTreeMap<i16,i32>>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    physical:Option<BTreeMap<i16,[u32;2]>>,
}
impl Source {
    pub fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE,
            "ordinary object source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "ordinary object source epoch differs"
        );
        let m = Memory::new(&v)?;
        let renderer = word(&m.read(0x09e88960 + 0x1c, 4)?, 0);
        let registry = word(&m.read(renderer + 0x14, 4)?, 0);
        let h = m.read(registry, 0x20)?;
        let begin = word(&h, 0x18);
        let end = word(&h, 0x1c);
        ensure!(
            begin <= end && (end - begin) % 4 == 0 && end - begin <= 65536,
            "ordinary object registry range invalid"
        );
        let raw = m.read(begin, (end - begin) as usize)?;
        let mut pointers = BTreeMap::new();
        let mut shadow_codes=BTreeMap::new();
        for (i, r) in raw.chunks_exact(4).enumerate() {
            let p = word(r, 0);
            if p != 0 {
                let id = i16::try_from(i + 1)?;
                let obj = m.read(p, 0x18c)?;
                ensure!(
                    i16::from_le_bytes(obj[0xf0..0xf2].try_into()?) == id,
                    "ordinary directory identity differs"
                );
                pointers.insert(id, p);
                shadow_codes.insert(id,word(&obj,0x184)as i32);
            }
        }
        let state = Self {
            identity: SOURCE.into(),
            registry,
            begin,
            end,
            pointers,
            shadow_codes:Some(shadow_codes),
            physical:None,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.identity == SOURCE && self.begin <= self.end && (self.end - self.begin) % 4 == 0,
            "ordinary directory provenance differs"
        );
        Ok(())
    }
    pub fn bind_physical(&mut self,encoded:&[u8])->Result<()> {
        ensure!(self.physical.is_none()&&sims_fixtures::digest(encoded)==SOURCE,"physical-next directory differs/already bound");
        let v:serde_json::Value=serde_json::from_slice(encoded)?;let m=Memory::new(&v)?;
        let mut fields=BTreeMap::new();
        for (&id,&pointer) in &self.pointers {
            let obj=m.read(pointer,0x18c)?;let vector=word(&obj,0x130);let owner=word(&obj,0xdc);
            ensure!(vector!=0&&word(&obj,0x134)>=vector+36&&owner==self.registry,"physical-next vector/registry ownership differs");
            fields.insert(id,[vector,owner]);
        }
        self.physical=Some(fields);self.validate()
    }
    pub fn physical(&self,id:i16)->Option<[u32;2]>{self.physical.as_ref()?.get(&id).copied()}
    pub fn range(&self)->[u32;2]{[self.begin,self.end]}
    pub fn compare_cursor(&self,q:&serde_json::Value,id:i16,pointer:u32)->Result<()> {
        let n=&q["object_cursor"];
        ensure!(n["id"]==id&&n["pointer"]==pointer&&n["registry"]==self.registry&&n["begin"]==self.begin&&n["end"]==self.end&&n["registry_entry"]==pointer,"ordinary selected cursor identity differs");Ok(())
    }
    pub fn compare_physical(&self,q:&serde_json::Value,id:i16,pointer:u32,child:&sims_sim::lot_presenter::physical_next::Next)->Result<()> {
        let [vector,owner]=self.physical(id).context("physical-next owner absent")?;let n=&q["physical_next"];
        ensure!(n["parent"]==pointer&&n["vector"]==vector&&n["registry"]==owner&&n["id"]==child.child(),"native reserved-record lookup differs");
        let raw=decode(n["raw"].as_str().context("native reserved record absent")?)?;
        ensure!(raw.len()==36&&i16::from_le_bytes(raw[16..18].try_into()?)==child.child(),"native physical-next ID bytes differ");
        if let Some(eax)=child.eax(){ensure!(q["registers"]["eax"]==eax,"physical-next EAX lifetime differs");}
        if child.registry_scope(){ensure!(q["registers"]["ecx"]==owner,"physical-next registry scope differs");}
        else {ensure!(q["registers"]["ecx"]==pointer,"physical-next parent scope differs");}
        let bp=q["ordinary_frame"]["bp"].as_i64().context("ordinary BP absent")?;
        ensure!(q["registers"]["esp"].as_i64()==Some(bp-0x238+i64::from(child.esp_delta())),"physical-next stack lifetime differs");
        if child.child()>0 {
            ensure!(n["begin"]==self.begin&&n["end"]==self.end,"physical-next registry range differs");
            let table=decode(n["registry_table"].as_str().context("native next registry absent")?)?;
            ensure!(table.len()==(self.end-self.begin)as usize,"native next registry extent differs");
            for (i,b)in table.chunks_exact(4).enumerate(){ensure!(word(b,0)==self.pointers.get(&((i+1)as i16)).copied().unwrap_or(0),"native next registry slot differs");}
        }
        if q["pc"]==0x40743bu32 {let a=&n["lookup"];ensure!(a["id"]==child.child()&&a["registry"]==owner&&a["caller"]==0x432841u32,"physical-next lookup arguments differ");}
        Ok(())
    }
    ///434e23 can return immediately without shading when this record has no
    ///shadow and its physical successor is NULL. Other cases need the retained
    ///shadow query/draw child, including an unknown older snapshot owner.
    pub fn shadow_absent(&self,id:i16,sim:&sims_sim::Simulation)->Result<bool>{
        self.validate()?;let code=self.shadow_codes.as_ref().and_then(|m|m.get(&id));
        let next=sim.native_lot.as_ref().and_then(|l|l.placement.links.next.get(&id));
        Ok(code==Some(&-1)&&next==Some(&0))
    }
    pub fn lookup(&self, id: i16, sim: &sims_sim::Simulation) -> Result<Option<u32>> {
        self.validate()?;
        if id <= 0 {
            return Ok(Some(0));
        }
        let in_range = (id as u32) < 1 + (self.end - self.begin) / 4;
        let p = self.pointers.get(&id).copied();
        let registered = sim.vm.world.objects.contains_key(&id);
        if registered {
            Ok(p)
        } else if !in_range || p.is_none() {
            Ok(Some(0))
        } else {
            Ok(None)
        }
    }
    pub fn compare(&self, q: &serde_json::Value, id: i16, selected: u32) -> Result<()> {
        let n = &q["object_lookup"];
        ensure!(
            n["head"] == id
                && n["registry"] == self.registry
                && n["begin"] == self.begin
                && n["end"] == self.end
                && n["selected"] == selected,
            "ordinary native registry identity differs"
        );
        Ok(())
    }
}
