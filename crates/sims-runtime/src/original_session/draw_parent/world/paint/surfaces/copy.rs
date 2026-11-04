//! 4b0388/5dee3e: copy the current model regions, color then depth.
//! Plane fields belong to the existing surface state. No returned engine
//! booleans or observed plane writes are installed into that state.
use super::*;
const INPUT: &str = "eb5ac13549838e729acfade2029a95de420c394da380d82ccdfb511c053b6dcc";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Reply { destination:u32, source:u32, request:backend::Request, hresult:u32 }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage { Idle, Color, ColorLock, ColorLoop, ColorUnlock, Depth, DepthLock, DepthLoop, DepthUnlock, Debug, Complete, ColorCount, DepthCount }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Call {
    destination:u32, source:u32, from:[i32;4], to:[i32;4], pc:u32,
    backend:Option<backend::Blit>, result:Option<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Copy {
    source:String, input_source:String, surfaces:[u32;2], debug:u8,
    replies:Vec<Reply>, input:usize, stage:Stage, region_source:u32, rectangles:Vec<[i32;4]>,
    index:usize, destination:Option<u32>, origin:Option<u32>, call:Option<Call>,
}
fn rectangle(v:&serde_json::Value)->Result<[i32;4]>{let b=decode(v)?;ensure!(b.len()==16,"Blt rectangle extent differs");Ok(std::array::from_fn(|i|word(&b,i*4)as i32))}
impl Copy {
    pub fn import(encoded:&[u8],input:&[u8])->Result<Self>{
        let input_source=sims_fixtures::digest(input);
        ensure!(sims_fixtures::digest(encoded)==SOURCE&&(input_source==INPUT||input_source==STREAM||input_source==END_STREAM||input_source==super::super::model::INPUT),"copy input source differs");
        let v:serde_json::Value=serde_json::from_slice(encoded)?;let m=Memory::new(&v)?;
        ensure!(v["tick"]==89073&&v["executes_target_code"]==false,"copy source epoch differs");
        let window=m.read(0x09e88960,0x520)?;
        for p in v["paint"]["planes"].as_array().context("copy plane dispatch absent")?{
            let raw=decode(&p["raw"])?;let vt=decode(&p["vtable_raw"])?;
            let br=decode(&p["backend_raw"])?;let bt=decode(&p["backend_vtable_raw"])?;
            ensure!(m.read(word(&raw,0),vt.len())?==vt&&word(&vt,0x9c)==0x5dee3e
                &&word(&vt,0x20)==0x5dece2&&word(&vt,0x24)==0x5ded38
                &&m.read(word(&br,0),bt.len())?==bt&&word(&bt,0x1c)==0x60505f,"copy virtual dispatch differs");
        }
        let input:serde_json::Value=serde_json::from_slice(input)?;let mut replies=vec![];
        for r in input["external_blts"].as_array().context("copy platform inputs absent")?{
            let number=|key:&str|->Result<u32>{r[key].as_u64().context("Blt input word absent")?.try_into().context("Blt input word overflow")};
            let source=number("source_backend")?;
            replies.push(Reply{destination:number("destination_backend")?,source,
                request:backend::Request::DirectDrawBlit{interface:number("interface")?,destination:rectangle(&r["destination_rectangle"] )?,
                    source_interface:number("source_interface")?,source:rectangle(&r["source_rectangle"] )?,flags:number("flags")?,
                    effects_address:source.wrapping_add(0xf4),effects:decode(&r["effects"] )?},hresult:number("hresult")?});
        }
        ensure!(replies.len()==2,"copy platform input count differs");
        Ok(Self{source:SOURCE.into(),input_source,surfaces:[word(&window,0x70),word(&window,0x74)],debug:window[0x17e],
            replies,input:0,stage:Stage::Idle,region_source:0,rectangles:vec![],index:0,destination:None,origin:None,call:None})
    }
    pub fn validate(&self,planes:&BTreeMap<u32,Plane>,backends:&BTreeMap<u32,backend::Fields>)->Result<()>{
        ensure!(self.source==SOURCE&&(self.input_source==INPUT||self.input_source==STREAM||self.input_source==END_STREAM||self.input_source==super::super::model::INPUT)&&self.input<=self.replies.len()&&self.index<=self.rectangles.len(),"copy lifetime differs");
        ensure!(self.surfaces.iter().all(|p|planes.contains_key(p)),"copy surface absent");
        if let Some(c)=&self.call{
            ensure!(matches!(self.stage,Stage::ColorLoop|Stage::DepthLoop)&&Some(c.destination)==self.destination&&Some(c.source)==self.origin,"plane copy lost caller");
            if let Some(b)=&c.backend{
                ensure!(c.pc==0x5df044,"copy backend lost plane");
                b.validate(&backends[&planes[&c.destination].backend],&backends[&planes[&c.source].backend]).map_err(anyhow::Error::msg)?;
            }
        }
        ensure!(self.stage!=Stage::Complete||self.call.is_none(),"completed copy retains child");Ok(())
    }
    pub fn started(&self)->bool{self.stage!=Stage::Idle}
    pub fn input_source(&self)->&str{&self.input_source}
    pub fn complete(&self)->bool{self.stage==Stage::Complete}
    pub fn start(&mut self,source:u32,rectangles:Vec<[i32;4]>)->Result<()>{ensure!(!self.started(),"surface copy already started");self.region_source=source;self.rectangles=rectangles;self.stage=Stage::Color;Ok(())}
    fn plane_flags(planes:&mut BTreeMap<u32,Plane>,id:u32,lock:bool)->Result<u8>{
        let p=planes.get_mut(&id).context("copy lock plane absent")?;
        let mut f=targets::Plane{buffer:word(&p.raw,0x34),half_pitch:word(&p.raw,0x38),locks:word(&p.raw,0x3c),flags:word(&p.raw,0x50)};
        // Flag16 bypasses the backend and preserves buffer/pitch caches.
        let result=if lock{f.publish_lock(16,0,0,0)}else{f.unlock(16,0)};
        for(o,v)in[(0x34,f.buffer),(0x38,f.half_pitch),(0x3c,f.locks),(0x50,f.flags)]{set(&mut p.raw,o,v);}Ok(result)
    }
    pub fn at_native_cut(&self,q:&serde_json::Value,planes:&BTreeMap<u32,Plane>)->bool{
        let pc=q["pc"].as_u64().unwrap_or(0)as u32;
        let Some(c)=&self.call else{
            if self.stage==Stage::Color&&pc==0x4b0388{return q["registers"]["ecx"]==0x09e88960u32;}
            return self.destination.is_some_and(|id|q["registers"]["ecx"]==id)
                &&(matches!(self.stage,Stage::ColorLock|Stage::DepthLock)&&pc==0x5dece2
                   ||matches!(self.stage,Stage::ColorCount|Stage::DepthCount)&&pc==0x5ded2e);
        };
        if let Some(b)=&c.backend{return b.pc()==pc&&(pc!=0x60505f||q["registers"]["ecx"]==planes[&c.destination].backend);}
        c.pc==pc&&(pc!=0x5dee3e||q["registers"]["ecx"]==c.destination)
    }
    pub fn compare_result(&self,q:&serde_json::Value)->Result<()>{
        let Some(c)=self.call.as_ref()else{
            if matches!(q["pc"].as_u64(),Some(0x5dece2|0x5ded2e)){return Ok(());}
            ensure!(q["pc"]==0x4b0388u32,"copy child absent at native cut");
            let raw=decode(&q["after_window_copy"]["stack"]["raw"])?;
            ensure!(raw.len()>=16&&word(&raw,0)==0x4a9351&&word(&raw,4)==self.region_source&&word(&raw,8)as usize==self.rectangles.len(),"native region-copy arguments differ");return Ok(());
        };
        let result=c.backend.as_ref().and_then(backend::Blit::result).or(c.result);
        if let Some(result)=result{ensure!(q["registers"]["eax"].as_u64().context("native copy AL absent")?as u8==result,"copy AL differs");}Ok(())
    }
    pub fn advance(&mut self,planes:&mut BTreeMap<u32,Plane>,backends:&BTreeMap<u32,backend::Fields>,flags:u32)->Result<bool>{
        self.validate(planes,backends)?;
        if !self.started()||self.complete(){return Ok(false);}
        if self.call.as_ref().is_some_and(|c|c.backend.is_none()&&c.pc==0x5def33){
            self.call=None;self.index+=1;self.validate(planes,backends)?;return Ok(true);
        }
        if let Some(c)=&mut self.call{
            let destination=&planes[&c.destination];let source=&planes[&c.source];
            if let Some(b)=&mut c.backend{
                let dst=&backends[&destination.backend];let src=&backends[&source.backend];
                if let Some(request)=b.pending(){
                    let r=self.replies.get(self.input).context("original Blt inputs exhausted")?;
                    ensure!(r.destination==destination.backend&&r.source==source.backend&&request==&r.request,"computed Blt call differs");
                    b.directdraw_return(dst,src,r.hresult).map_err(anyhow::Error::msg)?;self.input+=1;
                }else if !b.advance(dst,src).map_err(anyhow::Error::msg)?{return Ok(false);}
                if b.complete(){c.result=b.result();c.backend=None;c.pc=0x5df047;}
            }else{
                c.pc=match c.pc{
                    0x5dee3e=>{
                        if word(&destination.raw,0x44)==3{return Ok(false);}
                        if !clip(&mut c.from,&mut c.to,[word(&destination.raw,0x1c)as i32,word(&destination.raw,0x20)as i32]){
                            c.result=Some(1);0x5def2f
                        }else if destination.backend!=0&&source.backend!=0{0x5df044}
                        else{return Ok(false);}
                    },
                    0x5df044=>{c.backend=Some(backend::Blit::new(source.backend,c.from,c.to));0x5df044},
                    0x5df047=>0x5df060,0x5df060=>0x5df063,0x5df063=>0x5df066,0x5df066=>0x5def2f,
                    0x5def2f=>0x5def30,0x5def30=>0x5def31,0x5def31=>0x5def32,0x5def32=>0x5def33,
                    _=>anyhow::bail!("plane copy requires retained child"),
                };
            }
        }else{
            self.stage=match self.stage{
                Stage::Color=>if flags&1!=0{self.destination=Some(self.surfaces[0]);self.origin=Some(self.surfaces[1]);Stage::ColorLock}else{Stage::Depth},
                Stage::Depth=>if flags&2!=0{
                    self.destination=Some(word(&planes[&self.surfaces[0]].raw,0x5c));self.origin=Some(word(&planes[&self.surfaces[1]].raw,0x5c));Stage::DepthLock
                }else{Stage::Debug},
                Stage::ColorLock|Stage::DepthLock=>{
                    let okay=Self::plane_flags(planes,self.destination.context("copy destination absent")?,true)?!=0;self.index=0;
                    match(self.stage,okay){(Stage::ColorLock,true)=>Stage::ColorCount,(Stage::ColorLock,false)=>Stage::Depth,(_,true)=>Stage::DepthCount,(_,false)=>Stage::Debug}
                },
                Stage::ColorCount|Stage::DepthCount=>{
                    let p=planes.get_mut(&self.destination.context("count destination absent")?).context("count plane absent")?;
                    let mut f=targets::Plane{buffer:word(&p.raw,0x34),half_pitch:word(&p.raw,0x38),locks:word(&p.raw,0x3c),flags:word(&p.raw,0x50)};
                    f.finish_lock();set(&mut p.raw,0x3c,f.locks);if self.stage==Stage::ColorCount{Stage::ColorLoop}else{Stage::DepthLoop}
                },
                Stage::ColorLoop|Stage::DepthLoop=>{
                    if self.index==self.rectangles.len(){if self.stage==Stage::ColorLoop{Stage::ColorUnlock}else{Stage::DepthUnlock}}
                    else{let rectangle=self.rectangles[self.index];self.call=Some(Call{destination:self.destination.context("copy destination absent")?,source:self.origin.context("copy source absent")?,from:rectangle,to:rectangle,pc:0x5dee3e,backend:None,result:None});self.stage}
                },
                Stage::ColorUnlock|Stage::DepthUnlock=>{Self::plane_flags(planes,self.destination.context("copy unlock destination absent")?,false)?;if self.stage==Stage::ColorUnlock{Stage::Depth}else{Stage::Debug}},
                Stage::Debug=>{if self.debug!=0{return Ok(false);}Stage::Complete},
                _=>return Ok(false),
            };
        }
        self.validate(planes,backends)?;Ok(true)
    }
}
/// 5def36..5df022, with the caller's null optional clipping rectangle.
/// Arithmetic wraps as the original32-bit instructions; signed predicates are
/// evaluated after TEST/CMP, rather than using saturating host arithmetic.
fn clip(source:&mut[i32;4],destination:&mut[i32;4],size:[i32;2])->bool{
    for axis in 0..2{
        let end=axis+2;let delta=destination[end].wrapping_sub(size[axis]);
        if delta>0{source[end]=source[end].wrapping_sub(delta);destination[end]=destination[end].wrapping_sub(delta);if destination[end]<=destination[axis]{return false;}}
        let delta=destination[axis].wrapping_neg();
        if delta>0{source[axis]=source[axis].wrapping_add(delta);destination[axis]=destination[axis].wrapping_add(delta);if destination[axis]>=destination[end]{return false;}}
    }
    destination[0]<destination[2]&&destination[1]<destination[3]
}
