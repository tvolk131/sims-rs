//! Zero-allocation streaming [`postcard::ser_flavors::Flavor`] that feeds
//! serialized bytes directly into a [`sha2::Sha256`] hasher.
//!
//! Produces the exact same SHA-256 digest as hashing `postcard::to_allocvec`
//! without allocating an intermediate `Vec<u8>`.

use postcard::ser_flavors::Flavor;
use serde::Serialize;
use sha2::{Digest, Sha256};

const STAGING_LEN: usize = 256;

/// Streaming `postcard` serialization flavor feeding directly into [`Sha256`].
pub struct Sha256Flavor<'a> {
    hasher: &'a mut Sha256,
    buf: [u8; STAGING_LEN],
    pos: usize,
}

impl<'a> Sha256Flavor<'a> {
    #[inline]
    pub fn new(hasher: &'a mut Sha256) -> Self {
        Self {
            hasher,
            buf: [0u8; STAGING_LEN],
            pos: 0,
        }
    }

    #[inline]
    fn flush(&mut self) {
        if self.pos > 0 {
            self.hasher.update(&self.buf[..self.pos]);
            self.pos = 0;
        }
    }
}

impl Flavor for Sha256Flavor<'_> {
    type Output = ();

    #[inline]
    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        self.buf[self.pos] = data;
        self.pos += 1;
        if self.pos == STAGING_LEN {
            self.flush();
        }
        Ok(())
    }

    #[inline]
    fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let avail = STAGING_LEN - self.pos;
        if data.len() < avail {
            self.buf[self.pos..self.pos + data.len()].copy_from_slice(data);
            self.pos += data.len();
            return Ok(());
        }
        self.flush();
        if data.len() >= STAGING_LEN {
            self.hasher.update(data);
        } else {
            self.buf[..data.len()].copy_from_slice(data);
            self.pos = data.len();
        }
        Ok(())
    }

    #[inline]
    fn finalize(mut self) -> postcard::Result<Self::Output> {
        self.flush();
        Ok(())
    }
}

/// Streams `value`'s canonical `postcard` encoding directly into `hasher`
/// with zero heap allocations.
#[inline]
pub fn hash_postcard<T: Serialize + ?Sized>(value: &T, hasher: &mut Sha256) {
    postcard::serialize_with_flavor(value, Sha256Flavor::new(hasher))
        .expect("streaming postcard serialization into Sha256 is infallible");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_sha256_matches_to_allocvec_across_boundaries() {
        #[derive(Serialize)]
        struct Sample {
            tag: u32,
            bytes: Vec<u8>,
            nums: Vec<i32>,
            text: String,
        }

        for len in [0usize, 1, 127, 255, 256, 257, 512, 4097] {
            let sample = Sample {
                tag: len as u32,
                bytes: (0..len).map(|i| (i * 31 + 7) as u8).collect(),
                nums: (0..len.min(300)).map(|i| i as i32 * -17).collect(),
                text: "x".repeat(len.min(600)),
            };
            let encoded = postcard::to_allocvec(&sample).unwrap();
            let expected: [u8; 32] = Sha256::digest(&encoded).into();

            let mut hasher = Sha256::new();
            hash_postcard(&sample, &mut hasher);
            let actual: [u8; 32] = hasher.finalize().into();

            assert_eq!(actual, expected, "mismatch at len={len}");
        }
    }
}
