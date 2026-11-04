//!604f3e. Platform HRESULTs are external; lock-count changes execute here.
use super::*;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unlock {
    pc: u32,
    eax: Option<u32>,
    interface: Option<u32>,
    argument: Option<u32>,
    pending: Option<Request>,
    result: Option<u8>,
}
impl Default for Unlock { fn default()->Self{Self::new()} }
impl Unlock {
    pub fn new()->Self{Self{pc:0x604f3e,eax:None,interface:None,argument:None,pending:None,result:None}}
    pub fn pending(&self)->Option<&Request>{self.pending.as_ref()}
    pub fn complete(&self)->bool{self.pc==0}
    pub fn result(&self)->Option<u8>{self.result}
    pub fn at_native_cut(&self,pc:u32)->bool{!self.complete()&&self.pc==pc}
    pub fn validate(&self,fields:&Fields)->Result<(),String>{
        fields.validate()?;
        if self.complete()&&self.result.is_none(){return Err("backend unlock lost its result".into());}
        match &self.pending{
            Some(Request::DirectDrawUnlock{interface,argument}) if self.pc==0x604f7f&&Some(*interface)==self.interface&&Some(*argument)==self.argument&&*interface!=0=>(),
            Some(Request::Error{hresult}) if self.pc==0x604f9c&&Some(*hresult)==self.eax=>(),
            None if !matches!(self.pc,0x604f7f|0x604f9c)=>(),
            _=>return Err("backend unlock child lifetime differs".into()),
        }
        Ok(())
    }
    pub fn directdraw_return(&mut self,fields:&Fields,hresult:u32)->Result<(),String>{
        self.validate(fields)?;
        if !matches!(self.pending,Some(Request::DirectDrawUnlock{..})){return Err("backend unlock is not awaiting DirectDraw".into());}
        self.pending=None;self.eax=Some(hresult);self.pc=0x604f85;self.validate(fields)
    }
    pub fn advance(&mut self,fields:&mut Fields)->Result<bool,String>{
        self.validate(fields)?;
        if self.pending.is_some()||self.complete(){return Ok(false);}
        self.pc=match self.pc{
            0x604f3e=>{self.eax=Some(0x6644b4);0x604f43},
            //The EH frame helper does not change engine/backend fields. Its
            //register scratch is not manufactured as an engine return value.
            0x604f43=>{self.eax=None;0x604f48},
            0x604f48=>0x604f4b,0x604f4b=>0x604f4c,0x604f4c=>0x604f4e,
            0x604f4e=>{self.eax=Some(fields.locks());0x604f54},
            0x604f54=>0x604f56,
            0x604f56=>if self.eax==Some(0){0x604fbd}else{0x604f58},
            0x604f58=>0x604f5b,
            0x604f5b=>if self.eax.unwrap()<=1{0x604f66}else{0x604f5d},
            0x604f5d=>{self.eax=Some(self.eax.unwrap().wrapping_sub(1));0x604f5e},
            0x604f5e=>{set(&mut fields.raw,0x160,self.eax.unwrap());0x604f64},
            0x604f64=>0x604fbd,
            0x604f66=>{self.interface=Some(word(&fields.raw,8));self.eax=self.interface;0x604f69},
            0x604f69=>0x604f6b,
            0x604f6b=>if self.interface==Some(0){0x604f73}else{0x604f6d},
            0x604f6d=>0x604f6f,
            0x604f6f=>{self.argument=Some(0);0x604f71},
            0x604f71=>0x604f7e,
            0x604f73=>{self.interface=Some(word(&fields.raw,4));self.eax=self.interface;0x604f76},
            0x604f76=>{self.argument=Some(fields.buffer());0x604f7c},
            0x604f7c=>0x604f7e,
            0x604f7e=>{self.pending=Some(Request::DirectDrawUnlock{interface:self.interface.ok_or("unlock interface absent")?,argument:self.argument.ok_or("unlock argument absent")?});0x604f7f},
            0x604f85=>0x604f87,
            0x604f87=>if self.eax==Some(0){0x604fb6}else{0x604f89},
            0x604f89=>0x604f8c,0x604f8c=>0x604f90,0x604f90=>0x604f92,0x604f92=>0x604f95,
            0x604f95=>{self.pending=Some(Request::Error{hresult:self.eax.ok_or("unlock HRESULT absent")?});0x604f9c},
            0x604fb6=>{set(&mut fields.raw,0x160,0);0x604fbd},
            0x604fbd=>{self.result=Some(1);0x604fbf},
            0x604fbf=>0x604fc2,0x604fc2=>0x604fc3,0x604fc3=>0x604fca,0x604fca=>0x604fcb,
            0x604fcb=>0,
            _=>return Err("backend unlock PC requires its retained child".into()),
        };
        self.validate(fields)?;Ok(true)
    }
}
