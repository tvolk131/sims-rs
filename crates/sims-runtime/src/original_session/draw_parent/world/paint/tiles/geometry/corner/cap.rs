//!53c6b7..53ca56: cap bounds, named OffsetRect and ordered clip stores.
//!Descriptor flags/patterns come from current Scene; identities/side selector
//!come from B. Visible material/raster children remain explicit services.
use super::*;
mod draw;
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
pub struct Input {
    model:u32, pointer:u32, edge:u32, segment:u32, selected:u32,
    descriptor:[u8;20], flags:u8, pattern:u16, upper:u8,
    #[serde(default,skip_serializing_if="Option::is_none")]
    draw:Option<draw::Input>,
}
impl Input {
    pub(super) fn enrich(&mut self,new:&Self)->Result<()>{let(mut a,mut b)=(self.clone(),new.clone());a.draw=None;b.draw=None;ensure!(a==b,"cap supplemental static owners differ");if let Some(old)=&self.draw{ensure!(Some(old)==new.draw.as_ref(),"cap supplemental draw inputs differ");}else{self.draw=new.draw.clone();}Ok(())}
    pub fn import(memory:&Memory,source:&serde_json::Value,graph:&Graph,id:usize,model:u32,corner_raw:&[u8])->Result<Option<Self>> {
        let c=&graph.state.corners[id];
        let index=if c.valid[0]!=0{0}else if c.valid[1]!=0{1}else{return Ok(None)};
        let descriptor=c.caps[index];let direction=word(&descriptor,8)as usize;
        ensure!(direction<4,"corner cap direction unsupported");
        let edge_id=c.edges[direction].context("corner cap edge absent")?;
        let e=&graph.state.edges[edge_id];let first=e.ends[0]==id;
        ensure!(first||e.ends[1]==id,"corner cap edge does not own endpoint");
        let n=graph.state.floors[0].edges.iter().position(|v|*v==edge_id).context("corner cap edge identity absent")?;
        let row=&source["first_vector"]["models"][n];let edge=row["pointer"].as_u64().context("corner native cap edge absent")?as u32;
        let raw=memory.read(edge,0x100)?;ensure!(decode(&row["raw"])?==raw&&word(&raw,0x10)==e.kind as u32,"corner cap edge B alias differs");
        let selected=word(&raw,0x18);ensure!(selected<2,"corner selected descriptor side unsupported");
        let s=if first{e.segments.first()}else{e.segments.last()}.context("corner cap endpoint descriptor absent")?;
        let segment=if first{word(&raw,0x30)}else{word(&raw,0x34).checked_sub(6).context("corner cap segment underflow")?};
        ensure!(word(corner_raw,0x10+direction*4)==edge,"corner native edge link differs");
        Ok(Some(Self{model,pointer:model+0x28+index as u32*20,edge,segment,selected,descriptor,flags:s.flags,pattern:s.patterns[selected as usize],upper:c.valid[1],draw:Some(draw::Input::import(memory,source,graph,id,s.style)?)}))
    }
}
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
enum Stage{Entry,ClipLeft,ClipRight,Getter,GetterReturned,Bounds,BoundsReturned,Point,PointReturned,Project,Projected,Offset,OffsetReturned,Overlap,Visible,RestoreFirst,RestoreSecond,Complete}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
pub struct Job {
    input:Input,stage:Stage,view:u32,point:[u8;3],origin:[i32;2],zoom:u8,terrain:u8,
    original_clip:[i32;4],clip:[i32;4],side:u32,context_zoom:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    draw:Option<draw::State>,
    rectangle:Option<[i32;4]>,translated:Option<[u8;3]>,screen:Option<[i32;2]>,delta:Option<[i32;2]>,overlaps:Option<bool>,
}
impl Job {
    pub fn new(input:Input,view:u32,point:[u8;3],origin:[i32;2],zoom:u8,terrain:u8,clip:[i32;4])->Result<Self>{
        ensure!((1..=3).contains(&zoom),"corner cap zoom unsupported");
        let direction=word(&input.descriptor,8);let mut side=u32::from(input.upper);
        if input.flags&1!=0 && input.flags&6!=0 {side=u32::from((input.flags>>if direction==0||direction==3{2}else{1})&1);}
        Ok(Self{input,stage:Stage::Entry,view,point,origin,zoom,terrain,original_clip:clip,clip,side,context_zoom:None,draw:None,rectangle:None,translated:None,screen:None,delta:None,overlaps:None})
    }
    pub fn enrich(&mut self,new:&Input)->Result<()>{self.input.enrich(new)?;self.validate()}
    pub fn complete(&self)->bool{self.stage==Stage::Complete}
    pub fn validate(&self)->Result<()>{
        ensure!(self.input.model!=0&&self.input.edge!=0&&self.input.segment!=0&&self.input.selected<2&&(1..=3).contains(&self.zoom),"corner cap owner invalid");
        if let Some(d)=&self.draw{d.validate()?;ensure!(matches!(self.stage,Stage::Visible|Stage::RestoreFirst|Stage::RestoreSecond|Stage::Complete),"visible cap escaped caller");}
        let n=self.stage as u8;
        for(present,from,name)in[(self.context_zoom.is_some(),Stage::GetterReturned,"context zoom"),(self.rectangle.is_some(),Stage::BoundsReturned,"bounds"),(self.translated.is_some(),Stage::PointReturned,"point"),(self.screen.is_some(),Stage::Projected,"screen"),(self.delta.is_some(),Stage::Offset,"delta"),(self.overlaps.is_some(),Stage::Visible,"overlap")]{
            // Clipped and visible paths join different stages after Overlap.
            let expected=if name=="overlap"{matches!(self.stage,Stage::Visible|Stage::RestoreFirst|Stage::RestoreSecond|Stage::Complete)}else{n>=from as u8};
            ensure!(present==expected,"corner cap {name} lifetime differs");
        }
        Ok(())
    }
    pub fn advance(&mut self,sim:&sims_sim::Simulation,camera:&Camera,rotation:u8,context:&mut ContextFrame,row:&mut Row)->Result<bool>{
        self.validate()?;
        match self.stage {
            Stage::Entry=>self.stage=Stage::ClipLeft,
            Stage::ClipLeft=>{let span=(word(&self.input.descriptor,4)as i32).wrapping_shl(u32::from(self.zoom)-1);let x=if self.input.descriptor[16]!=0{self.origin[0].wrapping_sub(1)}else{self.origin[0].wrapping_sub(span)};if self.clip[0]<x{self.clip[0]=i32::from(x as i16);}row.request_geometry_clip(self.clip,0)?;self.stage=Stage::ClipRight},
            Stage::ClipRight=>{let span=(word(&self.input.descriptor,4)as i32).wrapping_shl(u32::from(self.zoom)-1);let x=if self.input.descriptor[16]!=0{span.wrapping_add(self.origin[0]).wrapping_sub(1)}else{self.origin[0]};if self.clip[2]>x{self.clip[2]=i32::from(x as i16);}row.request_geometry_clip(self.clip,1)?;self.stage=Stage::Getter},
            Stage::Getter=>{self.context_zoom=Some(context.state().words()[8]);self.stage=Stage::GetterReturned},
            Stage::GetterReturned=>self.stage=Stage::Bounds,
            Stage::Bounds=>{let z=self.context_zoom.unwrap();ensure!(z<4,"corner cap bounds context zoom unsupported");let h=*camera.floor_heights.get(z as usize).context("corner cap bounds floor height absent")?;self.rectangle=Some(level_geometry::segment_bounds(word(&self.input.descriptor,12),z as u8,h));self.stage=Stage::BoundsReturned},
            Stage::BoundsReturned=>self.stage=Stage::Point,
            Stage::Point=>{self.translated=Some(std::array::from_fn(|i|self.point[i].wrapping_add(self.input.descriptor[i])));self.stage=Stage::PointReturned},
            Stage::PointReturned=>self.stage=Stage::Project,
            Stage::Project=>{self.screen=Some(level_geometry::project(camera,self.translated.unwrap()));self.stage=Stage::Projected},
            Stage::Projected=>{let[x,y]=self.screen.unwrap();self.delta=Some([x,y.wrapping_sub(i32::from(self.terrain).wrapping_shl(u32::from(self.zoom)+1))]);self.stage=Stage::Offset},
            Stage::Offset=>{let d=self.delta.unwrap();let r=self.rectangle.as_mut().unwrap();for(i,v)in r.iter_mut().enumerate(){*v=v.wrapping_add(d[i%2]);}self.stage=Stage::OffsetReturned},
            Stage::OffsetReturned=>self.stage=Stage::Overlap,
            Stage::Overlap=>{let overlap=level_geometry::segment_overlaps(self.rectangle.unwrap(),self.clip);self.overlaps=Some(overlap);self.stage=if overlap{Stage::Visible}else{Stage::RestoreFirst}},
            Stage::Visible=>{if let Some(d)=&mut self.draw{let progress=d.advance(sim,context,row)?;ensure!(progress||d.complete(),"visible cap stalled");if d.complete(){self.stage=Stage::RestoreFirst}}else{self.draw=Some(draw::State::new(self.input.draw.clone().context("corner cap draw B metadata is unobserved")?,&self.input,self.translated.unwrap(),rotation,self.zoom,self.terrain,self.side,self.rectangle.unwrap(),self.clip,self.view,self.point[2])?)}},
            Stage::RestoreFirst=>{row.request_geometry_clip(self.original_clip,0)?;self.stage=Stage::RestoreSecond},
            Stage::RestoreSecond=>{row.request_geometry_clip(self.original_clip,1)?;self.stage=Stage::Complete},
            Stage::Complete=>return Ok(false),
        }
        self.validate()?;Ok(true)
    }
    pub fn at_cut(&self,pc:u32)->bool{if self.stage==Stage::Visible{if let Some(d)=&self.draw{return d.at_cut(pc)}}matches!((self.stage,pc),(Stage::Getter,0x53c79a)|(Stage::GetterReturned,0x53c79f)|(Stage::Bounds,0x53c7ab)|(Stage::BoundsReturned,0x53c7b0)|(Stage::Point,0x53c7b7)|(Stage::PointReturned,0x53c7bc)|(Stage::Project,0x53c7e2)|(Stage::Projected,0x53c7e7)|(Stage::Offset,0x53c830)|(Stage::OffsetReturned,0x53c836)|(Stage::Visible,0x53c890)|(Stage::RestoreFirst,0x53ca46)|(Stage::Complete,0x53ca56))}
    pub fn compare(&self,q:&serde_json::Value,row:Option<&Row>)->Result<()>{
        self.validate()?;let pc=q["pc"].as_u64().context("corner cap PC absent")?as u32;ensure!(self.at_cut(pc),"corner cap stage differs");
        let p=&q["corner_prefix"];let bp=p["bp"].as_u64().context("corner cap frame absent")?as u32;let f=decode(&p["frame"]["raw"])?;let stack=decode(&p["stack"]["raw"])?;
        ensure!(q["corner_model"]["pointer"]==self.input.model&&q["registers"]["ebx"]==self.input.pointer,"corner cap descriptor aliases differ");
        ensure!([word(&f,0xa0-0x58),word(&f,0xa0-0x54)]==[u32::from(self.original_clip[1]as u16)|(u32::from(self.original_clip[0]as u16)<<16),u32::from(self.original_clip[3]as u16)|(u32::from(self.original_clip[2]as u16)<<16)],"corner cap saved clip words differ");
        if let Some(d)=&self.draw{d.compare(q,row)?;return Ok(())}
        ensure!(word(&f,0xa0-0x30)==u32::from(self.input.pattern)&&word(&f,0xa0-0x3c)==self.side,"corner cap descriptor aliases differ");
        let n=self.stage as u8;
        if let Some(r)=self.rectangle {ensure!((0..4).all(|i|word(&f,0xa0-0x68+i*4)as i32==r[i]),"corner cap bounds differ");}
        if let Some(point)=self.translated {ensure!(f[0xa0-0x14..0xa0-0x11]==point,"corner cap point result differs");if n>=Stage::Project as u8{ensure!(f[0xa0-0x18..0xa0-0x15]==point,"corner cap translated point differs");}}
        if let Some([x,y])=self.screen{ensure!(word(&f,0xa0-0x44)as i32==x&&word(&f,0xa0-0x48)as i32==y,"corner cap projection differs");}
        match self.stage {
            Stage::Getter=>ensure!(q["registers"]["ecx"]==q["context"]["pointer"],"corner cap context getter differs"),
            Stage::GetterReturned=>ensure!(q["registers"]["eax"]==self.context_zoom.unwrap(),"corner cap zoom result differs"),
            Stage::Bounds=>ensure!([word(&stack,0),word(&stack,4),word(&stack,8)]==[bp-0x68,bp-0x50,self.context_zoom.unwrap()]&&word(&f,0xa0-0x50)==word(&self.input.descriptor,12)&&q["registers"]["ecx"]==self.view,"corner cap bounds ABI differs"),
            Stage::Point=>ensure!(q["registers"]["ecx"]==self.input.model&&[word(&stack,0),word(&stack,4)]==[bp-0x14,self.input.pointer],"corner cap point ABI differs"),
            Stage::PointReturned=>ensure!(q["registers"]["eax"]==bp-0x14,"corner cap point return alias differs"),
            Stage::Project=>ensure!([word(&stack,0),word(&stack,4),word(&stack,8)]==[bp-0x44,bp-0x48,bp-0x18]&&q["registers"]["ecx"]==self.view,"corner cap projection ABI differs"),
            Stage::Offset=>{let d=self.delta.unwrap();ensure!([word(&stack,0),word(&stack,4),word(&stack,8)]==[bp-0x68,d[0]as u32,d[1]as u32],"corner cap OffsetRect ABI differs");},
            Stage::RestoreFirst=>ensure!(self.overlaps==Some(false),"corner clipped cap branch differs"),
            Stage::Visible=>ensure!(self.overlaps==Some(true)&&[word(&stack,0),word(&stack,4),word(&stack,8),word(&stack,12)]==[u32::from(self.input.pattern),u32::from(self.zoom),bp-0x50,bp-0x38],"corner visible cap material ABI differs"),
            _=>(),
        }
        if matches!(self.stage,Stage::RestoreFirst|Stage::Visible){ensure!(word(&f,0xa0-0x6c)as i32==self.clip[3]&&word(&f,0xa0-0x70)as i32==self.clip[2],"corner cap clip comparison locals differ");}
        Ok(())
    }
}
