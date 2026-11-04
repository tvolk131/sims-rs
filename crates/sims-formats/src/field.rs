//! MSB-first, signed field streams used by original saved objects.
//! A zero occupies one bit; nonzero values have a two-bit width selector.
use crate::reader::western;
use crate::{Error, Result};

pub struct Fields<'a> {
    data: &'a [u8],
    bit: usize,
    /// Byte offset in the enclosing stream, for aligned strings.
    origin: usize,
}

impl<'a> Fields<'a> {
    pub fn new(data: &'a [u8], origin: usize) -> Self {
        Self {
            data,
            bit: 0,
            origin,
        }
    }
    pub fn bit_position(&self) -> usize {
        self.bit
    }
    pub fn byte_position(&self) -> usize {
        self.bit.div_ceil(8)
    }
    pub fn remaining_bits(&self) -> usize {
        self.data.len() * 8 - self.bit
    }
    fn bits(&mut self, count: usize) -> Result<u32> {
        if count > self.remaining_bits() {
            return Err(Error::new(self.byte_position(), "truncated field stream"));
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | u32::from((self.data[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(value)
    }
    fn signed(&mut self, widths: [usize; 4]) -> Result<i32> {
        if self.bits(1)? == 0 {
            return Ok(0);
        }
        let width = widths[self.bits(2)? as usize];
        let value = self.bits(width)?;
        Ok(((value << (32 - width)) as i32) >> (32 - width))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.signed([5, 8, 13, 16])? as i16)
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(self.i16()? as u16)
    }
    pub fn i32(&mut self) -> Result<i32> {
        self.signed([6, 11, 21, 32])
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(self.i32()? as u32)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.signed([2, 4, 6, 8])? as u8)
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub fn count(&mut self, minimum_bits: usize) -> Result<usize> {
        let count = self.i32()?;
        if count < 0 || count as usize > self.remaining_bits() / minimum_bits.max(1) {
            return Err(Error::new(
                self.byte_position(),
                format!("invalid field count {count}"),
            ));
        }
        Ok(count as usize)
    }
    /// Discard a partial byte, read a NUL-terminated Windows-1252 string,
    /// then align to an even offset in the enclosing stream.
    pub fn string(&mut self) -> Result<String> {
        let start = self.byte_position();
        let len = self.data[start..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| Error::new(start, "unterminated field string"))?;
        let end = start + len + 1;
        let padded = end + ((self.origin + end) & 1);
        if padded > self.data.len() {
            return Err(Error::new(end, "missing field string padding"));
        }
        self.bit = padded * 8;
        Ok(western(&self.data[start..start + len]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_bits_signed_values_and_string_alignment() {
        // zero, short -1 (1 00 11111), short 15 (1 00 01111), padding.
        let data = [0x4f, 0xc7, 0x80, b'A', 0, 0, 0];
        let mut r = Fields::new(&data, 0);
        assert_eq!(r.i16().unwrap(), 0);
        assert_eq!(r.i16().unwrap(), -1);
        assert_eq!(r.i16().unwrap(), 15);
        assert_eq!(r.string().unwrap(), "A");
        assert_eq!(r.byte_position(), 6);
        assert_eq!(r.i32().unwrap(), 0);
        let mut odd = Fields::new(&[b'A', 0, 0], 1);
        assert_eq!(odd.string().unwrap(), "A");
        assert_eq!(odd.byte_position(), 3);
        assert!(odd.i16().is_err());
        assert!(Fields::new(&[0xff], 0).i32().is_err());
    }
}
