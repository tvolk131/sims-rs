//! Modern 5918ba appearance read prefix, after decoded input is available and
//! before animation descriptors/load-tail callbacks. Existing initialized
//! records are cleared in native order; this does not load saved person words.
use super::{
    appearance::{Record, Records},
    appearance_reload::{Owner, Reload, Request as Lifecycle},
    appearance_selection::Descriptor,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Lifecycle(Lifecycle),
    BindRecord(usize),
    TextureNotification,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Clear,
    Destroy,
    Fixed(usize),
    Notice,
    Accessories(usize),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Read {
    fixed: [Descriptor; 5],
    accessories: Vec<Descriptor>,
    skip_bind: bool,
    old_head: Descriptor,
    lifecycle: Reload,
    stage: Stage,
    pending: Option<Request>,
}
impl Read {
    /// Save version >=51 read-side appearance layout. `skip_bind` is the
    /// original caller flag: strings still load and head notifications still
    /// occur, but record binding is skipped. Write mode is a separate codec.
    pub fn new(
        records: &Records,
        fixed: [Descriptor; 5],
        accessories: Vec<Descriptor>,
        skip_bind: bool,
    ) -> Result<Self, String> {
        let head = records
            .entries
            .get(records.head)
            .ok_or("saved appearance head record absent")?;
        if records
            .saved_order()
            .iter()
            .chain(records.accessories.iter())
            .any(|&id| id >= records.entries.len())
        {
            return Err("saved appearance record identity absent".into());
        }
        Ok(Self {
            fixed,
            accessories,
            skip_bind,
            old_head: Descriptor {
                name: head.name.clone(),
                texture: head.texture.clone(),
            },
            lifecycle: Reload::clear(),
            stage: Stage::Clear,
            pending: None,
        })
    }
    pub fn from_saved(
        records: &Records,
        saved: &sims_formats::saved_objects::Person,
        skip_bind: bool,
    ) -> Result<Self, String> {
        if saved.appearance.len() != 10 {
            return Err("saved appearance fixed record count unsupported".into());
        }
        Self::new(
            records,
            std::array::from_fn(|i| Descriptor {
                name: saved.appearance[2 * i].clone(),
                texture: saved.appearance[2 * i + 1].clone(),
            }),
            saved
                .accessories
                .iter()
                .map(|a| Descriptor {
                    name: a.name.clone(),
                    texture: a.binding.clone(),
                })
                .collect(),
            skip_bind,
        )
    }
    pub fn is_complete(&self) -> bool {
        self.stage == Stage::Done && self.pending.is_none()
    }
    pub fn next(
        &mut self,
        owner: &mut Owner,
        records: &mut Records,
    ) -> Result<Option<Request>, String> {
        if let Some(request) = &self.pending {
            return Ok(Some(request.clone()));
        }
        loop {
            let request = match self.stage {
                Stage::Clear | Stage::Destroy => {
                    // These lifecycle modes do not access person words. The
                    // enclosing reader has not deserialized those words yet.
                    if let Some(request) =
                        self.lifecycle
                            .next(owner, records, &mut Default::default())?
                    {
                        Request::Lifecycle(request)
                    } else {
                        if self.stage == Stage::Clear {
                            owner.selection.dirty = true;
                            self.lifecycle = Reload::destroy_accessories();
                            self.stage = Stage::Destroy;
                        } else {
                            self.stage = Stage::Fixed(0);
                        }
                        continue;
                    }
                }
                Stage::Fixed(i) => {
                    if i == self.fixed.len() {
                        self.stage = Stage::Notice;
                        continue;
                    }
                    let id = records.saved_order()[i];
                    records.entries[id].name = self.fixed[i].name.clone();
                    records.entries[id].texture = self.fixed[i].texture.clone();
                    self.stage = Stage::Fixed(i + 1);
                    if self.skip_bind {
                        continue;
                    }
                    Request::BindRecord(id)
                }
                Stage::Notice => {
                    let head = &records.entries[records.head];
                    self.stage = Stage::Accessories(0);
                    if head.name == self.old_head.name && head.texture == self.old_head.texture {
                        continue;
                    }
                    Request::TextureNotification
                }
                Stage::Accessories(i) => {
                    let Some(descriptor) = self.accessories.get(i) else {
                        self.stage = Stage::Done;
                        continue;
                    };
                    let id = records.entries.len();
                    records.entries.push(Record {
                        binding: None,
                        name: descriptor.name.clone(),
                        texture: descriptor.texture.clone(),
                    });
                    records.accessories.push(id);
                    self.stage = Stage::Accessories(i + 1);
                    if self.skip_bind {
                        continue;
                    }
                    Request::BindRecord(id)
                }
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request.clone());
            return Ok(Some(request));
        }
    }
    pub fn acknowledge(&mut self, owner: &mut Owner, records: &mut Records) -> Result<(), String> {
        match self
            .pending
            .as_ref()
            .ok_or("saved appearance service not pending")?
        {
            Request::Lifecycle(_) => self.lifecycle.acknowledge(owner, records)?,
            Request::BindRecord(_) | Request::TextureNotification => {}
        }
        self.pending = None;
        Ok(())
    }
}
