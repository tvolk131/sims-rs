//! Move-tool487191 deactivation and487105 reset. Native object/payload/history
//! services remain synchronous children; no callback is acknowledged here.
use super::Fields;
use serde::{Deserialize, Serialize};

// Fields in the constructor/cleanup subset that have not yet acquired useful
// domain names. Other tool storage is deliberately outside this projection.
const BYTES: [u16; 12] = [
    0x74, 0x79, 0x7a, 0x7b, 0x80, 0x81, 0x88, 0x89, 0x95, 0x96, 0x97, 0x98,
];
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub tool: Fields,
    pub clipboard: u32,       // +2c
    pub payload: Option<u32>, // +30, reference-counted command
    pub held: Option<u32>,    // +3c, stable native object identity
    pub hover: i16,           // +4c, script ID (not held identity)
    pub result: i32,          // +7c
    pub value90: u32,
    pub bytes: [u8; 12],
}
impl State {
    pub fn validate(&self) -> Result<(), String> {
        if self.payload == Some(0) || self.held == Some(0) || self.tool.definition == Some(0) {
            Err("move tool null identities must use None".into())
        } else {
            Ok(())
        }
    }
    pub fn constructor_fields(clipboard: u32) -> Self {
        let mut s = Self {
            tool: Fields::constructor_fields(),
            clipboard,
            payload: None,
            held: None,
            hover: 0,
            result: -1,
            value90: 0,
            bytes: [0; 12],
        };
        s.set_byte(0x97, 1);
        s
    }
    pub fn byte(&self, offset: u16) -> u8 {
        self.bytes[BYTES
            .iter()
            .position(|&v| v == offset)
            .expect("retained tool byte")]
    }
    pub fn set_byte(&mut self, offset: u16, value: u8) {
        self.bytes[BYTES
            .iter()
            .position(|&v| v == offset)
            .expect("retained tool byte")] = value;
    }
}
pub trait View {
    /// Object+1ac and object+f0, reread only at the original access points.
    fn disposable(&self, object: u32) -> Result<bool, String>;
    fn script_id(&self, object: u32) -> Result<i16, String>;
    /// ObjectUndoable+24 is its signed cost adjustment, consumed by48bdbd.
    fn clear_payload_cost(&mut self, payload: u32) -> Result<(), String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub registry: u32,      //49293c
    pub window: u32,        //Root+40, zero is the cursor's original early return
    pub key_function: u32,  // captured GetKeyState import
    pub control_state: u32, //App+b0 for4cf966
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    ///487d4e. The replacement frontend dismisses its placement hint when
    ///this returns; native tooltip-window/cache identities are not retained.
    ViewportLeave,
    Deactivate,
    Reset,
    Cursor,
    ClearCatalogSelection,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    StopPayload(u32),                         //virtual0c
    Reset,                                    //virtual18; concrete487105
    PayloadKind { payload: u32, value: u32 }, //48bcc2
    Append { clipboard: u32, payload: u32 },  //493259
    ObjectMode(u32),                          //virtual50(1)
    ReleasePayload(u32),                      //485a19, not an unconditional destructor
    DetachObject(u32),                        //42191c
    DeleteObject { registry: u32, id: i16 },  //44123f
    Hover,                                    //486d95(0,0)
    Cursor,                                   //486cee
    ClearCatalogSelection,                    //487df3
    ClearSelection(u32),                      //4cf966
    Undo(u32),                                //4932fa
    ClearUndo(u32),                           //493376, also used by4933dc
    ClearRedo(u32),                           //4933a9
    Key { function: u32, key: u32 },
    SetCursor { window: u32, value: u32 }, //4d2406
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Key(i16),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Discard,
    AfterReset,
    ClearRedo,
    Finish,
    Append,
    Release,
    ClearPayload,
    Detach,
    Delete,
    ClearHeld,
    Cursor,
    TailPayload,
    TailClear,
    Selection,
    SelectionClear,
    ControlKey,
    ChooseCursor,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    kind: Kind,
    stage: Stage,
    discard: bool,
    key_function: u32,
    control: i16,
    pending: Option<Request>,
}
impl Call {
    ///The native caller still owns+3c after44123f frees the cursor. Its next
    ///store clears that pointer; a saved host must retain this exact boundary.
    pub(super) fn awaiting_held_clear(&self) -> bool {
        self.kind == Kind::Reset && self.stage == Stage::ClearHeld
    }
    pub(super) fn awaiting_payload_clear(&self) -> bool {
        self.kind == Kind::Reset && matches!(self.stage, Stage::ClearPayload | Stage::TailClear)
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            stage: Stage::Entry,
            discard: false,
            key_function: 0,
            control: 0,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn acknowledge(&mut self, r: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(r) {
            return Err("move cleanup reply differs".into());
        }
        match (r, reply) {
            (Request::Key { key: 0x10, .. }, Reply::Key(_)) => (),
            (Request::Key { key: 0x11, .. }, Reply::Key(v)) => self.control = v,
            (Request::Key { .. }, _) | (_, Reply::Key(_)) => {
                return Err("move cleanup reply kind differs".into());
            }
            (_, Reply::Done) => (),
        }
        self.pending = None;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        use Request as R;
        use Stage as S;
        let stage = match self.kind {
            Kind::ViewportLeave => matches!(self.stage, S::Entry | S::Complete),
            Kind::Deactivate => matches!(
                self.stage,
                S::Entry | S::Discard | S::AfterReset | S::ClearRedo | S::Finish | S::Complete
            ),
            Kind::Reset => matches!(
                self.stage,
                S::Entry
                    | S::Append
                    | S::Release
                    | S::ClearPayload
                    | S::Detach
                    | S::Delete
                    | S::ClearHeld
                    | S::Cursor
                    | S::TailPayload
                    | S::TailClear
                    | S::Selection
                    | S::Complete
            ),
            Kind::Cursor => matches!(
                self.stage,
                S::Entry | S::ControlKey | S::ChooseCursor | S::Complete
            ),
            Kind::ClearCatalogSelection => matches!(
                self.stage,
                S::Entry | S::Selection | S::SelectionClear | S::Complete
            ),
        };
        let valid = match (self.stage, self.pending) {
            (S::Complete, Some(R::ObjectMode(_))) => self.kind == Kind::ViewportLeave,
            (_, None) => true,
            (S::Discard, Some(R::StopPayload(_)))
            | (S::AfterReset, Some(R::Reset))
            | (S::ClearRedo, Some(R::Undo(_)))
            | (S::Finish, Some(R::ClearRedo(_))) => self.kind == Kind::Deactivate,
            (S::Append, Some(R::PayloadKind { value: 3, .. }))
            | (S::Release, Some(R::Append { .. } | R::ObjectMode(_)))
            | (S::ClearPayload, Some(R::ReleasePayload(_)))
            | (S::Delete, Some(R::DetachObject(_)))
            | (S::ClearHeld, Some(R::DeleteObject { .. }))
            | (S::Cursor, Some(R::Hover))
            | (S::TailPayload, Some(R::Cursor))
            | (S::TailClear, Some(R::ReleasePayload(_)))
            | (S::Complete, Some(R::ClearCatalogSelection)) => self.kind == Kind::Reset,
            (S::SelectionClear, Some(R::ClearSelection(_))) => {
                self.kind == Kind::ClearCatalogSelection
            }
            (
                S::ControlKey,
                Some(R::Key {
                    key: 0x10,
                    function,
                }),
            )
            | (
                S::ChooseCursor,
                Some(R::Key {
                    key: 0x11,
                    function,
                }),
            ) => self.kind == Kind::Cursor && function == self.key_function,
            (S::Complete, Some(R::SetCursor { .. })) => self.kind == Kind::Cursor,
            _ => false,
        };
        if stage && valid {
            Ok(())
        } else {
            Err("move cleanup lost its child".into())
        }
    }
    pub fn advance(
        &mut self,
        s: &mut State,
        v: &mut impl View,
        i: Inputs,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        s.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        use Request as R;
        use Stage as S;
        let r = loop {
            match self.stage {
                S::Entry => match self.kind {
                    Kind::ViewportLeave => {
                        s.set_byte(0x97, 0);
                        self.stage = S::Complete;
                        if let Some(h) = s.held {
                            break R::ObjectMode(h);
                        }
                    }
                    Kind::Deactivate => {
                        self.stage = S::Discard;
                        if let Some(p) = s.payload {
                            break R::StopPayload(p);
                        }
                    }
                    Kind::Reset => {
                        self.stage = if s.held.is_some() {
                            S::Detach
                        } else {
                            S::ClearHeld
                        };
                        if let Some(o) = s.held {
                            if let Some(p) = s.payload {
                                if s.tool.flags[0] == 0 {
                                    self.stage = S::Append;
                                    break R::PayloadKind {
                                        payload: p,
                                        value: 3,
                                    };
                                }
                                self.stage = S::Release;
                                break R::ObjectMode(o);
                            }
                        }
                    }
                    Kind::ClearCatalogSelection => self.stage = S::Selection,
                    Kind::Cursor => {
                        if i.window == 0 {
                            self.stage = S::Complete;
                            continue;
                        }
                        self.key_function = i.key_function;
                        self.stage = S::ControlKey;
                        break R::Key {
                            function: self.key_function,
                            key: 0x10,
                        };
                    }
                },
                S::Discard => {
                    self.discard = s.tool.flags[0] == 0
                        && match s.held {
                            Some(o) => v.disposable(o)?,
                            None => false,
                        };
                    if self.discard {
                        if let Some(p) = s.payload {
                            v.clear_payload_cost(p)?;
                        }
                    }
                    self.stage = S::AfterReset;
                    break R::Reset;
                }
                S::AfterReset => {
                    self.stage = S::Finish;
                    if self.discard {
                        self.stage = S::ClearRedo;
                        break R::Undo(s.clipboard);
                    }
                }
                S::ClearRedo => {
                    self.stage = S::Finish;
                    break R::ClearRedo(s.clipboard);
                }
                S::Finish => {
                    s.tool.argument = -1;
                    s.result = -1;
                    s.tool.definition = None;
                    s.tool.mode = 1;
                    for o in [0x74, 0x7b, 0x81, 0x88, 0x89, 0x95, 0x96] {
                        s.set_byte(o, 0);
                    }
                    s.tool.flags = [0; 5];
                    s.value90 = 0;
                    s.set_byte(0x97, 1);
                    self.stage = S::Complete;
                }
                S::Append => {
                    self.stage = S::Release;
                    break R::Append {
                        clipboard: s.clipboard,
                        payload: s.payload.unwrap_or(0),
                    };
                }
                S::Release => {
                    self.stage = S::ClearPayload;
                    break R::ReleasePayload(s.payload.unwrap_or(0));
                }
                S::ClearPayload => {
                    s.payload = None;
                    self.stage = S::Detach;
                }
                S::Detach => {
                    // Only the initial held-object gate skips these calls. A child
                    // clearing+3c must not silently convert the mandatory call to a no-op.
                    self.stage = S::Delete;
                    break R::DetachObject(s.held.unwrap_or(0));
                }
                S::Delete => {
                    let id = v.script_id(s.held.ok_or("move reset held object vanished")?)?;
                    self.stage = S::ClearHeld;
                    break R::DeleteObject {
                        registry: i.registry,
                        id,
                    };
                }
                S::ClearHeld => {
                    s.held = None;
                    self.stage = S::Cursor;
                    break R::Hover;
                }
                S::Cursor => {
                    self.stage = S::TailPayload;
                    break R::Cursor;
                }
                S::TailPayload => {
                    self.stage = S::Selection;
                    if let Some(p) = s.payload {
                        self.stage = S::TailClear;
                        break R::ReleasePayload(p);
                    }
                }
                S::TailClear => {
                    s.payload = None;
                    self.stage = S::Selection;
                }
                S::Selection => {
                    if self.kind == Kind::Reset {
                        self.stage = S::Complete;
                        break R::ClearCatalogSelection;
                    }
                    self.stage = S::Complete;
                    if s.byte(0x98) != 0 {
                        self.stage = S::SelectionClear;
                        break R::ClearSelection(i.control_state);
                    }
                }
                S::SelectionClear => {
                    s.set_byte(0x98, 0);
                    self.stage = S::Complete;
                }
                S::ControlKey => {
                    self.stage = S::ChooseCursor;
                    break R::Key {
                        function: self.key_function,
                        key: 0x11,
                    };
                }
                S::ChooseCursor => {
                    let control = self.control < 0 && matches!(s.tool.mode, 3 | 4);
                    s.set_byte(0x79, u8::from(control));
                    let value = if s.tool.flags[2] != 0 && control {
                        6
                    } else if s.byte(0x80) != 0 {
                        12
                    } else if s.held.is_none() {
                        10
                    } else if s.byte(0x7a) != 0 {
                        35
                    } else if s.byte(0x7b) == 0 {
                        11
                    } else {
                        match s.result {
                            0 => 14,
                            2 => 16,
                            4 => 15,
                            6 => 13,
                            _ => 12,
                        }
                    };
                    self.stage = S::Complete;
                    break R::SetCursor {
                        window: i.window,
                        value,
                    };
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r);
        Ok(Some(r))
    }
}
