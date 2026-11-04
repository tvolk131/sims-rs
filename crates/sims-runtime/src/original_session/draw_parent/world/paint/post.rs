//!4a92a2: actual model-region registration/clip and explicit copy child.
use super::*;
use sims_vm::scheduler::clock::draw::regions;
const SOURCE:&str="1e93731059c1d73fce96f522656d579339275a76363a01e0a9feff1a042b155f";
fn decode(v:&serde_json::Value)->Result<Vec<u8>>{crate::original_session::completed_animations::bytes(v.as_str().context("post-window bytes absent")?)}
fn zero_word(v:&u32)->bool{*v==0}
fn zero_byte(v:&u8)->bool{*v==0}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
pub(super) struct Cursor {
    source:String,gate:u8,pc:u32,clock_calls:usize,test:Option<bool>,index:usize,
    region:Option<regions::Frame>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    clip:Option<[Option<i16>;4]>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    output_clip:Option<[Option<u32>;2]>,
    #[serde(default,skip_serializing_if="zero_word")]
    terrain_root:u32,
    #[serde(default,skip_serializing_if="zero_byte")]
    overlay:u8,
    #[serde(default,skip_serializing_if="Option::is_none")]
    terrain_pc:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    followup_pc:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    model_result:Option<u32>,
    #[serde(default,skip_serializing_if="is_zero_index")]
    object_index:usize,
}
fn is_zero_index(v:&usize)->bool{*v==0}
impl Cursor {
    pub fn import(encoded:&[u8])->Result<Self>{
        ensure!(sims_fixtures::digest(encoded)==SOURCE,"post-window B source differs");let v:serde_json::Value=serde_json::from_slice(encoded)?;
        ensure!(v["tick"]==89073&&v["executes_target_code"]==false,"post-window source epoch differs");let m=Memory::new(&v)?;
        let window=m.read(0x09e88960,0x520)?;
        Ok(Self{source:SOURCE.into(),gate:m.read(0x6e48c0,1)?[0],pc:0x4a92a2,clock_calls:0,test:None,index:0,region:None,clip:None,output_clip:None,terrain_root:word(&window,0x4b8),overlay:window[0x4c0],terrain_pc:None,followup_pc:None,model_result:None,object_index:0})
    }
    pub fn validate(&self,drawing:&drawing::State)->Result<()>{
        ensure!(self.source==SOURCE&&self.clock_calls<=2,"post-window input lifetime differs");
        ensure!(self.followup_pc.is_none()||self.pc==0x4a9418,"process drawing setter lost caller");
        if let Some(r)=&self.region{ensure!(self.pc==0x4a92f3,"region child lost bulk caller");r.validate(drawing.model_regions()).map_err(anyhow::Error::msg)?;}Ok(())
    }
    pub fn pc(&self)->u32{self.pc}
    pub fn clock(&self)->Option<(usize,u32)>{match self.pc{0x4a92c9|0x4a9361=>Some((self.clock_calls,0x6c7344)),0x4a9409=>Some((self.clock_calls,0x6c7330)),_=>None}}
    pub fn clock_return(&mut self)->Result<()>{ensure!(self.clock().is_some(),"post-window RDTSC not pending");self.clock_calls+=1;self.pc=match self.pc{0x4a92c9=>0x4a92ce,0x4a9361=>0x4a9366,0x4a9409=>0x4a940e,_=>unreachable!()};Ok(())}
    pub fn copy(&self,drawing:&drawing::State)->Option<(u32,usize)>{(self.pc==0x4a934c).then(||(word(&drawing.model_regions().header,0),drawing.model_regions().len()))}
    pub fn copy_return(&mut self)->Result<()>{ensure!(self.pc==0x4a934c,"region copy lost caller");self.pc=0x4a9351;Ok(())}
    pub fn front_pending(&self)->bool{self.pc==0x4a93b1}
    pub fn front_return(&mut self)->Result<()>{ensure!(self.front_pending(),"front acquisition lost caller");self.pc=0x4a93b6;Ok(())}
    pub fn end_pending(&self)->bool{self.pc==0x4a93fa}
    pub fn end_return(&mut self)->Result<()>{ensure!(self.end_pending(),"front EndFrame lost caller");self.pc=0x4a93ff;Ok(())}
    pub fn model_pending(&self)->Option<bool>{match self.pc{0x4a943a=>Some(true),0x4a9465=>Some(false),_=>None}}
    pub fn model_return(&mut self,result:u32)->Result<()>{ensure!(self.model_pending().is_some(),"model lost parent call");self.model_result=Some(result);self.pc=if self.pc==0x4a943a{0x4a943f}else{0x4a946a};Ok(())}
    pub fn drawable(&self,drawing:&drawing::State)->Option<Request>{
        if self.pc!=0x4a9458{return None;}let &(object,pointer)=drawing.model()?.lists[0].entries.get(self.object_index)?;
        Some(Request::DrawObject{object,pointer})
    }
    fn packed_clip(&self)->Result<[i16;4]>{let c=self.clip.context("packed clip absent")?;Ok([c[0].context("clip top absent")?,c[1].context("clip left absent")?,c[2].context("clip bottom absent")?,c[3].context("clip right absent")?])}
    pub fn advance(&mut self,drawing:&mut drawing::State,scene:&scene::State,profiler:&mut Buffer,record:Option<Record>,flags:u32,clip:[i32;4],surfaces:&mut surfaces::State)->Result<bool>{
        self.validate(drawing)?;
        if matches!(self.pc,0x4a941d..=0x4a9465)&&drawing.model().is_none(){return Ok(false);}
        if self.pc==0x4a9418{
            self.followup_pc=Some(match self.followup_pc{
                None=>0x584635,Some(0x584635)=>0x584639,
                Some(0x584639)=>{drawing.set_excluded_person(0);0x58463e},
                Some(0x58463e)=>{self.followup_pc=None;self.pc=0x4a941d;return Ok(true);},
                _=>anyhow::bail!("process drawing setter PC differs"),
            });return Ok(true);
        }
        if self.pc==0x4a93c9{
            self.terrain_pc=Some(match self.terrain_pc{
                None=>0x4b0c80,Some(0x4b0c80)=>0x4b0c85,Some(0x4b0c85)=>0x4b0c8a,
                Some(0x4b0c8a)=>0x4b0c90,Some(0x4b0c90)=>0x4b0c91,Some(0x4b0c91)=>0x4b0c92,
                Some(0x4b0c92)=>0x4b0c94,Some(0x4b0c94)=>0x4b0c96,Some(0x4b0c96)=>0x4b0c9c,
                Some(0x4b0c9c)=>if self.terrain_root==0{0x4b1065}else{0x4b0ca2},
                Some(0x4b1065)=>0x4b1068,Some(0x4b1068)=>0x4b1069,Some(0x4b1069)=>0x4b106a,
                Some(0x4b106a)=>0x4b1071,Some(0x4b1071)=>0x4b1072,
                Some(0x4b1072)=>{self.terrain_pc=None;self.pc=0x4a93ce;return Ok(true);},
                Some(0x4b0ca2)=>return Ok(false),
                _=>anyhow::bail!("terrain child PC differs"),
            });return Ok(true);
        }
        self.pc=match self.pc{
            0x4a92a2=>0x4a92a8,
            0x4a92a8=>if self.gate!=0{0x4a9358}else{0x4a92ae},
            0x4a92ae=>0x4a92b1,
            0x4a92b1=>if !drawing.front_present(){0x4a9358}else{0x4a92b7},
            0x4a92b7=>0x4a92ba,
            0x4a92ba=>if !drawing.back_present(){0x4a9358}else{0x4a92c0},
            0x4a92c0=>0x4a92c1,0x4a92c1=>0x4a92c6,0x4a92c6=>0x4a92c9,
            0x4a9358=>0x4a9359,0x4a9359=>0x4a935e,0x4a935e=>0x4a9361,
            0x4a92c9|0x4a9361|0x4a9409=>return Ok(false),
            0x4a92ce=>0x4a92cf,0x4a92cf=>0x4a92d1,0x4a9366=>0x4a9367,0x4a9367=>0x4a9369,
            0x4a92d1|0x4a9369=>{let r=record.context("post-window record absent")?;ensure!(r.name==0x6c7344&&r.argument==0,"post-window record arguments differ");profiler.append(r).map_err(anyhow::Error::msg)?;if self.pc==0x4a92d1{0x4a92d6}else{0x4a936e}},
            0x4a936e=>0x4a9371,
            0x4a92d6=>0x4a92d9,
            0x4a92d9=>{self.test=Some(flags&1!=0);0x4a92de},0x4a92de=>0x4a92e0,
            0x4a92e0=>if self.test.context("post-window region predicate absent")?{0x4a92e2}else{0x4a930d},
            0x4a92e2=>0x4a92e5,0x4a92e5=>0x4a92eb,0x4a92eb=>0x4a92ec,0x4a92ec=>0x4a92f1,0x4a92f1=>0x4a92f3,
            0x4a92f3=>{
                let(enabled,weight)=drawing.region_policy();
                if enabled==0||self.index==scene.regions.len(){0x4a92f8}
                else{
                    if self.region.is_none(){self.region=Some(regions::Frame::new(scene.regions.rectangle(self.index).map_err(anyhow::Error::msg)?));return Ok(true);}
                    let r=self.region.as_mut().unwrap();let list=drawing.model_regions_mut();
                    if let Some(request)=r.pending().cloned(){
                        let done=match request{regions::Request::Append{rectangle}=>list.append(rectangle),regions::Request::Coalesce=>list.coalesce(enabled,weight)}.map_err(anyhow::Error::msg)?;
                        if !done{return Ok(false);}r.acknowledge(list,&request).map_err(anyhow::Error::msg)?;
                    }else{r.advance(list,enabled).map_err(anyhow::Error::msg)?;}
                    if r.complete(){self.region=None;self.index+=1;}return Ok(true);
                }
            },
            0x4a92f8=>0x4a92fb,0x4a92fb=>0x4a92fe,0x4a92fe=>0x4a92ff,0x4a92ff=>0x4a9304,0x4a9304=>0x4a9306,
            0x4a9306=>{let(e,w)=drawing.region_policy();if !drawing.model_regions_mut().clip(clip,e,w).map_err(anyhow::Error::msg)?{return Ok(false);}0x4a930b},
            0x4a930b|0x4a930d=>0x4a9310,0x4a9310=>0x4a9313,0x4a9313=>0x4a9318,0x4a9318=>0x4a931b,0x4a931b=>0x4a931d,0x4a931d=>0x4a9323,
            0x4a9323=>if drawing.model_regions().len()==0{0x4a9371}else{0x4a9325},
            0x4a9325=>0x4a9328,0x4a9328=>0x4a932d,0x4a932d=>0x4a9330,0x4a9330=>0x4a9332,0x4a9332=>0x4a9335,0x4a9335=>0x4a9338,0x4a9338=>0x4a933d,0x4a933d=>0x4a9340,0x4a9340=>0x4a9343,0x4a9343=>0x4a9345,0x4a9345=>0x4a9347,0x4a9347=>0x4a934a,0x4a934a=>0x4a934b,0x4a934b=>0x4a934c,
            //The actual surface copy is a child, never a synthetic success.
            0x4a934c|0x4a93b1|0x4a93dc|0x4a93fa|0x4a943a|0x4a9458|0x4a9465|0x4a946a=>return Ok(false),
            0x4a9351=>0x4a9356,0x4a9356=>0x4a9371,
            0x4a9371=>0x4a9375,0x4a9375=>0x4a9378,
            0x4a9378=>{self.clip.get_or_insert([None;4])[0]=Some(clip[1]as i16);0x4a937c},
            0x4a937c=>0x4a9380,0x4a9380=>{self.clip.as_mut().context("packed clip absent")?[2]=Some(clip[3]as i16);0x4a9384},
            0x4a9384=>0x4a9387,0x4a9387=>0x4a938a,
            0x4a938a=>{self.clip.as_mut().context("packed clip absent")?[1]=Some(clip[0]as i16);0x4a938e},
            0x4a938e=>0x4a9392,0x4a9392=>{self.clip.as_mut().context("packed clip absent")?[3]=Some(clip[2]as i16);0x4a9396},
            0x4a9396=>0x4a9399,0x4a9399=>0x4a939c,
            0x4a939c|0x4a939e=>{let c=self.packed_clip()?;let i=usize::from(self.pc==0x4a939e);let b=[c[i*2].to_le_bytes(),c[i*2+1].to_le_bytes()].concat();self.output_clip.get_or_insert([None;2])[i]=Some(u32::from_le_bytes(b.try_into().unwrap()));if i==0{0x4a939e}else{0x4a93a1}},
            0x4a93a1=>{let c=self.packed_clip()?;surfaces.set_window_clip_half(0,[c[0],c[1]])?;0x4a93a6},
            0x4a93a6=>{let c=self.packed_clip()?;surfaces.set_window_clip_half(1,[c[2],c[3]])?;0x4a93ac},
            0x4a93ac=>0x4a93af,0x4a93af=>0x4a93b1,
            0x4a93b6=>0x4a93ba,0x4a93ba=>if surfaces.fields.active==0{0x4a93c0}else{0x4a93bc},
            0x4a93bc=>{surfaces.fields.phase=0;0x4a93c0},0x4a93c0=>0x4a93c3,0x4a93c3=>0x4a93c7,0x4a93c7=>0x4a93c9,
            0x4a93ce=>0x4a93d5,0x4a93d5=>if self.overlay==0{0x4a93f1}else{0x4a93d7},
            0x4a93d7=>0x4a93da,0x4a93da=>0x4a93dc,
            0x4a93f1=>0x4a93f4,0x4a93f4=>0x4a93f8,0x4a93f8=>0x4a93fa,
            0x4a93ff=>0x4a9401,0x4a9401=>0x4a9406,0x4a9406=>0x4a9409,
            0x4a940e=>0x4a940f,0x4a940f=>0x4a9411,
            0x4a9411=>{let r=record.context("post-EndFrame record absent")?;ensure!(r.name==0x6c7330&&r.argument==0,"post-EndFrame record arguments differ");profiler.append(r).map_err(anyhow::Error::msg)?;0x4a9416},
            0x4a9416=>0x4a9418,
            0x4a941d=>0x4a9424,0x4a9424=>0x4a9425,
            0x4a9425=>if drawing.model().unwrap().draw_enabled==0{0x4a946a}else{0x4a9427},
            0x4a9427=>0x4a942d,0x4a942d=>0x4a9433,
            0x4a9433=>if drawing.model().unwrap().lists[0].entries.is_empty(){0x4a946a}else{0x4a9435},
            0x4a9435=>0x4a9438,0x4a9438=>0x4a943a,
            0x4a943f=>0x4a9441,0x4a9441=>if(self.model_result.context("model HRESULT absent")?as i32)<0{0x4a946a}else{0x4a9443},
            0x4a9443=>{self.object_index=0;0x4a9449},0x4a9449=>0x4a944f,
            0x4a944f=>if self.object_index==drawing.model().unwrap().lists[0].entries.len(){0x4a9462}else{0x4a9451},
            0x4a9451=>0x4a9454,0x4a9454=>0x4a9456,0x4a9456=>0x4a9458,
            0x4a945d=>{self.object_index+=1;0x4a9460},0x4a9460=>0x4a9449,0x4a9462=>0x4a9465,
            _=>anyhow::bail!("post-window PC requires its actual child"),
        };self.validate(drawing)?;Ok(true)
    }
    pub fn compare(&self,q:&serde_json::Value,drawing:&drawing::State,profiler:&Buffer,record:Option<Record>)->Result<()>{
        self.validate(drawing)?;ensure!(q["pc"]==self.pc&&q["tick"]==89081,"post-window native phase differs");
        self.compare_owned(q,drawing,profiler,record)
    }
    pub fn compare_owned(&self,q:&serde_json::Value,drawing:&drawing::State,profiler:&Buffer,record:Option<Record>)->Result<()>{
        self.validate(drawing)?;ensure!(q["tick"]==89081,"post-window native epoch differs");
        let window=decode(&q["raw"])?;ensure!(window.len()>0x4c0&&word(&window,0x4b8)==self.terrain_root&&window[0x4c0]==self.overlay,"post-window terrain/overlay owners differ");
        let source=q["lists"].as_array().context("post-window native lists absent")?.iter().find(|r|r["label"]=="front_0x1ec").context("post-window native model list absent")?;
        ensure!(source["pointer"]==drawing.model_regions_pointer()&&decode(&source["raw"])?==drawing.model_regions().header&&decode(&source["backing"])?==drawing.model_regions().backing,"post-window model-region bytes differ");
        if self.pc==0x4a93b6{
            let output=self.output_clip.context("caller clip output absent")?;
            let expected=[output[0].context("caller first clip word absent")?.to_le_bytes(),output[1].context("caller second clip word absent")?.to_le_bytes()].concat();
            let base=q["world_stack"]["address"].as_u64().context("native caller stack absent")?as u32;let stack=decode(&q["world_stack"]["raw"])?;
            let bp=q["registers"]["ebp"].as_u64().context("native paint frame absent")?as u32;
            let at=bp.checked_add(12).and_then(|p|p.checked_sub(base)).context("native clip argument outside captured stack")?as usize;
            let argument=stack.get(at..at+4).context("native clip argument extent absent")?;let pointer=u32::from_le_bytes(argument.try_into().unwrap());
            let at=pointer.checked_sub(base).context("native clip output outside captured stack")?as usize;
            ensure!(stack.get(at..at+8)==Some(expected.as_slice()),"caller clip output differs");
        }
        if q.get("late_objects").is_some(){
            let e=&q["late_objects"];ensure!(drawing.excluded_person()==Some(word(&decode(&e["excluded_person"] )?,0)),"process drawing excluded-person differs");
            if let Some(m)=drawing.model(){m.compare(q)?;}
            if self.pc==0x4a943f{ensure!(self.model_result.is_some_and(|r|q["registers"]["eax"]==r),"parent model HRESULT differs");}
            if self.pc==0x4a9458&&q["pc"]==0x4a9458u32{
                let Some(Request::DrawObject{pointer,..})=self.drawable(drawing)else{anyhow::bail!("computed drawable absent")};
                ensure!(e["drawable"]["pointer"]==pointer,"computed native drawable dispatch differs");
                let stack=decode(&e["stack"]["raw"])?;
                //The second argument is the shared drawing-context address,
                //not its bitflags. Rust retains that value in the World owner.
                ensure!(word(&stack,0)==pointer&&q["context"]["pointer"]==word(&stack,4),"drawable context argument alias differs");
            }
        }
        if self.clock_calls>0{
            let native=decode(&q["profiler"]["used_raw"])?;let last=native.get(native.len().checked_sub(16).context("native record absent")?..).unwrap();
            let expected=Record::from_words(std::array::from_fn(|i|word(last,4*i)));
            ensure!(record==Some(expected)&&profiler.records().last()==Some(&expected),"post-window appended record differs");
        }
        Ok(())
    }
}
