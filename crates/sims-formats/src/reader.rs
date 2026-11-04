use std::fmt;

#[derive(Debug, Clone)]
pub struct Error {
    pub offset: usize,
    pub message: String,
}
impl Error {
    pub fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte 0x{:X}: {}", self.offset, self.message)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    pub fn fail<T>(&self, message: impl Into<String>) -> Result<T> {
        Err(Error::new(self.pos, message))
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| Error::new(self.pos, "size overflow"))?;
        let bytes = self.data.get(self.pos..end).ok_or_else(|| {
            Error::new(
                self.pos,
                format!("need {n} bytes; only {} remain", self.remaining()),
            )
        })?;
        self.pos = end;
        Ok(bytes)
    }
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub fn be16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn be32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn cstr(&mut self) -> Result<&'a [u8]> {
        let n = self.data[self.pos..]
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| Error::new(self.pos, "unterminated string"))?;
        let s = self.take(n)?;
        self.skip(1)?;
        Ok(s)
    }
    pub fn count(&self, count: usize, minimum: usize) -> Result<()> {
        if count > self.remaining() / minimum {
            self.fail(format!("count {count} exceeds remaining data"))
        } else {
            Ok(())
        }
    }
}
pub(crate) fn western(bytes: &[u8]) -> String {
    encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned()
}
