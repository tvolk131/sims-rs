//! Original459844 loaded gate and459120 autonomy-constants producer.
//!
//! The synchronous kernel owns no scheduler or random stream. Its World adapter
//! updates one explicit process owner; constructors retain the resumable Call.
//! FCNS decoding and resource publication are explicit source-bound host services;
//! lookup/default insertion and the twelve process stores execute here in order.
//! No startup preset or unavailable provider completion is installed.
use serde::{Deserialize, Serialize};

/// The producer instructions were checked in the unchanged playable image
/// a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321
/// and historical d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    CompleteCollection191,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub module: String,
    pub module_sha256: String,
    pub resource_id: u16,
    pub resource_sha256: String,
}
impl Binding {
    fn validate(&self) -> Result<(), String> {
        let hash = |s: &str| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit());
        if self.module.is_empty()
            || self.module.as_bytes().contains(&0)
            || self.resource_id != 2
            || !hash(&self.module_sha256)
            || !hash(&self.resource_sha256)
        {
            return Err("autonomy constants FCNS2 source identity is invalid".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Configuration {
    pub profile: Option<Profile>,
    pub source: Option<Binding>,
    /// Masked controls003f/007f/037f have source-bound supplied-input evidence.
    /// Unknown control is required lazily, not on the loaded-nonzero fastpath.
    pub fp_control: Option<u16>,
}

/// Exact raw process owners. None is unknown, including historical saves.
/// A known nonzero loaded byte never implies missing tuning words are known.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    pub loaded: Option<u8>,
    pub words: [Option<u32>; 12],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Field {
    FamilyThreshold,
    VisitorThreshold,
    LowAttenuation,
    ModerateAttenuation,
    HighAttenuation,
    VisitorLow,
    VisitorModerate,
    VisitorHigh,
    SittingThreshold,
    RandomCount,
    FriendshipThreshold,
    FunctionalDistance,
}
pub const ORDER: [Field; 12] = [
    Field::FamilyThreshold,
    Field::VisitorThreshold,
    Field::LowAttenuation,
    Field::ModerateAttenuation,
    Field::HighAttenuation,
    Field::VisitorLow,
    Field::VisitorModerate,
    Field::VisitorHigh,
    Field::SittingThreshold,
    Field::RandomCount,
    Field::FriendshipThreshold,
    Field::FunctionalDistance,
];
#[derive(Debug, Clone, Copy)]
struct Spec {
    address: u32,
    name: &'static [u8],
    default_bits: u32,
    integer: bool,
}
const SPECS: [Spec; 12] = [
    Spec {
        address: 0x6c4384,
        name: b"min autonomy score for family",
        default_bits: 0x3e4c_cccd,
        integer: false,
    },
    Spec {
        address: 0x6c4388,
        name: b"min autonomy score for visitors",
        default_bits: 0x3d4c_cccd,
        integer: false,
    },
    Spec {
        address: 0x6c4d80,
        name: b"low attenuation",
        default_bits: 0x3b03_126f,
        integer: false,
    },
    Spec {
        address: 0x6c4d84,
        name: b"moderate attenuation",
        default_bits: 0x3ca3_d70a,
        integer: false,
    },
    Spec {
        address: 0x6c4d88,
        name: b"high attenuation",
        default_bits: 0x3dcc_cccd,
        integer: false,
    },
    Spec {
        address: 0x6c4d8c,
        name: b"visitor low attenuation",
        default_bits: 0x3b03_126f,
        integer: false,
    },
    Spec {
        address: 0x6c4d90,
        name: b"visitor moderate attenuation",
        default_bits: 0x3ca3_d70a,
        integer: false,
    },
    Spec {
        address: 0x6c4d94,
        name: b"visitor high attenuation",
        default_bits: 0x3dcc_cccd,
        integer: false,
    },
    Spec {
        address: 0x6c438c,
        name: b"min autonomy score for sitting",
        default_bits: 0x3e4c_cccd,
        integer: false,
    },
    Spec {
        address: 0x6c4390,
        name: b"random selection count",
        default_bits: 0x4120_0000,
        integer: true,
    },
    Spec {
        address: 0x6c4398,
        name: b"friendship threshold",
        default_bits: 0x41c8_0000,
        integer: true,
    },
    Spec {
        address: 0x6c4394,
        name: b"functional distance attenuation",
        default_bits: 0x4040_0000,
        integer: false,
    },
];
impl Field {
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn address(self) -> u32 {
        SPECS[self.index()].address
    }
    pub fn name(self) -> &'static [u8] {
        SPECS[self.index()].name
    }
}

/// Semantic payload of the already decoded native ordered FCNS vector. Byte
/// strings exclude the terminator; the fixed native name/comment capacities
/// are64/256. No CP1252/Unicode rewriting or duplicate coalescing occurs here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub name: Vec<u8>,
    pub value_bits: u32,
    pub comment: Vec<u8>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub rows: Vec<Row>,
}
impl Table {
    pub fn validate(&self) -> Result<(), String> {
        for row in &self.rows {
            if row.name.len() >= 64
                || row.comment.len() >= 256
                || row.name.contains(&0)
                || row.comment.contains(&0)
            {
                return Err("autonomy constants native row string is invalid".into());
            }
        }
        Ok(())
    }
    ///4131f4 ->46f891 compares equal full lengths and folds ASCII A..Z only.
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.name.eq_ignore_ascii_case(name))
    }
    fn value_or_insert(&mut self, spec: Spec) -> u32 {
        if let Some(i) = self.find(spec.name) {
            return self.rows[i].value_bits;
        }
        //413271(create=1) ->4132a6 appends the default to the LOCAL table.
        //Only the later explicit Flush may mutate its owning resource state.
        self.rows.push(Row {
            name: spec.name.to_vec(),
            value_bits: spec.default_bits,
            comment: vec![],
        });
        spec.default_bits
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Load(Binding),
    ///The caller obtains the complete retained table through Call::table.
    ///Normal native flush return0/1 is ignored; no reply invents no success.
    Flush(Binding),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Loaded { binding: Binding, table: Table },
    Flushed { binding: Binding, return_value: i32 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Store {
    Word(Field, u32),
    Loaded(u8),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Service(Request),
    Store(Store),
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Word(u8),
    Flush,
    Loaded,
    Drop,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    configuration: Configuration,
    stage: Stage,
    pending: Option<Request>,
    table: Option<Table>,
}
impl Call {
    pub fn new(configuration: Configuration) -> Self {
        Self {
            configuration,
            stage: Stage::Entry,
            pending: None,
            table: None,
        }
    }
    pub(crate) fn belongs_to(&self, owner: &Owner) -> bool {
        self.configuration == owner.configuration
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn table(&self) -> Option<&Table> {
        self.table.as_ref()
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match (&self.pending, self.stage, &self.table) {
            (None, Stage::Entry | Stage::Complete, None) => true,
            (Some(Request::Load(binding)), Stage::Word(0), None) => {
                self.configuration.source.as_ref() == Some(binding)
            }
            (None, Stage::Word(i), Some(_)) => i < 12,
            (None, Stage::Flush | Stage::Loaded | Stage::Drop, Some(_)) => true,
            (Some(Request::Flush(binding)), Stage::Flush, Some(_)) => {
                self.configuration.source.as_ref() == Some(binding)
            }
            _ => false,
        };
        if !valid {
            return Err("autonomy constants continuation is inconsistent".into());
        }
        if self.table.is_some()
            && (self.configuration.profile.is_none() || self.configuration.source.is_none())
        {
            return Err("autonomy constants continuation lost its source".into());
        }
        if let Some(request) = &self.pending {
            match request {
                Request::Load(binding) | Request::Flush(binding) => binding.validate()?,
            }
        }
        if let Some(table) = &self.table {
            self.configuration.source.as_ref().unwrap().validate()?;
            table.validate()?;
        }
        Ok(())
    }
    fn control(&self) -> Result<(), String> {
        match self.configuration.fp_control {
            None => Err("autonomy constants x87 control is unknown".into()),
            Some(0x003f | 0x007f | 0x037f) => Ok(()),
            Some(_) => Err("autonomy constants x87 control is unverified".into()),
        }
    }
    fn convert(&self, bits: u32, integer: bool) -> Result<u32, String> {
        if integer {
            self.control()?;
            return Ok(crate::numeric::Wide::from_f32(bits).map_or(0, |n| n.ftol()) as u32);
        }
        let exponent = bits & 0x7f80_0000;
        let fraction = bits & 0x007f_ffff;
        if exponent == 0 && fraction != 0 || exponent == 0x7f80_0000 {
            self.control()?;
        }
        if exponent == 0x7f80_0000 && fraction != 0 && fraction & 0x0040_0000 == 0 {
            //The bounded Unicorn path preserves signaling bits/status0, which
            //does not establish original hardware FLD/FSTP quieting semantics.
            return Err("autonomy constants signaling-NaN float transfer is unverified".into());
        }
        Ok(bits)
    }
    /// One internal store or one explicit synchronous resource boundary.
    ///Store returns are budget cuts, not simulated VM/script yields.
    pub fn advance(&mut self, process: &mut Process) -> Result<Progress, String> {
        self.validate()?;
        if let Some(request) = &self.pending {
            return Ok(Progress::Service(request.clone()));
        }
        match self.stage {
            Stage::Entry => {
                let loaded = process
                    .loaded
                    .ok_or("autonomy constants loaded byte is unknown")?;
                if loaded != 0 {
                    self.stage = Stage::Complete;
                    return Ok(Progress::Complete);
                }
                self.configuration
                    .profile
                    .ok_or("autonomy constants code profile is unknown")?;
                let source = self
                    .configuration
                    .source
                    .as_ref()
                    .ok_or("autonomy constants FCNS2 source is unknown")?;
                source.validate()?;
                let request = Request::Load(source.clone());
                self.stage = Stage::Word(0);
                self.pending = Some(request.clone());
                Ok(Progress::Service(request))
            }
            Stage::Word(i) => {
                let spec = SPECS[i as usize];
                //Lookup/default insertion precedes conversion; any later host
                //unsupported conversion retains this already changed table.
                let bits = self.table.as_mut().unwrap().value_or_insert(spec);
                let value = self.convert(bits, spec.integer)?;
                process.words[i as usize] = Some(value);
                self.stage = if i == 11 {
                    Stage::Flush
                } else {
                    Stage::Word(i + 1)
                };
                Ok(Progress::Store(Store::Word(ORDER[i as usize], value)))
            }
            Stage::Flush => {
                let request = Request::Flush(self.configuration.source.as_ref().unwrap().clone());
                self.pending = Some(request.clone());
                Ok(Progress::Service(request))
            }
            Stage::Loaded => {
                process.loaded = Some(1);
                self.stage = Stage::Drop;
                Ok(Progress::Store(Store::Loaded(1)))
            }
            Stage::Drop => {
                //Native421146 frees the temporary table after the loaded store.
                //Rust owns its allocation; native allocation/unwind is not faked.
                self.table = None;
                self.stage = Stage::Complete;
                Ok(Progress::Complete)
            }
            Stage::Complete => Ok(Progress::Complete),
        }
    }
    /// Only an actual host completion supplies a reply. Host error/no reply
    ///keeps pending work and all preceding process stores exactly retained.
    pub fn reply(&mut self, reply: Reply) -> Result<(), String> {
        self.validate()?;
        match (&self.pending, reply) {
            (Some(Request::Load(expected)), Reply::Loaded { binding, table })
                if *expected == binding =>
            {
                binding.validate()?;
                table.validate()?;
                self.table = Some(table);
                self.pending = None;
            }
            (
                Some(Request::Flush(expected)),
                Reply::Flushed {
                    binding,
                    return_value: _,
                },
            ) if *expected == binding => {
                self.pending = None;
                self.stage = Stage::Loaded;
            }
            _ => return Err("autonomy constants resource reply differs".into()),
        }
        Ok(())
    }
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
mod tests;

/// Source-attributed observation of the actual process boundary. These hashes
/// identify retained evidence; effective words occur only in `Owner::process`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub capture: String,
    pub boundary: String,
    pub sample_line: u64,
    pub sample_sha256: String,
    pub pages_line: u64,
    pub pages_record_sha256: String,
    pub page_hashes: std::collections::BTreeMap<u32, String>,
}
impl Observation {
    pub fn validate(&self) -> Result<(), String> {
        let hash = |s: &str| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit());
        if self.capture.is_empty()
            || self.capture.contains('\0')
            || self.boundary.is_empty()
            || self.boundary.contains('\0')
            || self.sample_line == 0
            || self.pages_line <= self.sample_line
            || !hash(&self.sample_sha256)
            || !hash(&self.pages_record_sha256)
            || self.page_hashes.is_empty()
            || self.page_hashes.len() > 16
            || self
                .page_hashes
                .iter()
                .any(|(p, h)| p & 4095 != 0 || !hash(h))
        {
            return Err("invalid autonomy process observation provenance".into());
        }
        Ok(())
    }
}
/// Authoritative process tuning and immutable source/configuration provenance.
/// It lives above the replaceable neighborhood owner. No friend-threshold copy
/// is stored in person/family metadata or inferred from loaded/nonzero.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    /// Explicit stopped initial-process transcript, independent of old page observations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_evidence: Option<InputEvidence>,
    pub process: Process,
    pub configuration: Configuration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<Observation>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputEvidence {
    pub capture: String,
    pub capture_sha256: String,
    pub sample_line: u64,
    pub sample_sha256: String,
    pub entry: u32,
}
impl Owner {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(input) = &self.input_evidence {
            let hash = |s: &str| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit());
            if input.capture.is_empty()
                || input.capture.contains('\0')
                || input.sample_line == 0
                || input.entry != 0x459844
                || !hash(&input.capture_sha256)
                || !hash(&input.sample_sha256)
            {
                return Err("invalid autonomy initial-input provenance".into());
            }
        }
        if let Some(source) = &self.configuration.source {
            source.validate()?;
        }
        if let Some(observation) = &self.observation {
            observation.validate()?;
        }
        Ok(())
    }
    /// A constructor owns this child, while all calls mutate this same process.
    /// Do not demand provider/profile/control until the verified producer does.
    pub fn begin(&self) -> Call {
        Call::new(self.configuration.clone())
    }
    /// Pure lazy projection, independent of the producer's loaded byte. Native
    /// friendship reads DWORD6c4398 directly, even before any person is made.
    pub fn friendship_threshold(&self) -> Result<i32, String> {
        self.process.words[Field::FriendshipThreshold.index()]
            .map(|word| word as i32)
            .ok_or_else(|| "native friendship threshold unavailable".into())
    }
}
/// Advance only already verified producer work. The caller retains `Call`
/// when a real source-bound service is returned; no provider or Flush success
/// is fabricated here, and no scheduler, RNG or script yield is involved.
pub fn advance(world: &mut crate::World, call: &mut Call) -> Result<Progress, String> {
    let owner = world
        .autonomy_constants
        .as_mut()
        .ok_or("autonomy constants process owner is unknown")?;
    if !call.belongs_to(owner) {
        return Err("autonomy constants child configuration changed".into());
    }
    call.advance(&mut owner.process)
}
