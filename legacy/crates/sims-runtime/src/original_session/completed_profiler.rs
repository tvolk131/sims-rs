//! Source-bound first process-global profiler vector at fresh Goth B=89073.
//! Store this owner alongside the session; the application frame borrows it.
//! The full process import retains all four vectors and the enclosing counters;
//! renderer/callback execution and the enclosing Session still need their owners.
use super::completed_lot::Memory;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_vm::scheduler::clock::profiler::{Buffer, Record};

pub struct ProcessImported {
    pub profiler: sims_vm::scheduler::clock::profiler::tail::State,
    pub application_time: sims_vm::scheduler::clock::application::Fields,
    pub outer_counters: [u32; 2],
}

/// Recover all four logical vectors, their spare capacities, last timestamp,
/// frequency and enabled byte from the same independently retained B image.
pub fn import_process(source: &[u8]) -> Result<ProcessImported> {
    use sims_vm::scheduler::clock::{application, profiler::tail};
    ensure!(
        sims_fixtures::digest(source)
            == "243aea4cb12389b5be2800877b4e41c83cbbeab5ca50f7c49a060330527615ce",
        "full profiler source differs"
    );
    let v: Value = serde_json::from_slice(source)?;
    ensure!(
        v["schema"] == 1
            && v["tick"] == 89073
            && v["executes_target_code"] == false
            && v["native_parity_credit"] == 0,
        "profiler process source epoch differs"
    );
    let m = Memory::new(&v)?;
    let header = m.read(0x6fe8b8, 128)?;
    let mut vectors = Vec::new();
    let mut capacities = Vec::new();
    for i in 0..4 {
        let at = i * 12;
        let word = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
        let (begin, end, capacity) = (word(at), word(at + 4), word(at + 8));
        ensure!(
            begin <= end
                && end <= capacity
                && (end - begin) % 16 == 0
                && (capacity - begin) % 16 == 0
                && capacity - begin <= 16384,
            "profiler process vector extent differs"
        );
        let raw = if begin == end {
            Vec::new()
        } else {
            m.read(begin, (end - begin).try_into()?)?
        };
        vectors.push(
            raw.chunks_exact(16)
                .map(|b| {
                    std::array::from_fn(|j| {
                        u32::from_le_bytes(b[j * 4..j * 4 + 4].try_into().unwrap())
                    })
                })
                .collect::<Vec<[u32; 4]>>(),
        );
        capacities.push((capacity - begin) / 16);
    }
    let profiler = tail::State {
        arithmetic: tail::Arithmetic::CapturedQemu72Pc24Memory,
        events: Buffer::from_captured(
            vectors[0].iter().copied().map(Record::from_words).collect(),
            capacities[0],
        )
        .map_err(anyhow::Error::msg)?,
        pending: Buffer::from_captured(vectors[1].clone(), capacities[1])
            .map_err(anyhow::Error::msg)?,
        published: Buffer::from_captured(vectors[2].clone(), capacities[2])
            .map_err(anyhow::Error::msg)?,
        durations: Buffer::from_captured(
            vectors[3]
                .iter()
                .copied()
                .map(tail::Duration::from_words)
                .collect(),
            capacities[3],
        )
        .map_err(anyhow::Error::msg)?,
        last: Record::from_words(std::array::from_fn(|i| {
            u32::from_le_bytes(header[48 + i * 4..52 + i * 4].try_into().unwrap())
        })),
        frequency_bits: u64::from_le_bytes(header[64..72].try_into().unwrap()),
        enabled: header[72],
    };
    profiler.validate().map_err(anyhow::Error::msg)?;
    let time = m.read(0x6e54e0, 24)?;
    Ok(ProcessImported {
        profiler,
        application_time: application::Fields::from_words(std::array::from_fn(|i| {
            u32::from_le_bytes(time[i * 4..i * 4 + 4].try_into().unwrap())
        })),
        outer_counters: std::array::from_fn(|i| {
            u32::from_le_bytes(header[88 + i * 4..92 + i * 4].try_into().unwrap())
        }),
    })
}

pub fn import(source: &[u8]) -> Result<Buffer> {
    ensure!(
        sims_fixtures::digest(source)
            == "059a56d9711bae20bd992fbc5e0cc24fc0e7c8df76ee4f5d306f152527e2352a",
        "profiler frozen source differs"
    );
    let v: Value = serde_json::from_slice(source)?;
    ensure!(
        v["schema"] == 1
            && v["tick"] == 89073
            && v["executes_target_code"] == false
            && v["native_parity_credit"] == 0,
        "profiler source epoch differs"
    );
    let m = Memory::new(&v)?;
    for (address, count, digest) in [
        (
            0x5d532f,
            0x2e,
            "6c65e190fc2dd9c5cdc51fc49773a6dc3a5e460b3d16154b149a621e7c3c04a2",
        ),
        (
            0x4b3e5b,
            0x29,
            "897aa20ec0b38300ffd5c1b73be844a89cbcca38e4807375fbac46193f9f6334",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(address, count)?) == digest,
            "profiler helper source differs"
        );
    }
    let header = m.read(0x6fe8b8, 12)?;
    let word = |at| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    let (begin, end, capacity) = (word(0), word(4), word(8));
    ensure!(
        begin != 0
            && begin <= end
            && end <= capacity
            && (end - begin) % 16 == 0
            && (capacity - begin) % 16 == 0
            && capacity - begin <= 16384,
        "profiler vector extent differs"
    );
    let records = m
        .read(begin, (end - begin).try_into()?)?
        .chunks_exact(16)
        .map(|raw| {
            let words = std::array::from_fn(|i| {
                u32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap())
            });
            Record::from_words(words)
        })
        .collect::<Vec<_>>();
    let declared: Vec<[u32; 4]> = serde_json::from_value(v["record_words"].clone())
        .context("profiler record summary absent")?;
    ensure!(
        records
            .iter()
            .copied()
            .map(Record::words)
            .collect::<Vec<_>>()
            == declared
            && v["capacity"] == (capacity - begin) / 16,
        "profiler summary differs from actual bytes"
    );
    Buffer::from_captured(records, (capacity - begin) / 16).map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private frozen original profiler source"]
    fn frozen_vector_and_rejected_source_change() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let bytes = std::fs::read(root.join(
            "fixtures/private/goth-profiler-append-20261031-first/profiler-source-first.json",
        ))?;
        let buffer = import(&bytes)?;
        assert_eq!((buffer.records().len(), buffer.capacity()), (3, 32));
        let mut altered: Value = serde_json::from_slice(&bytes)?;
        altered["capacity"] = 64.into();
        assert!(import(&serde_json::to_vec(&altered)?).is_err());
        let process_source = std::fs::read(
            root.join("fixtures/private/goth-outer-frame-20261031-first/outer-source-third.json"),
        )?;
        let process = import_process(&process_source)?;
        assert_eq!(process.profiler.events, buffer);
        assert_eq!(process.outer_counters, [9516, 2379]);
        assert_eq!(process.profiler.durations.records().len(), 19);
        let saved = postcard::to_allocvec(&(
            process.profiler.clone(),
            process.application_time,
            process.outer_counters,
        ))?;
        let restored: (
            sims_vm::scheduler::clock::profiler::tail::State,
            sims_vm::scheduler::clock::application::Fields,
            [u32; 2],
        ) = postcard::from_bytes(&saved)?;
        assert_eq!(
            restored,
            (
                process.profiler,
                process.application_time,
                process.outer_counters
            )
        );
        let mut changed: Value = serde_json::from_slice(&process_source)?;
        changed["vectors"][3]["record_words"][0][2] = 0.into();
        assert!(import_process(&serde_json::to_vec(&changed)?).is_err());
        Ok(())
    }
}
