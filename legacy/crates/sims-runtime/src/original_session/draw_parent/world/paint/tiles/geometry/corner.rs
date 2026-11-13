//!53c227 ordinary corner drawing. Bind static B identities and the CURRENT
//!Scene corner; later original observations are comparisons only.
use super::*;
mod cap;
use sims_sim::lot_presenter::{level_geometry,room_masks::Camera,walls::{construction::Graph,Corner}};
use sims_vm::scheduler::clock::draw::context::{Frame as ContextFrame,LightUpdate,placement::PlacementUpdate};
use crate::original_session::draw_parent::world::paint::row::{GeometrySideJob,GeometrySideParams};
type Row=crate::original_session::draw_parent::world::paint::row::State;
const SETUP:&str="48bdda26252d0600a22b49fc681cb982a98cff89d1b8279b6aee8731bde22531";
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
enum Stage{Entry,Style,StyleReturned,ViewGate,Mode,Zoom,ZoomReturned,Terrain,TerrainReturned,Project,Projected,Rotate,Rotated,Center,Centered,Getter,GetterReturned,Place,Placing,Placed,Room,RoomReturned,Light,LightReturned,LightStore,Lighting,LightStored,DefaultLight,Dispatch,AfterSides,Epilogue,Returned,SidePalette,SidePaletteReturned,Side,Cap}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
pub struct State{
 model:u32,view:u32,point:[u8;3],rotation:u8,corner:Corner,mode:u8,view_visibility:u8,room0:Option<u32>,stage:Stage,
 table:Option<u32>,zoom:Option<u8>,terrain:Option<[u8;4]>,screen:Option<[i32;2]>,rotated:Option<[u8;3]>,physical:Option<[i32;2]>,placement:Option<PlacementUpdate>,light:Option<u32>,update:Option<LightUpdate>,room:Option<u32>,
 #[serde(default,skip_serializing_if="Option::is_none")]
 side:Option<GeometrySideJob>,
 #[serde(default,skip_serializing_if="Option::is_none")]
 cap_input:Option<cap::Input>,
 #[serde(default,skip_serializing_if="Option::is_none")]
 cap:Option<cap::Job>,
}
impl State{
 pub fn import(encoded:&[u8],setup:&[u8],request:&level_geometry::Request,graph:&Graph)->Result<Self>{
  ensure!(sims_fixtures::digest(encoded)==INPUT&&sims_fixtures::digest(setup)==SETUP&&request.kind==level_geometry::Kind::Second,"corner input/kind differs");
  let v:serde_json::Value=serde_json::from_slice(encoded)?;let s:serde_json::Value=serde_json::from_slice(setup)?;let m=Memory::new(&v)?;let sm=Memory::new(&s)?;graph.validate().map_err(anyhow::Error::msg)?;
  ensure!(v["tick"]==89073&&s["tick"]==89073&&v["executes_target_code"]==false&&s["executes_target_code"]==false,"corner input epoch differs");
  let group=v["geometry"]["groups"].as_array().context("corner vector absent")?.iter().find(|g|g["model_offset"]==8).context("corner vector offset absent")?;
  let entries=group["records"].as_array().context("corner identities absent")?;let index=entries.iter().position(|e|e["pointer"]==request.model).context("corner native identity absent")?;
  let floor=&graph.state.floors[0];ensure!(floor.corners.len()==entries.len(),"corner scene/vector extent differs");let id=floor.corners[index];let raw=m.read(request.model,0x100)?;
  ensure!(decode(&entries[index]["raw"])?==raw&&graph.geometry.positions[id].map(|v|v as u8)==raw[..3],"corner B identity/point differs");
  // B's masks/caps may already be stale. Retain current computed Scene values.
  let a=&s["setup"];let view=a["process_child_4"].as_u64().context("corner view identity absent")?as u32;let vr=sm.read(view,0x80)?;let table=word(&vr,0);let vt=sm.read(table,0x100)?;
  ensure!(word(&vt,0x24)==0x4a6e11&&sims_fixtures::digest(&sm.read(0x4a6e11,7)?) == "b26137ce17a8386215bb1fde2b53f227d965b04ec16598700dc9cd28de01928f"&&word(&vt,0x14)==0x4aaf3c&&sims_fixtures::digest(&sm.read(0x4aaf3c,6)?) == "41d2c23fed43a1e28c11cda3efe2d5f2c1983f13669f2da6c9b83c8622baf8f3","corner original virtual getters differ");
  let rooms=a["room_directory"].as_array().context("corner room directory absent")?;let room0=rooms.iter().find(|r|r["id"]==0).map(|r|r["room"].as_u64().context("corner room pointer absent").map(|x|x as u32)).transpose()?;
  let value=Self{model:request.model,view,point:raw[..3].try_into()?,rotation:graph.rotation,corner:graph.state.corners[id].clone(),mode:raw[0x50],view_visibility:vr[0x50],room0,stage:Stage::Entry,table:None,zoom:None,terrain:None,screen:None,rotated:None,physical:None,placement:None,light:None,update:None,room:None,side:None,cap_input:cap::Input::import(&sm,&s,graph,id,request.model,&raw)?,cap:None};value.validate()?;Ok(value)
 }
 pub fn enrich(&mut self,fresh:Self)->Result<()>{ensure!(self.model==fresh.model&&self.view==fresh.view&&self.point==fresh.point&&self.rotation==fresh.rotation&&self.corner==fresh.corner&&self.mode==fresh.mode&&self.view_visibility==fresh.view_visibility&&self.room0==fresh.room0,"corner supplemental scene/B inputs differ");if let Some(new)=&fresh.cap_input{self.cap_input.as_mut().context("corner cap supplemental input absent")?.enrich(new)?;if let Some(cap)=&mut self.cap{cap.enrich(new)?;}}self.validate()}
 pub fn model(&self)->u32{self.model}
 pub fn at_entry(&self)->bool{self.stage==Stage::Entry}
 pub fn complete(&self)->bool{self.stage==Stage::Returned}
 pub fn validate(&self)->Result<()>{ensure!(self.rotation<4&&self.model!=0&&self.view!=0,"corner identity invalid");let n=self.stage as u8;
  for(present,from,name)in[(self.table.is_some(),Stage::StyleReturned,"lookup"),(self.zoom.is_some(),Stage::ZoomReturned,"zoom"),(self.terrain.is_some(),Stage::TerrainReturned,"terrain"),(self.screen.is_some(),Stage::Projected,"screen"),(self.rotated.is_some(),Stage::Rotated,"rotation"),(self.physical.is_some(),Stage::Centered,"physical"),(self.placement.is_some(),Stage::Place,"placement")]{ensure!(present==(n>=from as u8),"corner {name} lifetime differs");}
  if self.stage!=Stage::DefaultLight{ensure!(self.light.is_some()==(n>=Stage::LightReturned as u8)&&self.update.is_some()==(n>=Stage::Lighting as u8),"corner light lifetime differs");}
if let Some(c)=&self.cap{c.validate()?;ensure!(self.stage==Stage::Cap||matches!(self.stage,Stage::Epilogue|Stage::Returned)&&c.complete(),"corner cap escaped caller");}
if let Some(j)=&self.side{j.validate()?;ensure!(matches!(self.stage,Stage::SidePalette|Stage::SidePaletteReturned|Stage::Side),"corner C3 escaped its caller");}
if let Some(l)=&self.update{l.validate().map_err(anyhow::Error::msg)?;}Ok(())}
 pub fn advance(&mut self,sim:&sims_sim::Simulation,camera:&Camera,context:&mut ContextFrame,row:Option<&mut Row>,clip:[i32;4])->Result<bool>{
  self.validate()?;
  if matches!(self.stage,Stage::Placing|Stage::Placed){self.placement.as_ref().context("corner placement continuation absent")?.validate_words(context.state().words()).map_err(anyhow::Error::msg)?;}
  match self.stage{
   Stage::Entry=>self.stage=Stage::Style,
   Stage::Style=>{self.table=Some(self.view.wrapping_add(0xb8));self.stage=Stage::StyleReturned},
   Stage::StyleReturned=>self.stage=Stage::ViewGate,
   Stage::ViewGate=>{ensure!(self.view_visibility==0,"corner visibility callback is pending");self.stage=Stage::Mode},
   Stage::Mode=>{ensure!(self.mode==0,"corner cached projection mode is pending");self.stage=Stage::Zoom},
   Stage::Zoom=>{self.zoom=Some(camera.zoom);self.stage=Stage::ZoomReturned},
   Stage::ZoomReturned=>self.stage=Stage::Terrain,
   Stage::Terrain=>{let lot=sim.architecture.as_ref().and_then(|a|a.original.as_ref()).context("corner terrain owner absent")?;self.terrain=Some(sims_sim::lot_presenter::room_masks::terrain_corners(lot.size,&lot.heights,self.point.map(|v|v as i8),self.rotation).map_err(anyhow::Error::msg)?);self.stage=Stage::TerrainReturned},
   Stage::TerrainReturned=>self.stage=Stage::Project,
   Stage::Project=>{ensure!(usize::from(camera.zoom)<camera.floor_heights.len(),"corner projection height absent");self.screen=Some(level_geometry::project(camera,self.point));self.stage=Stage::Projected},
   Stage::Projected=>self.stage=Stage::Rotate,
   Stage::Rotate=>{let[x,y,z]=self.point;self.rotated=Some(match 4u8.wrapping_sub(self.rotation)&3{1=>[63u8.wrapping_sub(y),x,z],2=>[63u8.wrapping_sub(x),63u8.wrapping_sub(y),z],3=>[y,63u8.wrapping_sub(x),z],_=>[x,y,z]});self.stage=Stage::Rotated},
   Stage::Rotated=>self.stage=Stage::Center,
   Stage::Center=>{let[x,y,_]=self.rotated.unwrap().map(|v|v as i8);self.physical=Some([(i32::from(y)<<4)|8,(i32::from(x)<<4)|8]);self.stage=Stage::Centered},
   Stage::Centered=>self.stage=Stage::Getter,
   Stage::Getter=>self.stage=Stage::GetterReturned,
   Stage::GetterReturned=>{self.placement=Some(PlacementUpdate::new(self.physical.unwrap(),i32::from(self.point[2]as i8),-1,self.rotation));self.stage=Stage::Place},
   Stage::Place=>self.stage=Stage::Placing,
   Stage::Placing=>{let p=self.placement.as_mut().unwrap();let progress=row.context("corner shared placement owner absent")?.advance_geometry_placement(p,sim,context)?;ensure!(progress||p.complete(),"corner placement stalled");if p.complete(){self.stage=Stage::Placed}},
   Stage::Placed=>self.stage=Stage::Room,
   Stage::Room=>{self.room=if sim.room_environment.as_ref().context("corner room owner absent")?.manager.rooms.contains_key(&0){Some(self.room0.context("corner native room0 identity absent")?)}else{None};self.stage=Stage::RoomReturned},
   Stage::RoomReturned=>self.stage=if self.room.is_some(){Stage::Light}else{Stage::DefaultLight},
   Stage::Light=>{self.light=Some(sim.room_environment.as_ref().context("corner light owner absent")?.manager.rooms.get(&0).context("corner room0 disappeared")?.effective_light().to_bits());self.stage=Stage::LightReturned},
   Stage::LightReturned=>self.stage=Stage::LightStore,
   Stage::LightStore=>{self.update=Some(LightUpdate::new(self.light.unwrap()));self.stage=Stage::Lighting},
   Stage::Lighting=>{let l=self.update.as_mut().unwrap();context.advance_light(l).map_err(anyhow::Error::msg)?;if l.complete(){self.stage=Stage::LightStored}},
   Stage::LightStored=>self.stage=Stage::Dispatch,
   Stage::DefaultLight=>anyhow::bail!("corner default-light reset is pending"),
   Stage::Dispatch=>{let m=self.corner.masks;
    ensure!(m[0]&!m[2]&!m[3]==0,"corner first-side raster dispatch is pending");
    let flags=m[1]&!m[2]&!m[3];
    if flags==0{self.stage=Stage::AfterSides}else{
     ensure!(flags&0x40==0||self.point[2]!=2,"corner nonzero C3 depth bias is pending");
     let z=self.zoom.unwrap();let origin=self.screen.unwrap();let offset=row.context("corner side depth owner absent")?.geometry_corner_vertical_offset(z)?;
     let screen=[origin[0].wrapping_sub(8i32.wrapping_shl(u32::from(z))).wrapping_sub(1),origin[1].wrapping_sub(i32::from(self.terrain.unwrap()[1]).wrapping_shl(u32::from(z)+1)).wrapping_sub(4i32.wrapping_shl(u32::from(z))).wrapping_sub(offset)];
     self.side=Some(GeometrySideJob::new_corner(GeometrySideParams{point:self.point,rotation:self.rotation,zoom:z,height:0,flags,screen,light:self.light.unwrap(),clip,alternate:0},self.model)?);self.stage=Stage::SidePalette;
    }},
   Stage::SidePalette=>self.stage=Stage::SidePaletteReturned,
   Stage::SidePaletteReturned=>self.stage=Stage::Side,
   Stage::Side=>{let side=self.side.as_mut().context("corner C3 absent")?;let progress=row.context("corner shared C3 owner absent")?.advance_geometry_side(side,sim,context,camera)?;ensure!(progress||side.complete(),"corner C3 stalled");if side.complete(){self.side=None;self.stage=Stage::AfterSides}},
   Stage::AfterSides=>{if self.corner.valid.iter().any(|v|*v!=0){self.cap=Some(cap::Job::new(self.cap_input.clone().context("corner cap B inputs absent")?,self.view,self.point,self.screen.unwrap(),self.zoom.unwrap(),self.terrain.unwrap()[1],clip)?);self.stage=Stage::Cap}else{self.stage=Stage::Epilogue}},
   Stage::Cap=>{let cap=self.cap.as_mut().context("corner cap frame absent")?;let progress=cap.advance(sim,camera,self.rotation,context,row.context("corner cap shared row absent")?)?;ensure!(progress||cap.complete(),"corner cap stalled");if cap.complete(){self.stage=Stage::Epilogue}},
   Stage::Epilogue=>self.stage=Stage::Returned,
   Stage::Returned=>return Ok(false),
  }
  self.validate()?;Ok(true)
 }
 pub fn at_cut(&self,pc:u32)->bool{if self.stage==Stage::Cap{return self.cap.as_ref().is_some_and(|c|c.at_cut(pc))}if self.stage==Stage::Side{return self.side.as_ref().is_some_and(|s|s.at_cut(pc))}if self.stage==Stage::SidePalette{return pc==0x53c6ab}if self.stage==Stage::SidePaletteReturned{return matches!(pc,0x53c6b0|0x53c6b2)}matches!((pc,self.stage),(0x53c227,Stage::Entry)|(0x53c250,Stage::Style)|(0x53c253,Stage::StyleReturned)|(0x53c298,Stage::ViewGate)|(0x53c306,Stage::Mode)|(0x53c30b,Stage::Zoom)|(0x53c30e,Stage::ZoomReturned)|(0x53c330,Stage::Terrain)|(0x53c335,Stage::TerrainReturned)|(0x53c346,Stage::Project)|(0x53c34b,Stage::Projected)|(0x53c35f,Stage::Rotate)|(0x53c364,Stage::Rotated)|(0x53c385,Stage::Center)|(0x53c38a,Stage::Centered)|(0x53c38d,Stage::Getter)|(0x53c392,Stage::GetterReturned)|(0x53c394,Stage::Place)|(0x53c399,Stage::Placed)|(0x53c3a7,Stage::Room)|(0x53c3ac,Stage::RoomReturned)|(0x53c3b2,Stage::Light)|(0x53c3b7,Stage::LightReturned)|(0x53c3bd,Stage::LightStore)|(0x53c3c2,Stage::LightStored)|(0x53c3c6,Stage::DefaultLight)|(0x53c3cb,Stage::Dispatch)|(0x53c6b7,Stage::AfterSides)|(0x53ca56,Stage::Epilogue)|(0x5404a7,Stage::Returned))}
 pub fn compare(&self,q:&serde_json::Value,graphics:GeometryView<'_>)->Result<()>{
  self.validate()?;if self.stage==Stage::Cap||self.stage==Stage::Epilogue&&self.cap.is_some(){return self.cap.as_ref().context("corner cap comparison frame absent")?.compare(q,graphics.map(|g|g.0))}if self.stage==Stage::Side{let(row,context)=graphics.context("corner C3 comparison owner absent")?;return row.compare_geometry_side(self.side.as_ref().unwrap(),q,context)}let pc=q["pc"].as_u64().context("corner PC absent")?as u32;ensure!(self.at_cut(pc),"corner stage differs");if self.complete(){ensure!(q["geometry_body_event"]["deallocated_child_locals"]==true,"corner return inspected dead locals");return Ok(())}
  let n=decode(&q["corner_model"]["raw"])?;ensure!(q["corner_model"]["pointer"]==self.model&&n[..3]==self.point&&n[8]==self.corner.base&&n[9..13]==self.corner.masks&&n[0x24..0x26]==self.corner.valid.map(u8::from)&&n[0x28..0x3c]==self.corner.caps[0]&&n[0x3c..0x50]==self.corner.caps[1]&&n[0x50]==self.mode,"corner current model inputs differ");
  if self.at_entry(){return Ok(())}
  let p=&q["corner_prefix"];let bp=p["bp"].as_u64().context("corner allocated frame absent")?as u32;let f=decode(&p["frame"]["raw"])?;let stack=decode(&p["stack"]["raw"])?;ensure!(p["frame"]["address"]==bp-0xa0&&f.len()==0xc0&&q["registers"]["edi"]==self.model,"corner frame/model aliases differ");
  if !matches!(self.stage,Stage::Style|Stage::StyleReturned){ensure!(f[0xa0-0x18..0xa0-0x15]==self.rotated.unwrap_or(self.point),"corner local point differs");ensure!(word(&f,0xa0-0x30)==self.table.unwrap(),"corner lookup owner local differs");}
  if let Some(t)=self.terrain{if self.stage!=Stage::TerrainReturned{ensure!(f[0xa0-0x2c..0xa0-0x28]==t,"corner terrain local differs");}}
  if let Some([x,y])=self.screen{ensure!(word(&f,0xa0-0x20)as i32==x&&word(&f,0xa0-0x34)as i32==y,"corner projected screen differs");}
  if let Some(p)=self.physical{ensure!((word(&f,0xa0-0x58)as i32,word(&f,0xa0-0x54)as i32)==(p[0],p[1]),"corner physical point differs");}
  match self.stage{
   Stage::Style|Stage::Zoom=>ensure!(q["registers"]["ecx"]==self.view,"corner virtual receiver differs"),
   Stage::StyleReturned=>ensure!(q["registers"]["eax"]==self.table.unwrap(),"corner lookup getter result differs"),
   Stage::ZoomReturned=>ensure!(q["registers"]["eax"]==self.zoom.unwrap(),"corner zoom result differs"),
   Stage::Terrain=>ensure!(word(&stack,0)==bp-0x50&&stack[4..7]==self.point,"corner terrain arguments differ"),
   Stage::TerrainReturned=>ensure!(q["registers"]["eax"]==bp-0x50&&f[0xa0-0x50..0xa0-0x4c]==self.terrain.unwrap(),"corner terrain result differs"),
   Stage::Project=>ensure!([word(&stack,0),word(&stack,4),word(&stack,8)]==[bp-0x20,bp-0x34,self.model],"corner projection arguments differ"),
   Stage::Rotate=>ensure!(q["registers"]["ecx"]==bp-0x18&&word(&stack,0)==u32::from(4u8.wrapping_sub(self.rotation)&3),"corner inverse rotation arguments differ"),
   Stage::Center=>ensure!(q["registers"]["ecx"]==bp-0x18&&[word(&stack,0),word(&stack,4),word(&stack,8),word(&stack,12)]==[bp-0x58,u32::from(self.point[2]),q["context"]["pointer"].as_u64().unwrap()as u32,u32::MAX],"corner center/placement arguments differ"),
   Stage::Centered=>ensure!(q["registers"]["eax"]==bp-0x58,"corner center return pointer differs"),
   Stage::Getter=>ensure!(word(&stack,0)==bp-0x58&&q["registers"]["ecx"]==q["context"]["pointer"],"corner placement getter differs"),
   Stage::GetterReturned|Stage::Place=>ensure!(q["registers"]["ecx"]==q["context"]["pointer"],"corner placement context differs"),
   Stage::Room=>ensure!(word(&stack,0)==0,"corner room0 lookup argument differs"),
   Stage::RoomReturned=>ensure!(q["registers"]["eax"]==self.room.unwrap_or(0),"corner room0 lookup result differs"),
   Stage::Light=>ensure!(q["registers"]["ecx"]==self.room.unwrap(),"corner light receiver differs"),
   Stage::SidePaletteReturned=>ensure!(q["registers"]["eax"]==graphics.context("corner palette owner absent")?.0.geometry_side_cache()?,"corner palette getter result differs"),
   Stage::LightStore=>ensure!(word(&stack,0)==self.light.unwrap()&&q["registers"]["ecx"]==q["context"]["pointer"],"corner light argument differs"),
   _=>(),
  }
  Ok(())
 }
}
