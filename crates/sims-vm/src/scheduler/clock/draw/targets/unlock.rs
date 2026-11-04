//!5ded38, including individually ordered plane writes after backend success.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneUnlock {
    pc:u32, flags:u32, mask:Option<u32>, backend_result:Option<u8>, result:Option<u8>,
}
impl PlaneUnlock {
    pub fn new(flags:u32)->Self{Self{pc:0x5ded38,flags,mask:None,backend_result:None,result:None}}
    pub fn complete(&self)->bool{self.pc==0}
    pub fn result(&self)->Option<u8>{self.result}
    pub fn at_native_cut(&self,pc:u32)->bool{!self.complete()&&pc==self.pc}
    pub fn pending_backend(&self)->bool{self.pc==0x5ded5a&&self.backend_result.is_none()}
    pub fn validate(&self)->Result<(),String>{
        if self.complete()&&self.result.is_none(){return Err("plane unlock result absent".into());}
        if matches!(self.pc,0x5ded5d|0x5ded5f)&&self.backend_result.is_none(){return Err("plane unlock backend result absent".into());}
        Ok(())
    }
    pub fn backend_return(&mut self,value:u8)->Result<(),String>{
        self.validate()?;if !self.pending_backend(){return Err("plane unlock backend not pending".into());}
        self.backend_result=Some(value);self.pc=0x5ded5d;self.validate()
    }
    pub fn advance(&mut self,fields:&mut Plane)->Result<bool,String>{
        self.validate()?;if self.pending_backend()||self.complete(){return Ok(false);}
        self.pc=match self.pc{
            0x5ded38=>0x5ded39,0x5ded39=>0x5ded3d,0x5ded3d=>0x5ded3e,0x5ded3e=>0x5ded41,0x5ded41=>0x5ded43,
            0x5ded43=>if self.flags==16{0x5ded45}else{0x5ded4b},
            0x5ded45=>{fields.flags&=!16;0x5ded49},0x5ded49=>0x5ded6e,
            0x5ded4b=>0x5ded4e,
            0x5ded4e=>if self.flags&0xcc!=0{0x5ded55}else{0x5ded50},
            0x5ded50=>0x5ded53,
            0x5ded53=>if self.flags&0x700==0{0x5ded69}else{0x5ded55},
            0x5ded55=>0x5ded58,0x5ded58=>0x5ded5a,
            0x5ded5d=>0x5ded5f,
            0x5ded5f=>if self.backend_result==Some(0){self.result=Some(0);0x5ded73}else{0x5ded61},
            0x5ded61=>{fields.buffer=0;0x5ded65},
            0x5ded65=>{fields.half_pitch=0;0x5ded69},
            0x5ded69=>{self.mask=Some(!self.flags);0x5ded6b},
            0x5ded6b=>{fields.flags&=self.mask.ok_or("plane unlock mask absent")?;0x5ded6e},
            0x5ded6e=>{fields.locks=fields.locks.wrapping_sub(1);0x5ded71},
            0x5ded71=>{self.result=Some(1);0x5ded73},
            0x5ded73=>0x5ded74,0x5ded74=>0x5ded75,0x5ded75=>0,
            _=>return Err("plane unlock PC differs".into()),
        };self.validate()?;Ok(true)
    }
}
