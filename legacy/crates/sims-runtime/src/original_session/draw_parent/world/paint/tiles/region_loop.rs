//! Ordered null-explicit-clip level/region handoff in 4aceb8. Readbacks are
//! comparisons only; region geometry comes from the current retained list.
use super::*;
const SOURCE: &str = "a8fc950de149bc26da8a4e2b160f85e9e696699cb8a9b594ed26dbef9d44f9f7";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source { identity: String, overlay: u8 }
impl Source {
    pub fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(sims_fixtures::digest(encoded)==SOURCE,"region loop B source differs");
        let v:serde_json::Value=serde_json::from_slice(encoded)?;
        ensure!(v["tick"]==89073&&v["executes_target_code"]==false,"region loop source epoch differs");
        let m=Memory::new(&v)?;
        Ok(Self{identity:SOURCE.into(),overlay:m.read(0x6f00e5,1)?[0]})
    }
    pub fn validate(&self)->Result<()>{ensure!(self.identity==SOURCE,"region loop input owner differs");Ok(())}
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Cursor {
    pc:u32, level:i32, stored_level:i32, cached:i32, index:usize,
    region:u32, end:u32, rectangle:Option<[i32;4]>, packed:[Option<i16>;4], copied:usize,
    eax:Option<u32>, esi:u32,
}
impl Cursor {
    pub fn new(level:i32,cached:i32,index:usize,regions:&sims_vm::scheduler::clock::draw::regions::List)->Result<Self>{
        regions.validate().map_err(anyhow::Error::msg)?;
        let next=level.wrapping_add(1);
        Ok(Self{pc:0x4ad369,level:next,stored_level:level,cached,index,region:word(&regions.header,0).checked_add(u32::try_from(index)?.checked_mul(16).context("region offset overflow")?).context("region pointer overflow")?,end:word(&regions.header,4),rectangle:None,packed:[None;4],copied:0,eax:None,esi:next as u32})
    }
    pub fn validate(&self)->Result<()>{ensure!(self.region<=self.end&&self.copied<=4,"region loop extent differs");Ok(())}
    pub fn pc(&self)->u32{self.pc}
    pub fn index(&self)->usize{self.index}
    pub fn next_level(&self)->Option<i32>{(self.pc==0x4ad2af).then_some(self.level)}
    pub fn blocked(&self)->bool{matches!(self.pc,0x4ad249|0x4ad3a7)}
    fn packed(&self,half:usize)->Result<[i16;2]>{Ok([self.packed[half*2].context("clip local absent")?,self.packed[half*2+1].context("clip local absent")?])}
    fn load_ax(&mut self,value:i32){self.eax=Some((self.eax.unwrap()&0xffff0000)|u32::from(value as u16));}
    pub fn advance(&mut self,source:&Source,regions:&sims_vm::scheduler::clock::draw::regions::List,surfaces:&mut surfaces::State,flags:u32)->Result<bool>{
        self.validate()?;source.validate()?;regions.validate().map_err(anyhow::Error::msg)?;
        let next=match self.pc {
            0x4ad369=>0x4ad2a1,
            0x4ad2a1=>{self.eax=Some(self.level as u32);0x4ad2a3},
            0x4ad2a3=>{self.stored_level=self.level;0x4ad2a6},
            0x4ad2a6=>0x4ad2a9,
            0x4ad2a9=>if self.level>self.cached{0x4ad36e}else{0x4ad2af},
            0x4ad36e=>0x4ad372,
            0x4ad372=>0x4ad374,
            0x4ad374=>{self.index=self.index.checked_add(1).context("region index overflow")?;self.region=self.region.checked_add(16).context("region pointer overflow")?;0x4ad378},
            0x4ad378=>0x4ad1b8,
            0x4ad1b8=>{self.eax=Some(self.region);0x4ad1bb},
            0x4ad1bb=>0x4ad1c1,
            0x4ad1c1=>if self.region==self.end{0x4ad1c3}else{0x4ad1cd},
            0x4ad1c3=>0x4ad1c7,
            0x4ad1c7=>0x4ad386,
            0x4ad1cd=>0x4ad1d1,
            0x4ad1d1=>0x4ad1d3,
            0x4ad1d3=>{self.esi=self.region;self.rectangle=Some(regions.rectangle(self.index).map_err(anyhow::Error::msg)?);0x4ad1d6},
            0x4ad1d6=>{self.load_ax(self.rectangle.unwrap()[1]);0x4ad1da},
            0x4ad1da=>{self.packed[0]=Some(self.eax.unwrap()as i16);0x4ad1de},
            0x4ad1de=>{self.load_ax(self.rectangle.unwrap()[3]);0x4ad1e2},
            0x4ad1e2=>{self.packed[2]=Some(self.eax.unwrap()as i16);0x4ad1e6},
            0x4ad1e6=>{self.load_ax(self.rectangle.unwrap()[0]);0x4ad1e9},
            0x4ad1e9=>{self.packed[1]=Some(self.eax.unwrap()as i16);0x4ad1ed},
            0x4ad1ed=>{self.load_ax(self.rectangle.unwrap()[2]);0x4ad1f1},
            0x4ad1f1=>{self.packed[3]=Some(self.eax.unwrap()as i16);0x4ad1f5},
            0x4ad1f5=>{let p=self.packed(0)?;self.eax=Some(u32::from(p[0]as u16)|(u32::from(p[1]as u16)<<16));0x4ad1f8},
            0x4ad1f8=>{surfaces.set_window_clip_half(0,self.packed(0)?)?;0x4ad1fd},
            0x4ad1fd=>{let p=self.packed(1)?;self.eax=Some(u32::from(p[0]as u16)|(u32::from(p[1]as u16)<<16));0x4ad200},
            0x4ad200=>{surfaces.set_window_clip_half(1,self.packed(1)?)?;0x4ad205},
            0x4ad205=>0x4ad239,
            0x4ad239=>0x4ad23c,
            0x4ad23c=>0x4ad243,
            0x4ad243=>{self.copied+=1;self.esi=self.esi.wrapping_add(4);0x4ad244},
            0x4ad244=>{self.copied+=1;self.esi=self.esi.wrapping_add(4);0x4ad245},
            0x4ad245=>{self.copied+=1;self.esi=self.esi.wrapping_add(4);0x4ad246},
            0x4ad246=>{self.copied+=1;self.esi=self.esi.wrapping_add(4);0x4ad247},
            0x4ad247=>if source.overlay==0{0x4ad286}else{0x4ad249},
            0x4ad286=>0x4ad289,
            0x4ad289=>{self.eax=Some(self.cached as u32);0x4ad28c},
            0x4ad28c=>0x4ad28e,
            0x4ad28e=>0x4ad291,
            0x4ad291=>0x4ad293,
            0x4ad293=>{self.esi=1;0x4ad294},
            0x4ad294=>0x4ad295,
            0x4ad295=>0x4ad296,
            0x4ad296=>0x4ad297,
            0x4ad297=>0x4ad29c,
            0x4ad386=>0x4ad38a,
            0x4ad38a=>{self.esi=surfaces.fields.phase as u32;0x4ad38d},
            0x4ad38d=>0x4ad391,
            0x4ad391=>if surfaces.acquired_here()?{0x4ad393}else{0x4ad39d},
            0x4ad393=>0x4ad396,0x4ad396=>0x4ad398,
            0x4ad398=>{
                if !surfaces.end_started(){if !surfaces.start_end()?{return Ok(false);}self.eax=None;return Ok(true);}
                if !surfaces.end_complete(){return surfaces.advance_end(flags);}
                0x4ad39d
            },
            0x4ad39d=>{if surfaces.end_overlay().is_none(){return Ok(false);}0x4ad3a1},
            0x4ad3a1=>if surfaces.end_overlay().context("window overlay input absent")?==0{0x4ad47b}else{0x4ad3a7},
            0x4ad47b=>0x4ad47e,0x4ad47e=>0x4ad47f,0x4ad47f=>0x4ad480,0x4ad480=>0x4ad487,
            0x4ad487=>0x4ad488,0x4ad488=>0x4ad489,0x4ad489=>0x4a92a2,
            0x4ad2af|0x4ad249|0x4ad3a7|0x4ad29c|0x4a92a2=>return Ok(false),
            _=>anyhow::bail!("region loop continuation PC differs"),
        };self.pc=next;self.validate()?;Ok(true)
    }
    pub fn compare(&self,q:&serde_json::Value,surfaces:&surfaces::State)->Result<()>{
        self.validate()?;ensure!(q["pc"]==self.pc&&q["tick"]==89081,"region tail phase differs");
        surfaces.compare_window_tail(q)?;
        if self.pc==0x4a92a2{return Ok(());}
        let p=&q["post_base"];let f=&p["parent_frame"];let raw=decode(&f["raw"])?;let bp=p["parent_bp"].as_u64().context("region tail BP absent")?as u32;
        ensure!(f["address"]==bp-0x70&&raw.len()==0xa0&&word(&raw,0x7c)==0,"region tail frame/NULL explicit clip differs");
        ensure!(word(&raw,0x58)==self.region&&word(&raw,0x44)as i32==self.stored_level&&word(&raw,0x48)as i32==self.cached,"region tail pointer/level lifetime differs");
        ensure!(decode(&p["clip"])?==surfaces.window_clip().iter().flat_map(|x|x.to_le_bytes()).collect::<Vec<_>>(),"region tail partial clip differs");
        if let Some(eax)=self.eax{ensure!(q["registers"]["eax"]==eax,"region tail EAX/AX differs at{:x}",self.pc);}
        ensure!(q["registers"]["esi"]==self.esi,"region tail ESI differs at{:x}",self.pc);
        for (i,v)in self.packed.iter().enumerate(){if let Some(v)=v{ensure!(i16::from_le_bytes(raw[0x4c+2*i..0x4e+2*i].try_into()?)==*v,"region packed local store differs");}}
        if let Some(rect)=self.rectangle{for (i,v)in rect[..self.copied].iter().enumerate(){ensure!(word(&raw,0x20+4*i)as i32==*v,"region rectangle copy differs");}}
        if self.pc==0x4ad29c{let stack=decode(&p["stack"]["raw"])?;ensure!(std::array::from_fn::<_,4,_>(|i|word(&stack,4*i))==[0x6f0c10,1,1,1]&&word(&stack,16)==q["context"]["pointer"].as_u64().context("base context absent")?as u32,"next base scan ABI differs");}
        Ok(())
    }
}
