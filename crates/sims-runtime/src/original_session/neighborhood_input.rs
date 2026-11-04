//! Explicit neighborhood input with separate observation/replacement provenance.
//! Unknown state stays unknown. Managed saves retain the frozen binding;
//! restoring it never reads original logs or rediscovers installed house slots.
use super::{House, Session};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub mod replacement;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status<'a> {
    Missing,
    ScopedCapture {
        house: &'a str,
        neighborhood: &'a str,
        lot: i16,
        count: usize,
    },
    Replacement(&'a replacement::Replacement),
    PreexistingVector {
        house: &'a str,
        neighborhood: &'a str,
        lot: i16,
        count: usize,
    },
}

const EXE: &str = "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58";
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_valid(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Identity {
    house: String,
    lot: i16,
    house_hash: String,
    neighborhood: String,
    neighborhood_hash: String,
}
impl Identity {
    fn from_house(h: &House) -> Result<Self> {
        let find = |source: &str| {
            h.sources
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(source))
                .map(|(_, v)| v.clone())
                .context("captured neighborhood source hash absent")
        };
        Ok(Self {
            house: h.source.clone(),
            lot: h.number,
            house_hash: find(&h.source)?,
            neighborhood: h.neighborhood_source.clone(),
            neighborhood_hash: find(&h.neighborhood_source)?,
        })
    }
    fn neighborhood_number(&self) -> Result<u16> {
        let path = self.neighborhood.to_ascii_lowercase();
        let dir = path
            .strip_suffix("/neighborhood.iff")
            .context("neighborhood capture namespace absent")?;
        ensure!(
            self.house
                .to_ascii_lowercase()
                .starts_with(&format!("{dir}/houses/"))
                && self.lot > 0,
            "captured house namespace differs"
        );
        let suffix = dir
            .strip_prefix("userdata")
            .context("unsupported captured neighborhood namespace")?;
        let n = if suffix.is_empty() {
            1
        } else {
            suffix.parse()?
        };
        ensure!(
            n > 0 && hash_valid(&self.house_hash) && hash_valid(&self.neighborhood_hash),
            "invalid captured neighborhood identity"
        );
        Ok(n)
    }
}
#[derive(Deserialize)]
struct Summary {
    neighborhood: u16,
    lot: i16,
    files: Vec<Log>,
}
#[derive(Deserialize)]
struct Log {
    sha256: String,
}
#[derive(Deserialize)]
struct Vector {
    header: [u32; 3],
    words: Vec<u32>,
}
impl Vector {
    fn decode(&self, stride: u32) -> Result<Vec<i32>> {
        let [begin, end, cap] = self.header;
        if self.header == [0; 3] {
            ensure!(self.words.is_empty(), "null neighborhood vector has values");
            return Ok(vec![]);
        }
        ensure!(
            begin >= 0x10000
                && begin % 4 == 0
                && end % 4 == 0
                && cap % 4 == 0
                && begin <= end
                && end <= cap,
            "invalid neighborhood vector header"
        );
        ensure!(
            (end - begin) % stride == 0
                && (cap - begin) % stride == 0
                && (cap - begin) / stride <= 4096
                && self.words.len() == ((end - begin) / 4) as usize,
            "neighborhood vector extent differs"
        );
        Ok(self.words.iter().map(|n| *n as i32).collect())
    }
}
#[derive(Deserialize)]
struct Sample {
    schema: u32,
    kind: String,
    owner: u32,
    house: u32,
    simulator: u32,
    neighborhood: u32,
    lot: i16,
    tick_before: u32,
    tick_after: u32,
    rng_before: u32,
    rng_after: u32,
    coordinates: Vector,
    nearby: Vector,
}
impl Sample {
    fn vectors(&self, house: i16) -> Result<(Vec<[i32; 2]>, Vec<i32>)> {
        ensure!(
            (3..=5).contains(&self.schema)
                && self.kind == "polling_not_atomic"
                && [self.owner, self.house, self.simulator, self.neighborhood]
                    .into_iter()
                    .all(|p| p >= 0x10000),
            "neighborhood observation owner/schema unavailable"
        );
        ensure!(
            self.lot == house
                && self.tick_before == self.tick_after
                && self.rng_before == self.rng_after,
            "neighborhood observation crossed context or execution"
        );
        let flat = self.coordinates.decode(8)?;
        let coordinates = flat
            .chunks_exact(2)
            .map(|p| [p[0], p[1]])
            .collect::<Vec<_>>();
        let order = self.nearby.decode(4)?;
        ensure!(
            sims_vm::nearby_houses::order(&coordinates, i32::from(house))
                .map_err(anyhow::Error::msg)?
                == order,
            "observed nearby vector differs from native load ordering"
        );
        Ok((coordinates, order))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Captured {
    schema: u32,
    identity: Identity,
    original_exe_sha256: String,
    summary_sha256: String,
    log_sha256: String,
    sample_index: usize,
    sample_sha256: String,
    sample: Vec<u8>,
}
impl Captured {
    ///The supplied summary scopes the log to a neighborhood and lot. Raw
    ///polling records contain lot IDs but do not identify the numbered folder.
    pub fn from_polling(
        house: &House,
        summary: &[u8],
        log: &[u8],
        sample_index: usize,
    ) -> Result<Self> {
        Self::from_identity(Identity::from_house(house)?, summary, log, sample_index)
    }
    fn from_identity(
        identity: Identity,
        summary: &[u8],
        log: &[u8],
        sample_index: usize,
    ) -> Result<Self> {
        ensure!(
            summary.len() <= 1024 * 1024 && log.len() <= 32 * 1024 * 1024,
            "neighborhood capture exceeds bound"
        );
        let scope: Summary = serde_json::from_slice(summary)?;
        let log_hash = sha(log);
        ensure!(
            scope.neighborhood == identity.neighborhood_number()?
                && scope.lot == identity.lot
                && scope.files.iter().any(|f| f.sha256 == log_hash),
            "observation summary does not bind this log/house"
        );
        let sample = log
            .split_inclusive(|b| *b == b'\n')
            .nth(sample_index)
            .context("neighborhood sample absent")?
            .to_vec();
        let result = Self {
            schema: 1,
            identity,
            original_exe_sha256: EXE.into(),
            summary_sha256: sha(summary),
            log_sha256: log_hash,
            sample_index,
            sample_sha256: sha(&sample),
            sample,
        };
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1
                && self.original_exe_sha256 == EXE
                && self.sample.len() <= 1024 * 1024
                && hash_valid(&self.summary_sha256)
                && hash_valid(&self.log_sha256)
                && sha(&self.sample) == self.sample_sha256,
            "captured neighborhood provenance differs"
        );
        self.identity.neighborhood_number()?;
        let record: Sample = serde_json::from_slice(&self.sample)?;
        record.vectors(self.identity.lot)?;
        Ok(())
    }
    pub fn nearby_houses(&self) -> Result<Vec<i32>> {
        self.validate()?;
        Ok(serde_json::from_slice::<Sample>(&self.sample)?
            .vectors(self.identity.lot)?
            .1)
    }
    pub fn validate_house(&self, house: &House) -> Result<()> {
        self.validate()?;
        ensure!(
            self.identity == Identity::from_house(house)?,
            "neighborhood observation belongs to another import"
        );
        Ok(())
    }
}

impl Session {
    ///Explicitly attach captured process input before running more scripts.
    ///No interpreter, callback, tick or RNG operation is performed here.
    pub fn bind_neighborhood_capture(&mut self, captured: Captured) -> Result<()> {
        captured.validate_house(&self.house)?;
        ensure!(
            self.host.presentation.neighborhood_input.is_none(),
            "neighborhood capture already bound"
        );
        ensure!(
            self.host.presentation.neighborhood_replacement.is_none(),
            "replacement neighborhood input already bound"
        );
        self.require_neighborhood_input_boundary()?;
        let order = captured.nearby_houses()?;
        let people = self
            .simulation
            .vm
            .world
            .person
            .as_mut()
            .context("neighborhood person registry absent")?;
        ensure!(
            people
                .nearby_houses
                .as_ref()
                .is_none_or(|old| *old == order),
            "captured input would replace a known nearby vector"
        );
        people.nearby_houses = Some(order);
        self.host.presentation.neighborhood_input = Some(captured);
        Ok(())
    }
    fn require_neighborhood_input_boundary(&self) -> Result<()> {
        ensure!(
            !self.player_input_pending()
                && !self.publication_pending()
                && !self.house_notification_pending()
                && !self.clock_pending()
                && !self.load_completion_pending()
                && !self.postload_pending()
                && !self.simulation.selection_work_pending()
                && self.simulation.vm.lot_rng_scope.is_none()
                && self.simulation.visit.is_none()
                && self.simulation.vm.scheduled_object_visit.is_none()
                && !self
                    .simulation
                    .vm
                    .threads
                    .values()
                    .any(|t| matches!(t.status, sims_vm::Status::Faulted(_))),
            "neighborhood input requires an unowned, nonfaulted boundary"
        );
        Ok(())
    }
    pub(super) fn validate_neighborhood_input(&self) -> Result<()> {
        ensure!(
            self.host.presentation.neighborhood_input.is_none()
                || self.host.presentation.neighborhood_replacement.is_none(),
            "captured and replacement neighborhood input conflict"
        );
        if let Some(replacement) = &self.host.presentation.neighborhood_replacement {
            replacement.validate_house(&self.house)?;
            ensure!(
                self.simulation
                    .vm
                    .world
                    .person
                    .as_ref()
                    .and_then(|people| people.nearby_houses.as_deref())
                    == Some(replacement.nearby_houses()),
                "retained replacement neighborhood input lost its live vector"
            );
        }
        if let Some(c) = &self.host.presentation.neighborhood_input {
            c.validate_house(&self.house)?;
            ensure!(
                self.simulation
                    .vm
                    .world
                    .person
                    .as_ref()
                    .and_then(|p| p.nearby_houses.as_ref())
                    == Some(&c.nearby_houses()?),
                "retained neighborhood input lost its live vector"
            );
        }
        Ok(())
    }
    pub fn neighborhood_input_status(&self) -> Result<Status<'_>> {
        self.validate_neighborhood_input()?;
        let count = self
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .and_then(|people| people.nearby_houses.as_ref())
            .map(Vec::len);
        Ok(if let Some(replacement) = &self.host.presentation.neighborhood_replacement {
            Status::Replacement(replacement)
        } else if self.host.presentation.neighborhood_input.is_some() {
            Status::ScopedCapture {
                house: &self.house.source,
                neighborhood: &self.house.neighborhood_source,
                lot: self.house.number,
                count: count.unwrap_or(0),
            }
        } else if let Some(count) = count {
            Status::PreexistingVector {
                house: &self.house.source,
                neighborhood: &self.house.neighborhood_source,
                lot: self.house.number,
                count,
            }
        } else {
            Status::Missing
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn identity() -> Identity {
        Identity {
            house: "UserData2/Houses/House09.iff".into(),
            lot: 9,
            house_hash: "a".repeat(64),
            neighborhood: "UserData2/Neighborhood.iff".into(),
            neighborhood_hash: "b".repeat(64),
        }
    }
    fn record() -> Value {
        json!({"schema":5,"kind":"polling_not_atomic","owner":65536,"house":69632,"simulator":73728,"neighborhood":77824,
            "lot":9,"tick_before":178127,"tick_after":178127,"rng_before":123,"rng_after":123,
            "coordinates":{"header":[0,0,0],"words":[]},"nearby":{"header":[0,0,0],"words":[]}})
    }
    fn input(r: &Value) -> (Vec<u8>, Vec<u8>) {
        let log = serde_json::to_vec(r).unwrap();
        let summary =
            serde_json::to_vec(&json!({"neighborhood":2,"lot":9,"files":[{"sha256":sha(&log)}]}))
                .unwrap();
        (summary, log)
    }
    fn captured(r: &Value) -> Result<Captured> {
        let (s, l) = input(r);
        Captured::from_identity(identity(), &s, &l, 0)
    }
    #[test]
    fn captured_empty_input_roundtrips_without_external_files_and_unknown_stays_absent() {
        assert!(super::super::View::default().neighborhood_input.is_none());
        let c = captured(&record()).unwrap();
        assert!(c.nearby_houses().unwrap().is_empty());
        let bytes = serde_json::to_vec(&c).unwrap();
        let restored: Captured = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(c, restored);
        assert!(restored.nearby_houses().unwrap().is_empty());
        let mut damaged = serde_json::to_value(c).unwrap();
        damaged["sample"][0] = json!(0);
        assert!(
            serde_json::from_value::<Captured>(damaged)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    #[test]
    fn captured_vectors_require_native_extents_and_order_including_signed_words() {
        let mut r = record();
        r["coordinates"] = json!({"header":[65536,65568,65568],"words":[0,0,10,10,11,10,100,100]});
        r["nearby"] = json!({"header":[69632,69644,69644],"words":[1,2,3]});
        assert_eq!(
            captured(&r).unwrap().nearby_houses().unwrap(),
            vec![1, 2, 3]
        );
        r["coordinates"]["words"][2] = json!(4294967295u32);
        assert_eq!(
            captured(&r).unwrap().nearby_houses().unwrap(),
            vec![1, 2, 3]
        );
        r["nearby"]["words"] = json!([3, 2, 1]);
        assert!(captured(&r).is_err());
        for v in [
            json!({"header":[0,0,0],"words":[1]}),
            json!({"header":[4,4,4],"words":[]}),
            json!({"header":[65536,65544,65544],"words":[1]}),
            json!({"header":[65536,65532,65544],"words":[]}),
        ] {
            r["coordinates"] = v;
            assert!(captured(&r).is_err());
        }
    }
    #[test]
    fn capture_scope_ownership_read_failures_and_execution_changes_are_rejected() {
        for (key, value) in [
            ("lot", json!(5)),
            ("tick_after", json!(178128)),
            ("rng_after", json!(124)),
            ("owner", json!(0)),
            ("neighborhood", json!(0)),
            ("schema", json!(2)),
            ("kind", json!("unreadable")),
            ("nearby", Value::Null),
        ] {
            let mut r = record();
            r[key] = value;
            assert!(captured(&r).is_err(), "accepted {key}");
        }
        let (s, l) = input(&record());
        let mut scope: Value = serde_json::from_slice(&s).unwrap();
        scope["neighborhood"] = json!(1);
        assert!(
            Captured::from_identity(identity(), &serde_json::to_vec(&scope).unwrap(), &l, 0)
                .is_err()
        );
        let mut other = l.clone();
        other.push(b' ');
        assert!(Captured::from_identity(identity(), &s, &other, 0).is_err());
        assert!(Captured::from_identity(identity(), &s, &l, 1).is_err());
    }
}
