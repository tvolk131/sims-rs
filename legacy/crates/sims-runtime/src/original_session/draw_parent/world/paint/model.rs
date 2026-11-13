//! Retained 5ce76a/5ce7c7 model gate. B owns the fields; only actual external
//! interface replies are inputs. Engine flags/branches are computed here.
use super::*;
pub(super) const SOURCE:&str="445ac5b85f3408db51aad0a7b2a0cdd01d5de3a66437f4ad530dfc1cbbddb392";
pub(super) const INPUT:&str="2407321571b66ceb615d3f54004988ef63c10987c518db7e44f0cdbeb9e38c9d";
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
pub(in crate::original_session::draw_parent::world) struct DrawList {pub header:[u32;3],pub entries:Vec<(sims_vm::ObjectId,u32)>}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
struct Reply {offset:u32,arguments:Vec<u32>,target:u32,hresult:u32}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
struct Call {pc:u32,begin:bool,reset:u8,result:Option<u32>,clear:Option<u32>}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
pub(in crate::original_session::draw_parent::world) struct State {
    source:String,input_source:String,pointer:u32,backend:u32,interface:u32,methods:[u32;3],
    active:u8,dirty:u8,last_hresult:u32,viewport:u32,replies:Vec<Reply>,input:usize,
    call:Option<Call>,pub lists:[DrawList;2],pub draw_enabled:u8,
}
fn decode(v:&serde_json::Value)->Result<Vec<u8>>{crate::original_session::completed_animations::bytes(v.as_str().context("model bytes absent")?)}
impl State {
    pub fn import(source:&[u8],input:&[u8],lists:[DrawList;2])->Result<Self>{
        ensure!(sims_fixtures::digest(source)==SOURCE&&sims_fixtures::digest(input)==INPUT,"model input source differs");
        let value:serde_json::Value=serde_json::from_slice(source)?;let m=Memory::new(&value)?;
        ensure!(value["tick"]==89073&&value["executes_target_code"]==false,"model B epoch differs");
        let view=m.read(0x09e88960,0x520)?;let pointer=word(&view,0x68);let raw=m.read(pointer,0x204)?;
        let backend=word(&raw,0x60);let br=m.read(backend,0x3c)?;let interface=word(&br,0x38);let vt=word(&m.read(interface,4)?,0);let table=m.read(vt,0x100)?;
        let methods=[0x24,0x28,0x30].map(|o|word(&table,o));
        let input:serde_json::Value=serde_json::from_slice(input)?;let mut replies=vec![];
        for r in input["external_model_calls"].as_array().context("model platform inputs absent")?{
            let number=|k:&str|->Result<u32>{r[k].as_u64().context("model reply word absent")?.try_into().context("model reply word overflow")};
            ensure!(r["model"]==pointer&&r["interface"]==interface,"model reply owner differs");
            let arguments=r["arguments"].as_array().context("model arguments absent")?.iter().map(|v|v.as_u64().context("model argument absent").and_then(|n|n.try_into().context("model argument overflow"))).collect::<Result<Vec<u32>>>()?;
            replies.push(Reply{offset:number("offset")?,arguments,target:number("target")?,hresult:number("hresult")?});
        }
        let state=Self{source:SOURCE.into(),input_source:INPUT.into(),pointer,backend,interface,methods,
            active:raw[0x64],dirty:raw[0xd0],last_hresult:word(&raw,0x5c),viewport:word(&raw,0xd8),replies,input:0,call:None,lists,draw_enabled:m.read(0x6c724f,1)?[0]};
        state.validate()?;Ok(state)
    }
    pub fn validate(&self)->Result<()>{
        ensure!(self.source==SOURCE&&self.input_source==INPUT&&self.pointer==0x09e89030&&self.input<=self.replies.len(),"model owner lifetime differs");
        for l in &self.lists{ensure!(l.header[0]<=l.header[1]&&l.header[1]<=l.header[2]&&l.header[1]-l.header[0]==4*l.entries.len()as u32,"model drawable-list extent differs");}
        ensure!(self.call.as_ref().is_none_or(|c|c.clear.is_none()||c.begin&&c.pc==0x5ce7bc),"model clear lost caller");Ok(())
    }
    pub fn start(&mut self,begin:bool,reset:u8)->Result<()>{self.validate()?;ensure!(self.call.is_none(),"model call overlaps a live child");self.call=Some(Call{pc:if begin{0x5ce76a}else{0x5ce7c7},begin,reset,result:None,clear:None});Ok(())}
    pub fn started(&self)->bool{self.call.is_some()}
    pub fn complete(&self)->bool{self.call.as_ref().is_some_and(|c|c.pc==if c.begin{0x5ce7c4}else{0x5ce7de})}
    pub fn finish(&mut self)->Result<u32>{ensure!(self.complete(),"model returned before callee epilogue");Ok(self.call.take().unwrap().result.context("model result absent")?)}
    fn reply(&mut self,offset:u32,arguments:Vec<u32>)->Result<u32>{
        let index=match offset{0x24=>0,0x28=>1,0x30=>2,_=>anyhow::bail!("model interface slot differs")};
        let r=self.replies.get(self.input).context("model platform input exhausted")?;
        ensure!(r.offset==offset&&r.arguments==arguments&&r.target==self.methods[index],"model computed interface call differs");let result=r.hresult;self.input+=1;Ok(result)
    }
    pub fn advance(&mut self)->Result<bool>{
        self.validate()?;let Some(mut c)=self.call.take()else{return Ok(false)};
        let result=(||->Result<bool>{
            if c.pc==0x5ce7bc{
                c.clear=Some(match c.clear{None=>0x5ce911,Some(0x5ce911)=>0x5ce912,Some(0x5ce912)=>0x5ce914,Some(0x5ce914)=>0x5ce91b,
                    Some(0x5ce91b)=>if self.dirty==0{0x5cea4e}else{0x5ce921},Some(0x5cea4e)=>{c.result=Some(self.last_hresult);0x5cea51},Some(0x5cea51)=>0x5cea52,
                    Some(0x5cea52)=>{c.clear=None;c.pc=0x5ce7c1;return Ok(true);},Some(0x5ce921)=>return Ok(false),_=>anyhow::bail!("model reset PC differs")});return Ok(true);
            }
            c.pc=match c.pc{
                0x5ce76a=>0x5ce76b,0x5ce76b=>0x5ce76d,0x5ce76d=>0x5ce770,
                0x5ce770=>{self.active=1;0x5ce774},0x5ce774=>0x5ce777,0x5ce777=>0x5ce778,0x5ce778=>0x5ce77a,
                0x5ce77a|0x5ce799=>{c.result=Some(self.reply(0x24,vec![self.interface])?);if c.pc==0x5ce77a{0x5ce77d}else{0x5ce79c}},
                0x5ce77d=>0x5ce77f,0x5ce77f=>if c.result.context("model first result absent")?==0{0x5ce79e}else{0x5ce781},
                0x5ce781=>0x5ce786,0x5ce786=>if c.result==Some(0x887601c2){0x5ce788}else{0x5ce7c3},
                // Lost-surface restoration is an actual engine child, not success.
                0x5ce788=>return Ok(false),0x5ce79c=>0x5ce7c3,
                0x5ce79e=>0x5ce7a1,0x5ce7a1=>0x5ce7a7,0x5ce7a7=>0x5ce7aa,0x5ce7aa=>0x5ce7ab,0x5ce7ab=>0x5ce7ad,
                0x5ce7ad=>{c.result=Some(self.reply(0x30,vec![self.interface,self.viewport])?);0x5ce7b0},
                0x5ce7b0=>0x5ce7b2,0x5ce7b2=>if c.result==Some(0){0x5ce7b4}else{0x5ce7c3},
                0x5ce7b4=>0x5ce7b8,0x5ce7b8=>if c.reset==0{0x5ce7c1}else{0x5ce7ba},0x5ce7ba=>0x5ce7bc,
                0x5ce7c1=>{c.result=Some(0);0x5ce7c3},0x5ce7c3=>0x5ce7c4,
                0x5ce7c7=>0x5ce7c8,0x5ce7c8=>0x5ce7ca,0x5ce7ca=>0x5ce7cd,0x5ce7cd=>0x5ce7d0,0x5ce7d0=>0x5ce7d1,0x5ce7d1=>0x5ce7d3,
                0x5ce7d3=>{c.result=Some(self.reply(0x28,vec![self.interface])?);0x5ce7d6},0x5ce7d6=>0x5ce7d8,
                0x5ce7d8=>if c.result==Some(0){0x5ce7da}else{0x5ce7dd},0x5ce7da=>{self.active=0;0x5ce7dd},0x5ce7dd=>0x5ce7de,
                0x5ce7c4|0x5ce7de=>return Ok(false),_=>anyhow::bail!("model callee PC differs"),
            };Ok(true)
        })();self.call=Some(c);self.validate()?;result
    }
    pub fn at_cut(&self,q:&serde_json::Value)->bool{self.call.as_ref().is_some_and(|c|q["pc"]==c.clear.unwrap_or(c.pc))}
    pub fn compare(&self,q:&serde_json::Value)->Result<()>{
        self.validate()?;let e=&q["late_objects"];let raw=decode(&e["model"]["raw"])?;
        ensure!(e["model"]["pointer"]==self.pointer&&word(&raw,0x60)==self.backend&&word(&raw,0x5c)==self.last_hresult&&raw[0x64]==self.active&&raw[0xd0]==self.dirty&&word(&raw,0xd8)==self.viewport,"model retained fields differ");
        let pc=q["pc"].as_u64().context("model native PC absent")?as u32;
        if matches!(pc,0x5ce76a|0x5ce7c7){
            ensure!(q["registers"]["ecx"]==self.pointer,"native model receiver differs");
            if pc==0x5ce76a{let stack=decode(&e["stack"]["raw"])?;ensure!(stack.len()>=8&&self.call.as_ref().is_some_and(|c|word(&stack,4)as u8==c.reset),"native model reset argument differs");}
        }
        if matches!(pc,0x5ce77d|0x5ce79c|0x5ce7b0|0x5ce7c4|0x5ce7d6|0x5ce7de){ensure!(self.call.as_ref().and_then(|c|c.result).is_some_and(|r|q["registers"]["eax"]==r),"model HRESULT/register return differs");}
        for(list,offset)in self.lists.iter().zip([0x1d4,0x1e0]){
            let r=e["drawable_lists"].as_array().context("native drawable lists absent")?.iter().find(|r|r["offset"]==offset).context("native drawable list absent")?;
            ensure!(r["first"]==list.header[0]&&r["last"]==list.header[1]&&decode(&r["raw"])?==list.entries.iter().flat_map(|(_,p)|p.to_le_bytes()).collect::<Vec<_>>(),"computed drawable list differs");
        }Ok(())
    }
}
