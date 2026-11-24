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

/// Writes the same packed field stream (the stream's packed mode, set by
/// 0x4644f5). A zero is one clear bit (0x46460a). Any other value is a set
/// bit, a two-bit selector, and the value's low bits at the selected width
/// (0x46463d, 0x46464f). The selector is the first width whose sign-extended
/// range holds the value (0x46465b compares the value against each width's
/// mask, `-1 << (width - 1)`, from 0x464512), and the widest otherwise. Bits
/// go most significant first; the last byte's unused bits stay clear.
#[derive(Debug, Default, Clone)]
pub struct FieldWriter {
    bytes: Vec<u8>,
    bit: usize,
}

impl FieldWriter {
    pub fn new() -> Self {
        Self::default()
    }
    fn bits(&mut self, count: usize, value: u32) {
        for index in (0..count).rev() {
            if self.bit.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (value >> index) & 1 != 0 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.bit % 8);
            }
            self.bit += 1;
        }
    }
    fn signed(&mut self, value: i32, widths: [usize; 4]) {
        if value == 0 {
            self.bits(1, 0);
            return;
        }
        let selector = widths
            .iter()
            .position(|&width| {
                let mask = -1i32 << (width - 1);
                value & mask == mask || value & mask == 0
            })
            .unwrap_or(3);
        self.bits(1, 1);
        self.bits(2, selector as u32);
        let width = widths[selector];
        let low = if width == 32 {
            value as u32
        } else {
            (value as u32) & ((1 << width) - 1)
        };
        self.bits(width, low);
    }
    pub fn i16(&mut self, value: i16) {
        self.signed(i32::from(value), [5, 8, 13, 16]);
    }
    pub fn i32(&mut self, value: i32) {
        self.signed(value, [6, 11, 21, 32]);
    }
    pub fn u32(&mut self, value: u32) {
        self.i32(value as i32);
    }
    pub fn finish(self) -> Vec<u8> {
        self.bytes
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

    #[test]
    fn the_writer_picks_the_narrowest_width_and_reads_back() {
        let shorts = [
            0, -1, 15, -16, 16, 127, -128, 128, 4095, -4096, 4096, 32767, -32768,
        ];
        let ints = [
            0,
            31,
            -32,
            32,
            1023,
            -1024,
            1024,
            1 << 20,
            -(1 << 20),
            i32::MAX,
            i32::MIN,
        ];
        let mut writer = FieldWriter::new();
        for &value in &shorts {
            writer.i16(value);
        }
        for &value in &ints {
            writer.i32(value);
        }
        let bytes = writer.finish();
        let mut reader = Fields::new(&bytes, 0);
        for &value in &shorts {
            assert_eq!(reader.i16().unwrap(), value);
        }
        for &value in &ints {
            assert_eq!(reader.i32().unwrap(), value);
        }
        assert!(reader.remaining_bits() < 8);

        // The same bits as the reader's fixture: zero, short -1, short 15.
        let mut writer = FieldWriter::new();
        writer.i16(0);
        writer.i16(-1);
        writer.i16(15);
        assert_eq!(writer.finish(), [0x4f, 0xc7, 0x80]);
        // 16 needs the second width (8 bits): 1 01 00010000.
        let mut writer = FieldWriter::new();
        writer.i16(16);
        assert_eq!(writer.finish(), [0xa2, 0x00]);
    }
}
