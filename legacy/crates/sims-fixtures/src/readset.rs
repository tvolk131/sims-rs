//! Borrowed SIMS-MEM-1 capture ranges. Integrity/coverage is not native admission.
//! Unobserved holes never become zeroes. Overlapping bytes must agree. Both the
//! original record order and a sorted, disjoint borrowed coverage index survive.
use anyhow::{Context, Result, ensure};

const MAGIC: &[u8] = b"SIMS-MEM-1\0";
const MAX_RECORDS: usize = 16384;
const MAX_RECORD: usize = 1 << 20;
const MAX_OBSERVED: usize = 32 << 20;
const MAX_FILE: usize = MAGIC.len() + 4 + MAX_RECORDS * 8 + MAX_OBSERVED;

#[derive(Clone, Copy, Debug)]
pub struct Record<'a> {
    pub address: u32,
    pub bytes: &'a [u8],
}
impl Record<'_> {
    fn end(&self) -> u64 {
        u64::from(self.address) + self.bytes.len() as u64
    }
}

pub struct Readset<'a> {
    records: Vec<Record<'a>>,
    coverage: Vec<Record<'a>>,
    observed_bytes: usize,
    source_sha256: String,
}
impl<'a> Readset<'a> {
    pub fn parse(raw: &'a [u8]) -> Result<Self> {
        ensure!(
            raw.len() <= MAX_FILE && raw.starts_with(MAGIC),
            "invalid bounded readset header"
        );
        let mut at = MAGIC.len();
        let word = |at: &mut usize| -> Result<u32> {
            let bytes = raw.get(*at..*at + 4).context("partial readset word")?;
            *at += 4;
            Ok(u32::from_le_bytes(bytes.try_into()?))
        };
        let count = word(&mut at)? as usize;
        ensure!(count <= MAX_RECORDS, "readset record count exceeds bound");
        let mut records = Vec::with_capacity(count);
        let mut observed_bytes = 0usize;
        for _ in 0..count {
            let address = word(&mut at)?;
            let length = word(&mut at)? as usize;
            observed_bytes = observed_bytes
                .checked_add(length)
                .context("readset extent overflow")?;
            ensure!(
                length <= MAX_RECORD
                    && observed_bytes <= MAX_OBSERVED
                    && (length == 0 || address != 0)
                    && u64::from(address) + length as u64 <= 1u64 << 32,
                "readset range exceeds native/observation bounds"
            );
            let bytes = raw.get(at..at + length).context("partial readset body")?;
            at += length;
            records.push(Record { address, bytes });
        }
        ensure!(at == raw.len(), "unframed readset suffix");
        let mut sorted = records.clone();
        sorted.sort_by_key(|r| (r.address, std::cmp::Reverse(r.bytes.len())));
        let mut coverage: Vec<Record<'a>> = Vec::with_capacity(count);
        for r in sorted.into_iter().filter(|r| !r.bytes.is_empty()) {
            let covered_end = coverage.last().map_or(0, Record::end);
            if u64::from(r.address) < covered_end {
                let overlap = (r.end().min(covered_end) - u64::from(r.address)) as usize;
                let mut position = u64::from(r.address);
                let mut source = 0;
                let mut i = coverage
                    .partition_point(|s| u64::from(s.address) <= position)
                    .checked_sub(1)
                    .context("unobserved overlap prefix")?;
                while source < overlap {
                    let s = coverage.get(i).context("unobserved overlap")?;
                    ensure!(
                        u64::from(s.address) <= position && position < s.end(),
                        "unobserved overlap hole"
                    );
                    let offset = (position - u64::from(s.address)) as usize;
                    let n = (s.bytes.len() - offset).min(overlap - source);
                    ensure!(
                        s.bytes[offset..offset + n] == r.bytes[source..source + n],
                        "conflicting observed overlap"
                    );
                    source += n;
                    position += n as u64;
                    i += 1;
                }
            }
            if r.end() > covered_end {
                let skip = covered_end.saturating_sub(u64::from(r.address)) as usize;
                coverage.push(Record {
                    address: r
                        .address
                        .checked_add(skip.try_into()?)
                        .context("readset suffix overflow")?,
                    bytes: &r.bytes[skip..],
                });
            }
        }
        Ok(Self {
            records,
            coverage,
            observed_bytes,
            source_sha256: crate::digest(raw),
        })
    }

    pub fn records(&self) -> &[Record<'a>] {
        &self.records
    }
    pub fn observed_bytes(&self) -> usize {
        self.observed_bytes
    }
    /// Exact ordered capture identity. Equality is not source admission or a
    /// claim that independently captured ranges belong to one native stop.
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    pub fn coverage(&self) -> &[Record<'a>] {
        &self.coverage
    }

    /// Copy only the requested result. The capture itself stays borrowed.
    pub fn read(&self, address: u32, count: usize) -> Result<Vec<u8>> {
        self.read_inner(address, count)
            .with_context(|| format!("observed native read {address:#010x}+{count:#x}"))
    }
    fn read_inner(&self, address: u32, count: usize) -> Result<Vec<u8>> {
        ensure!(
            count <= MAX_OBSERVED && u64::from(address) + count as u64 <= 1u64 << 32,
            "invalid bounded native read request"
        );
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut i = self
            .coverage
            .partition_point(|s| s.address <= address)
            .checked_sub(1)
            .context("unobserved native range")?;
        let mut position = u64::from(address);
        let mut result = Vec::with_capacity(count);
        while result.len() < count {
            let s = self
                .coverage
                .get(i)
                .context("unobserved native range suffix")?;
            ensure!(
                u64::from(s.address) <= position && position < s.end(),
                "unobserved native range hole"
            );
            let offset = (position - u64::from(s.address)) as usize;
            let n = (s.bytes.len() - offset).min(count - result.len());
            result.extend_from_slice(&s.bytes[offset..offset + n]);
            position += n as u64;
            i += 1;
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture(rows: &[(u32, &[u8])]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend((rows.len() as u32).to_le_bytes());
        for (address, data) in rows {
            out.extend(address.to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(*data);
        }
        out
    }
    #[test]
    fn adjacent_overlapping_unsorted_ranges_preserve_bytes_and_record_order() -> Result<()> {
        let raw = capture(&[(103, b"def"), (100, b"abc"), (101, b"bcde"), (110, b"xy")]);
        let r = Readset::parse(&raw)?;
        assert_eq!(r.source_sha256(), crate::digest(&raw));
        assert_eq!(r.read(100, 6)?, b"abcdef");
        assert_eq!(r.read(102, 3)?, b"cde");
        assert!(r.read(104, 7).is_err());
        assert!(r.read(99, 1).is_err());
        assert_eq!(r.records()[0].address, 103);
        assert_eq!(r.observed_bytes(), 12);
        assert!(r.coverage().iter().all(|s| s.bytes.as_ptr() >= raw.as_ptr()
            && (s.bytes.as_ptr() as usize + s.bytes.len()) <= raw.as_ptr() as usize + raw.len()));
        Ok(())
    }
    #[test]
    fn conflicting_contained_or_cross_segment_overlap_is_rejected() {
        for rows in [
            vec![(100, b"abc".as_slice()), (102, b"Xd")],
            vec![(100, b"abc".as_slice()), (103, b"def"), (101, b"bcXe")],
        ] {
            assert!(Readset::parse(&capture(&rows)).is_err());
        }
    }
    #[test]
    fn malformed_truncated_overflowing_and_unframed_inputs_fail() {
        let valid = capture(&[(100, b"abc")]);
        for len in 0..valid.len() {
            assert!(Readset::parse(&valid[..len]).is_err());
        }
        let mut suffix = valid;
        suffix.push(0);
        assert!(Readset::parse(&suffix).is_err());
        assert!(Readset::parse(&capture(&[(u32::MAX, b"ab")])).is_err());
        assert!(Readset::parse(&capture(&[(0, b"a")])).is_err());
        let empty = capture(&[]);
        let r = Readset::parse(&empty).unwrap();
        assert!(r.read(100, 1).is_err());
        assert_eq!(r.read(0, 0).unwrap(), b"");
    }

    #[test]
    fn borrowed_coverage_matches_independent_byte_oracle_across_overlaps_and_holes() -> Result<()> {
        use std::collections::BTreeMap;
        let mut seed = 0xe8946251u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let bank: Vec<_> = (0..512).map(|i| (i * 13) as u8).collect();
        for _ in 0..128 {
            let mut rows = Vec::new();
            let mut oracle = BTreeMap::new();
            for _ in 0..64 {
                let start = next() as usize % 480;
                let length = next() as usize % 33;
                let address = 0x10000 + start as u32;
                let data = &bank[start..start + length];
                for (i, byte) in data.iter().enumerate() {
                    oracle.insert(address + i as u32, *byte);
                }
                rows.push((address, data));
            }
            let raw = capture(&rows);
            let r = Readset::parse(&raw)?;
            for _ in 0..256 {
                let address = 0x10000 + next() % 512;
                let length = next() as usize % 65;
                let expected: Option<Vec<_>> = (0..length)
                    .map(|i| oracle.get(&(address + i as u32)).copied())
                    .collect();
                match expected {
                    Some(bytes) => assert_eq!(r.read(address, length)?, bytes),
                    None => assert!(r.read(address, length).is_err()),
                }
            }
        }
        let final_byte = capture(&[(u32::MAX, b"z")]);
        let r = Readset::parse(&final_byte)?;
        assert_eq!(r.read(u32::MAX, 1)?, b"z");
        assert!(r.read(u32::MAX, 2).is_err());
        Ok(())
    }
}
