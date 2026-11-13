//! 60505f: retained DirectDraw copy. Only the platform HRESULT is supplied.
//! Lost-surface restoration and the engine's software fallback remain explicit
//! children at their original call sites; neither is treated as success.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blit {
    pc: u32,
    source_address: u32,
    source: [i32; 4],
    destination: [i32; 4],
    secondary: Option<bool>,
    pending: Option<Request>,
    hresult: Option<u32>,
    result: Option<u8>,
}
impl Blit {
    pub fn new(source_address: u32, source: [i32; 4], destination: [i32; 4]) -> Self {
        Self { pc: 0x60505f, source_address, source, destination,
            secondary: None, pending: None, hresult: None, result: None }
    }
    pub fn pc(&self) -> u32 { self.pc }
    pub fn complete(&self) -> bool { self.pc == 0 }
    pub fn pending(&self) -> Option<&Request> { self.pending.as_ref() }
    pub fn result(&self) -> Option<u8> { self.result }
    fn request(&self, destination: &Fields, source: &Fields) -> Result<Request, String> {
        let offset = if self.secondary.ok_or("copy interface branch absent")? { 8 } else { 4 };
        Ok(Request::DirectDrawBlit {
            interface: word(&destination.raw, offset), destination: self.destination,
            source_interface: word(&source.raw, offset), source: self.source,
            flags: word(&source.raw, 0x158),
            effects_address: self.source_address.wrapping_add(0xf4),
            effects: source.raw[0xf4..0x158].to_vec(),
        })
    }
    pub fn validate(&self, destination: &Fields, source: &Fields) -> Result<(), String> {
        destination.validate()?; source.validate()?;
        if self.complete() && self.result.is_none() {
            return Err("copy result lifetime differs".into());
        }
        if let Some(branch) = self.secondary {
            if branch != (word(&destination.raw, 8) != 0) {
                return Err("copy interface owner changed".into());
            }
        }
        if matches!(self.pc, 0x605091 | 0x605106) {
            if self.pending.as_ref() != Some(&self.request(destination, source)?) {
                return Err("copy DirectDraw arguments differ".into());
            }
        } else if self.pending.is_some() {
            return Err("copy platform child lost caller".into());
        }
        Ok(())
    }
    pub fn directdraw_return(&mut self, destination: &Fields, source: &Fields, hresult: u32) -> Result<(), String> {
        self.validate(destination, source)?;
        if self.pending.is_none() { return Err("copy is not awaiting DirectDraw".into()); }
        self.pending = None; self.hresult = Some(hresult);
        self.pc = match self.pc { 0x605091 => 0x605094, 0x605106 => 0x605109,
            _ => return Err("copy platform return address differs".into()) };
        self.validate(destination, source)
    }
    /// No backend fields are written by this function's observed success path.
    pub fn advance(&mut self, destination: &Fields, source: &Fields) -> Result<bool, String> {
        self.validate(destination, source)?;
        if self.pending.is_some() || self.complete() { return Ok(false); }
        self.pc = match self.pc {
            // EH-frame setup only: no engine/backend field publication.
            0x60505f => 0x605064, 0x605064 => 0x605069,
            0x605069 => 0x60506a, 0x60506a => 0x60506b, 0x60506b => 0x60506c,
            0x60506c => 0x60506e,
            0x60506e => { self.secondary = Some(word(&destination.raw, 8) != 0); 0x605071 },
            0x605071 => 0x605073,
            0x605073 => if self.secondary.unwrap() { 0x605075 } else { 0x6050e7 },
            0x605075 => 0x605078, 0x605078 => 0x60507a, 0x60507a => 0x605080,
            0x605080 => 0x605081, 0x605081 => 0x605087, 0x605087 => 0x60508a,
            0x60508a => 0x60508d, 0x60508d => 0x605090,
            0x605090 => { self.pending = Some(self.request(destination, source)?); 0x605091 },
            0x6050e7 => 0x6050ea, 0x6050ea => 0x6050ed, 0x6050ed => 0x6050f3,
            0x6050f3 => 0x6050f5, 0x6050f5 => 0x6050f6, 0x6050f6 => 0x6050fc,
            0x6050fc => 0x6050ff, 0x6050ff => 0x605102, 0x605102 => 0x605105,
            0x605105 => { self.pending = Some(self.request(destination, source)?); 0x605106 },
            0x605094 => 0x605099, 0x605109 => 0x60510e,
            0x605099 => if self.hresult == Some(0x887601c2) { 0x60509f } else { 0x60515d },
            0x60510e => if self.hresult == Some(0x887601c2) { 0x605110 } else { 0x60515d },
            0x60515d => 0x60515f,
            0x60515f => if self.hresult == Some(0) { 0x605171 } else { 0x605161 },
            0x605171 => { self.result = Some(1); 0x605173 },
            0x605173 => 0x605176, 0x605176 => 0x605177, 0x605177 => 0x605178,
            0x605178 => 0x605179, 0x605179 => 0x605180, 0x605180 => 0x605181,
            0x605181 => 0,
            // These paths need real retained platform/engine children.
            0x60509f | 0x605110 | 0x605161 => return Ok(false),
            _ => return Err("copy PC requires its retained child".into()),
        };
        self.validate(destination, source)?; Ok(true)
    }
}
