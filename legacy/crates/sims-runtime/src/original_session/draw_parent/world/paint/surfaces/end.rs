//!4b027d/4b02a0. Only DirectDraw's HRESULT is an external reply.
use super::*;
const NATIVE:&str="c9a569590917da6cfbdb01a3da8469b11474bbb2ac8b4261bc77cce33819197a";
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
struct Reply {backend:u32,interface:u32,argument:u32,hresult:u32}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
struct Work {plane:u32,frame:targets::PlaneUnlock,backend:Option<backend::Unlock>}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
pub(super) struct End {
    source:String,trace:String,surfaces:[u32;2],overlay:u8,replies:Vec<Reply>,input:usize,
    pc:u32,phase:Option<i32>,surface:Option<u32>,surface_pc:Option<u32>,
    predicate:Option<bool>,depth:Option<u32>,work:Option<Work>,
}
impl End {
    pub fn input_source(&self)->&str{&self.trace}
    pub fn import_platform(encoded:&[u8],input:&[u8])->Result<Self>{
        let input_hash=sims_fixtures::digest(input);
        ensure!(sims_fixtures::digest(encoded)==SOURCE&&matches!(sims_fixtures::digest(input).as_str(),END_STREAM|super::super::model::INPUT),"front EndFrame platform inputs differ");
        let v:serde_json::Value=serde_json::from_slice(encoded)?;let m=Memory::new(&v)?;let window=m.read(0x09e88960,0x520)?;
        ensure!(v["tick"]==89073&&v["executes_target_code"]==false,"front EndFrame epoch differs");
        for p in v["paint"]["planes"].as_array().context("front EndFrame planes absent")?{
            let raw=decode(&p["raw"])?;let vt=decode(&p["vtable_raw"])?;let br=decode(&p["backend_raw"])?;let bt=decode(&p["backend_vtable_raw"])?;
            ensure!(m.read(word(&raw,0),vt.len())?==vt&&word(&vt,0x24)==0x5ded38&&m.read(word(&br,0),bt.len())?==bt&&word(&bt,0x10)==0x604f3e,"front EndFrame virtual dispatch differs");
        }
        let inputs:serde_json::Value=serde_json::from_slice(input)?;let mut replies=vec![];
        for r in inputs["unlocks"].as_array().context("actual front unlock replies absent")?{
            let n=|key:&str|->Result<u32>{r[key].as_u64().context("front unlock input word absent")?.try_into().context("front unlock word overflow")};
            replies.push(Reply{backend:n("backend")?,interface:n("interface")?,argument:n("argument")?,hresult:n("hresult")?});
        }
        ensure!(replies.len()==2,"front unlock input count differs");
        Ok(Self{source:SOURCE.into(),trace:input_hash,surfaces:[word(&window,0x70),word(&window,0x74)],overlay:window[0x51],
            replies,input:0,pc:0,phase:None,surface:None,surface_pc:None,predicate:None,depth:None,work:None})
    }
    pub fn import(encoded:&[u8],native:&[u8])->Result<Self>{
        ensure!(sims_fixtures::digest(encoded)==SOURCE&&sims_fixtures::digest(native)==NATIVE,"end-frame inputs differ");
        let v:serde_json::Value=serde_json::from_slice(encoded)?;
        ensure!(v["tick"]==89073&&v["executes_target_code"]==false,"end-frame source epoch differs");
        let m=Memory::new(&v)?;let window=m.read(0x09e88960,0x520)?;
        for p in v["paint"]["planes"].as_array().context("end planes absent")?{
            let raw=decode(&p["raw"])?;let vt=decode(&p["vtable_raw"])?;
            let br=decode(&p["backend_raw"])?;let bt=decode(&p["backend_vtable_raw"])?;
            ensure!(m.read(word(&raw,0),vt.len())?==vt&&word(&vt,0x24)==0x5ded38
                &&m.read(word(&br,0),bt.len())?==bt&&word(&bt,0x10)==0x604f3e,"end virtual dispatch differs");
        }
        use std::io::Read as _;
        let mut text=String::new();flate2::read::GzDecoder::new(native).read_to_string(&mut text)?;
        let rows=text.lines().map(serde_json::from_str::<serde_json::Value>).collect::<std::result::Result<Vec<_>,_>>()?;
        let rows:Vec<_>=rows.iter().filter(|q|q.get("surface_end").is_some()).collect();
        let mut replies=vec![];
        for pair in rows.windows(2){let(a,b)=(pair[0],pair[1]);if a["pc"]!=0x604f7fu32{continue;}
            ensure!(b["pc"]==0x604f85u32&&b["registers"]["esp"].as_u64()==a["registers"]["esp"].as_u64().map(|s|s+8)
                &&b["registers"]["ebp"]==a["registers"]["ebp"],"DirectDraw unlock return ABI differs");
            let call=&a["surface_end"]["external_unlock"];
            replies.push(Reply{backend:a["registers"]["esi"].as_u64().context("unlock backend absent")?.try_into()?,
                interface:call["interface"].as_u64().context("unlock interface absent")?.try_into()?,
                argument:call["argument"].as_u64().context("unlock argument absent")?.try_into()?,
                hresult:b["registers"]["eax"].as_u64().context("unlock HRESULT absent")?.try_into()?});
        }
        ensure!(replies.len()==2,"end platform input count differs");
        Ok(Self{source:SOURCE.into(),trace:NATIVE.into(),surfaces:[word(&window,0x70),word(&window,0x74)],overlay:window[0x51],
            replies,input:0,pc:0,phase:None,surface:None,surface_pc:None,predicate:None,depth:None,work:None})
    }
    pub fn validate(&self,planes:&BTreeMap<u32,Plane>,backends:&BTreeMap<u32,backend::Fields>)->Result<()>{
        ensure!(self.source==SOURCE&&(self.trace==NATIVE||self.trace==END_STREAM||self.trace==super::super::model::INPUT)&&self.replies.len()==2&&self.input<=self.replies.len(),"end input lifetime differs");
        ensure!(self.surfaces.iter().all(|p|planes.contains_key(p)),"end surface owner absent");
        if let Some(w)=&self.work{
            ensure!(matches!(self.surface_pc,Some(0x4b02c1|0x4b02df)),"plane unlock lost caller");
            let p=planes.get(&w.plane).context("unlock plane absent")?;w.frame.validate().map_err(anyhow::Error::msg)?;
            if let Some(b)=&w.backend{ensure!(w.frame.pending_backend(),"backend unlock lost plane");b.validate(&backends[&p.backend]).map_err(anyhow::Error::msg)?;}
        }
        ensure!(!self.complete()||self.surface_pc.is_none()&&self.work.is_none(),"completed end frame retains child");Ok(())
    }
    pub fn started(&self)->bool{self.pc!=0}
    pub fn complete(&self)->bool{self.pc==u32::MAX}
    pub fn consumed(&self)->usize{self.input}
    pub fn overlay(&self)->u8{self.overlay}
    pub fn start(&mut self)->Result<()>{ensure!(!self.started(),"end frame already started");self.pc=0x4b027d;Ok(())}
    pub fn at_native_cut(&self,q:&serde_json::Value,planes:&BTreeMap<u32,Plane>)->bool{
        let pc=q["pc"].as_u64().unwrap_or(0)as u32;
        if let Some(w)=&self.work{
            if let Some(b)=&w.backend{return b.at_native_cut(pc)&&(pc!=0x604f3e||q["registers"]["ecx"]==planes[&w.plane].backend);}
            return w.frame.at_native_cut(pc)&&(pc!=0x5ded38||q["registers"]["ecx"]==w.plane);
        }
        self.surface_pc.map_or(self.pc==pc,|at|at==pc)
    }
    pub fn advance(&mut self,fields:&mut targets::Fields,planes:&mut BTreeMap<u32,Plane>,backends:&mut BTreeMap<u32,backend::Fields>,flags:u32)->Result<bool>{
        self.validate(planes,backends)?;
        if !self.started()||self.complete(){return Ok(false);}
        if self.surface_pc.is_some(){
            if let Some(w)=self.work.as_mut(){
                let id=planes[&w.plane].backend;
                if let Some(b)=w.backend.as_mut(){
                    let fields=backends.get_mut(&id).context("unlock backend absent")?;
                    if let Some(request)=b.pending().cloned(){
                        let backend::Request::DirectDrawUnlock{interface,argument}=request else{return Ok(false)};
                        let r=self.replies.get(self.input).context("original unlock inputs exhausted")?;
                        ensure!(r.backend==id&&r.interface==interface&&r.argument==argument,"computed DirectDraw unlock request differs");
                        b.directdraw_return(fields,r.hresult).map_err(anyhow::Error::msg)?;self.input+=1;
                    }else{ensure!(b.advance(fields).map_err(anyhow::Error::msg)?,"backend unlock stalled");}
                    if b.complete(){w.frame.backend_return(b.result().context("unlock result absent")?).map_err(anyhow::Error::msg)?;w.backend=None;}
                }else if w.frame.pending_backend(){w.backend=Some(backend::Unlock::new());}
                else if w.frame.complete(){
                    self.work=None;self.surface_pc=Some(if self.surface_pc==Some(0x4b02c1){0x4b02c4}else{0x4b02e2});
                }else{
                    let p=planes.get_mut(&w.plane).unwrap();let mut f=targets::Plane{buffer:word(&p.raw,0x34),half_pitch:word(&p.raw,0x38),locks:word(&p.raw,0x3c),flags:word(&p.raw,0x50)};
                    ensure!(w.frame.advance(&mut f).map_err(anyhow::Error::msg)?,"plane unlock stalled");
                    for(o,v)in[(0x34,f.buffer),(0x38,f.half_pitch),(0x3c,f.locks),(0x50,f.flags)]{set(&mut p.raw,o,v);}
                }
            }else{
                let next=match self.surface_pc.unwrap(){
                    0x4b02a0=>0x4b02a1,0x4b02a1=>0x4b02a2,0x4b02a2=>0x4b02a3,0x4b02a3=>0x4b02a5,0x4b02a5=>0x4b02a9,0x4b02a9=>0x4b02ab,0x4b02ab=>0x4b02ad,0x4b02ad=>0x4b02ae,
                    0x4b02ae=>{self.predicate=Some(flags&1!=0);0x4b02b3},0x4b02b3=>0x4b02b7,0x4b02b7=>0x4b02b9,
                    0x4b02b9=>if self.predicate.context("color unlock predicate absent")?{0x4b02bb}else{0x4b02c4},
                    0x4b02bb=>0x4b02bd,0x4b02bd=>0x4b02bf,0x4b02bf=>0x4b02c1,
                    0x4b02c1=>{self.work=Some(Work{plane:self.surface.context("end surface absent")?,frame:targets::PlaneUnlock::new(64),backend:None});0x4b02c1},
                    0x4b02c4=>0x4b02c8,0x4b02c8=>0x4b02ca,0x4b02ca=>0x4b02cb,
                    0x4b02cb=>{self.predicate=Some(flags&2!=0);0x4b02d0},0x4b02d0=>0x4b02d2,
                    0x4b02d2=>if self.predicate.context("depth unlock predicate absent")?{0x4b02d4}else{0x4b02e2},
                    0x4b02d4=>{self.depth=Some(word(&planes[&self.surface.context("end surface absent")?].raw,0x5c));0x4b02d7},
                    0x4b02d7=>0x4b02d9,
                    0x4b02d9=>if self.depth==Some(0){0x4b02e2}else{0x4b02db},
                    0x4b02db=>0x4b02dd,0x4b02dd=>0x4b02df,
                    0x4b02df=>{self.work=Some(Work{plane:self.depth.context("depth receiver absent")?,frame:targets::PlaneUnlock::new(64),backend:None});0x4b02df},
                    0x4b02e2=>{fields.active=0;0x4b02e5},0x4b02e5=>{fields.surface=0;0x4b02eb},0x4b02eb=>0x4b02ec,
                    0x4b02ec=>{fields.buffers.color=0;0x4b02f2},0x4b02f2=>{fields.buffers.depth=0;0x4b02f8},
                    0x4b02f8=>0x4b02f9,0x4b02f9=>0x4b02fa,
                    0x4b02fa=>{self.surface_pc=None;self.pc=0x4b029d;return Ok(true);},
                    _=>anyhow::bail!("end-surface PC differs"),
                };self.surface_pc=Some(next);
            }
        }else{
            self.pc=match self.pc{
                0x4b027d=>{self.phase=Some(fields.phase);0x4b0280},0x4b0280=>0x4b0282,
                0x4b0282=>if self.phase.unwrap()<0{0x4b029d}else{0x4b0284},0x4b0284=>0x4b0287,
                0x4b0287=>if self.phase.unwrap()>=2{0x4b029d}else{0x4b0289},0x4b0289=>0x4b028d,
                0x4b028d=>{fields.phase=3;0x4b0294},
                0x4b0294=>{let p=self.surfaces[self.phase.unwrap()as usize];ensure!(planes.contains_key(&p),"selected end surface absent");self.surface=Some(p);0x4b0298},
                0x4b0298=>{self.surface_pc=Some(0x4b02a0);0x4b0298},0x4b029d=>u32::MAX,
                _=>anyhow::bail!("end-window PC differs"),
            };
        }
        self.validate(planes,backends)?;Ok(true)
    }
}
