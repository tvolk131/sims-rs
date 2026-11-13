//! Qualified constructor with owned Rust caption storage. The high-range IDs
//! are logical allocation handles, never predicted original heap addresses.
//! Only the written terminator is qualified; reserved bytes are unread scratch.
use super::*;
use sims_vm::scheduler::clock::draw::actions::record;
const FIRST: u32 = 0xf000_0000;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Storage {
    next: u32,
    allocations: BTreeMap<u32, Vec<u8>>,
    released: BTreeMap<u32, u32>,
}
pub(super) fn import(source: &[u8]) -> Result<Storage> {
    ensure!(
        sims_fixtures::digest(source)
            == "b8cf9f6be283efdf8780b49adc2fa0cf3667059aaed726f65ec24e3535a121fe",
        "record constructor source differs"
    );
    let v: serde_json::Value = serde_json::from_slice(source)?;
    ensure!(
        v["schema"] == 1
            && v["tick"] == 89073
            && v["executes_target_code"] == false
            && v["native_parity_credit"] == 0,
        "record constructor source epoch differs"
    );
    let memory = Memory::new(&v)?;
    for (address, bytes, hash) in [
        (
            0x4e877f,
            56,
            "bd16bb38e2d77c001171503f752708a5a7a1155dc4dcb2704f30c99a736cfa1d",
        ),
        (
            0x5b87b0,
            61,
            "81680338b7b9a1c67b5b8ab145f521c0c10f5574b8d89481b742ef36b9c5fd18",
        ),
        (
            0x401a66,
            50,
            "61aab7cda9134bb9a9ab2e235b77ded7dd828d7ae0c5719a8fd146daa43ca703",
        ),
        (
            0x401639,
            10,
            "51f02260d323b1414703bed8b8b47dadc6ba93f8cd7d5b374560bed263fb29c0",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&memory.read(address, bytes)?) == hash,
            "record constructor instruction source differs"
        );
    }
    Ok(Storage {
        next: FIRST,
        allocations: BTreeMap::new(),
        released: BTreeMap::new(),
    })
}
impl Storage {
    pub(super) fn validate_links(
        &self,
        output: &[actions::Record],
        temporary: Option<&actions::Record>,
    ) -> Result<()> {
        let mut owners = std::collections::BTreeSet::new();
        for raw in output {
            let [p, end, cap] = [16, 20, 24].map(|off| record::word(raw, off));
            if p < FIRST {
                continue;
            }
            let bytes = self
                .allocations
                .get(&p)
                .ok_or_else(|| anyhow::anyhow!("output caption allocation absent"))?;
            ensure!(
                p <= end
                    && end < cap
                    && cap - p == bytes.len() as u32
                    && bytes[(end - p) as usize] == 0
                    && owners.insert(p),
                "output caption allocation/ownership differs"
            );
        }
        if let Some(raw) = temporary {
            let p = record::word(raw, 16);
            if self.allocations.contains_key(&p) {
                ensure!(
                    owners.insert(p),
                    "temporary caption aliases an output owner"
                );
            }
        }
        ensure!(
            owners == self.allocations.keys().copied().collect(),
            "caption allocation has no retained owner"
        );
        Ok(())
    }
    pub(super) fn validate_record(
        &self,
        record: Option<&actions::Record>,
        constructed_only: bool,
    ) -> Result<()> {
        if let Some(raw) = record {
            let [begin, end, cap] = [16, 20, 24].map(|off| record::word(raw, off));
            let live = self.allocations.get(&begin).map(|b| b.len() as u32);
            let released = self.released.get(&begin).copied();
            ensure!(
                begin <= end && end < cap && live.or(released) == Some(cap - begin),
                "caption temporary allocation binding differs"
            );
            if let Some(bytes) = self.allocations.get(&begin) {
                ensure!(
                    bytes[(end - begin) as usize] == 0,
                    "caption terminator differs"
                );
            }
            if constructed_only {
                ensure!(
                    self.allocations.len() == 1
                        && live == Some(8)
                        && end == begin
                        && record::word(raw, 4) == 0
                        && record::word(raw, 12) == 0,
                    "record constructor temporary/storage binding differs"
                );
            }
        } else {
            ensure!(
                self.allocations.is_empty(),
                "caption allocations have no records"
            );
        }
        Ok(())
    }
    pub(super) fn validate(&self) -> Result<()> {
        let mut issued = BTreeMap::new();
        for (&p, b) in &self.allocations {
            ensure!(
                !b.is_empty() && b.len() <= 65536,
                "caption allocation extent differs"
            );
            issued.insert(p, b.len() as u32);
        }
        for (&p, &n) in &self.released {
            ensure!(
                issued.insert(p, n).is_none(),
                "released caption remains owned"
            );
        }
        let mut cursor = FIRST;
        for (p, n) in issued {
            ensure!(
                p == cursor && n > 0 && n <= 65536,
                "caption allocation history differs"
            );
            cursor = cursor
                .checked_add(n)
                .ok_or_else(|| anyhow::anyhow!("caption handle range overflow"))?;
        }
        ensure!(cursor == self.next, "caption allocation cursor differs");
        Ok(())
    }
    pub(super) fn assign(&mut self, header: &mut [u32; 3], bytes: &[u8]) -> Result<()> {
        record::assign_caption(header, record::Text::Bytes(bytes), self).map_err(anyhow::Error::msg)
    }
    pub(super) fn free(&mut self, p: u32, n: u32) -> Result<()> {
        record::Caption::free(self, p, n).map_err(anyhow::Error::msg)
    }
    fn locate(&self, p: u32, n: usize) -> Result<(u32, usize), String> {
        self.allocations
            .range(..=p)
            .next_back()
            .and_then(|(&start, b)| {
                let i = (p - start) as usize;
                (i <= b.len() && n <= b.len() - i).then_some((start, i))
            })
            .ok_or_else(|| "caption pointer outside owned storage".into())
    }
    pub(super) fn construct(&mut self) -> Result<actions::Record> {
        self.validate()?;
        // Undefined bytes stay scratch. The collection owner retains a separate
        // initialized-byte mask and must not compare these host values to native.
        let mut raw = [0u8; 28];
        record::construct(&mut raw, self).map_err(anyhow::Error::msg)?;
        self.validate()?;
        Ok(raw)
    }
}
impl record::Strings for Storage {
    fn allocate(&mut self, bytes: u32) -> Result<u32, String> {
        if bytes == 0 || bytes > 65536 {
            return Err("record constructor allocation size unsupported".into());
        }
        let next = self
            .next
            .checked_add(bytes)
            .ok_or("record caption handle range exhausted")?;
        let pointer = self.next;
        self.allocations.insert(pointer, vec![0; bytes as usize]);
        self.next = next;
        Ok(pointer)
    }
    fn write_nul(&mut self, pointer: u32) -> Result<(), String> {
        record::Caption::write(self, pointer, &[0])
    }
}
impl record::Caption for Storage {
    fn read(&self, p: u32, n: u32) -> Result<Vec<u8>, String> {
        let (start, i) = self.locate(p, n as usize)?;
        Ok(self.allocations[&start][i..i + n as usize].to_vec())
    }
    fn write(&mut self, p: u32, bytes: &[u8]) -> Result<(), String> {
        let (start, i) = self.locate(p, bytes.len())?;
        self.allocations.get_mut(&start).unwrap()[i..i + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
    fn free(&mut self, p: u32, n: u32) -> Result<(), String> {
        if self
            .allocations
            .get(&p)
            .is_none_or(|b| b.len() != n as usize)
        {
            return Err("caption free extent differs".into());
        }
        self.allocations.remove(&p);
        self.released.insert(p, n);
        Ok(())
    }
}
