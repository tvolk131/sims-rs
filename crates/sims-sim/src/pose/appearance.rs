//! Native appearance-record replacement (583541/583559/5835c9).
//! Asset lookup and binding allocation are host services. Failed lookup retains
//! the old binding; failed binding creation clears it. Successful replacement
//! creates the new binding before removing the old one.
use serde::{Deserialize, Serialize};

/// 595629 selects descriptor bindings before resolving their anchor bones or
/// invoking the mesh callback. The all-bits mask also includes zero-flag entries.
pub fn binding_selected(flags: u32, mask: u32) -> bool {
    mask == u32::MAX || flags & mask != 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub id: u64,
    pub owner: u64,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub binding: Option<Binding>,
    pub name: String,
    pub texture: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Input {
    /// 583541 passes pointers to the record's own strings. Re-read them after
    /// host calls; binding/lookup services can change the live record.
    Current,
    Replace {
        name: String,
        texture: String,
    },
}
impl Input {
    fn strings<'a>(&'a self, record: &'a Record) -> (&'a str, &'a str) {
        match self {
            Self::Current => (&record.name, &record.texture),
            Self::Replace { name, texture } => (name, texture),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Lookup {
        name: String,
    },
    Bind {
        asset: u64,
        texture: Option<String>,
    },
    Unbind {
        binding: Binding,
    },
    /// Original skeleton +98 write, after clearing the record's binding pointer.
    Dirty {
        owner: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Asset(Option<u64>),
    Binding(Option<Binding>),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Name,
    Bind,
    Clear,
    Dirty,
    Commit,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    input: Option<Input>,
    stage: Stage,
    pending: Option<Request>,
    asset: Option<u64>,
    replacement: Option<Binding>,
    dirty_owner: Option<u64>,
    result: bool,
}
impl Execution {
    pub fn bind(input: Input) -> Self {
        Self {
            input: Some(input),
            stage: Stage::Name,
            pending: None,
            asset: None,
            replacement: None,
            dirty_owner: None,
            result: false,
        }
    }
    pub fn clear() -> Self {
        Self {
            input: None,
            stage: Stage::Clear,
            pending: None,
            asset: None,
            replacement: None,
            dirty_owner: None,
            result: false,
        }
    }
    pub fn result(&self) -> Option<bool> {
        (self.stage == Stage::Complete && self.pending.is_none()).then_some(self.result)
    }
    pub fn next(&mut self, record: &mut Record) -> Option<Request> {
        loop {
            if let Some(request) = &self.pending {
                return Some(request.clone());
            }
            let request = match self.stage {
                Stage::Name => {
                    let (name, _) = self.input.as_ref().unwrap().strings(record);
                    if name.is_empty() {
                        self.stage = Stage::Complete;
                        continue;
                    }
                    Request::Lookup { name: name.into() }
                }
                Stage::Bind => {
                    let (_, texture) = self.input.as_ref().unwrap().strings(record);
                    Request::Bind {
                        asset: self.asset.unwrap(),
                        texture: (!texture.is_empty()).then(|| texture.into()),
                    }
                }
                Stage::Clear => {
                    if let Some(binding) = record.binding {
                        Request::Unbind { binding }
                    } else {
                        self.stage = Stage::Commit;
                        continue;
                    }
                }
                Stage::Dirty => Request::Dirty {
                    owner: self.dirty_owner.unwrap(),
                },
                Stage::Commit => {
                    if let Some(binding) = self.replacement {
                        let (name, texture) = self.input.as_ref().unwrap().strings(record);
                        let (name, texture) = (name.to_owned(), texture.to_owned());
                        record.name = name;
                        record.texture = texture;
                        record.binding = Some(binding);
                        self.result = true;
                    }
                    self.stage = Stage::Complete;
                    continue;
                }
                Stage::Complete => return None,
            };
            self.pending = Some(request.clone());
            return Some(request);
        }
    }
    pub fn acknowledge(&mut self, record: &mut Record, response: Response) -> Result<(), String> {
        match (self.pending.as_ref(), response) {
            (Some(Request::Lookup { .. }), Response::Asset(asset)) => {
                self.asset = asset;
                self.stage = if asset.is_some() {
                    Stage::Bind
                } else {
                    Stage::Complete
                };
            }
            (Some(Request::Bind { .. }), Response::Binding(binding)) => {
                self.replacement = binding;
                self.stage = Stage::Clear;
            }
            (Some(Request::Unbind { binding }), Response::Done) => {
                self.dirty_owner = Some(binding.owner);
                record.binding = None;
                self.stage = Stage::Dirty;
            }
            (Some(Request::Dirty { .. }), Response::Done) => self.stage = Stage::Commit,
            _ => return Err("native appearance response does not match pending service".into()),
        }
        self.pending = None;
        Ok(())
    }
}

/// Constructor-owned records, with stable identities in allocation order.
/// 5918ba visits the two body records, two hands, then head; accessories follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Records {
    pub entries: Vec<Record>,
    pub generic: usize,
    pub head: usize,
    pub hands: [usize; 2],
    pub body: [usize; 2],
    pub accessories: Vec<usize>,
}
impl Default for Records {
    fn default() -> Self {
        Self {
            entries: vec![Record::default(); 6],
            generic: 0,
            head: 1,
            hands: [2, 3],
            body: [4, 5],
            accessories: vec![],
        }
    }
}
impl Records {
    pub fn saved_order(&self) -> [usize; 5] {
        [
            self.body[0],
            self.body[1],
            self.hands[0],
            self.hands[1],
            self.head,
        ]
    }
    /// Stored descriptor view for asset loading. Does not run the original
    /// unbind/rebind services or claim a resumed animator.
    pub fn from_saved(saved: &sims_formats::saved_objects::Person) -> Self {
        let mut out = Self::default();
        for (id, pair) in out
            .saved_order()
            .into_iter()
            .zip(saved.appearance.chunks_exact(2))
        {
            out.entries[id].name = pair[0].clone();
            out.entries[id].texture = pair[1].clone();
        }
        for accessory in &saved.accessories {
            out.accessories.push(out.entries.len());
            out.entries.push(Record {
                binding: None,
                name: accessory.name.clone(),
                texture: accessory.binding.clone(),
            });
        }
        out
    }
}
