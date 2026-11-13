//! Original53a01f own-window update and4cfcdb family-count text child.
//!
//! This is not the53ab6d visibility publisher or the5dce19 graphics/window
//! caller. The supplied host owns application queries, executing427219 cache
//! access, CPState text, temporary storage and widgets. No render-time query or
//! implicit startup call is made here. Every synchronous child remains pending
//! until its matching typed reply is accepted; rejected services retain all
//! earlier host effects and do not invent a successful return.
use serde::{Deserialize, Serialize};

/// Actual ViewControl+d4/+1d0, read afresh at their individual native reads.
/// None at advance is an unknown/unbound owner; a known raw null field stays0
/// until its child reports the unsupported dereference at the original cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub owner: u32,
    pub state: Option<u32>,
    pub label: Option<u32>,
}

/// Exact owned-buffer binding read from CPState+238 after Store returns. The
/// buffer identity is retained through suspension; a later replacement of the
/// CPState's field must not redirect the already returned CString reference.
/// Providers retain the referenced buffer/lifetime and bytes independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountText {
    pub state: u32,
    pub buffer: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumericRequest {
    LotQuery,                             //40745f, live process lot owner
    ApplicationFamily,                    // live global->application->+10, including known null
    SelectedPerson,                       //4e986d; only rawffffffff is the early return
    Person { index: u32 },                //4e9608, original caller removes its argument
    NeighborWord { person: u32 },         // actual person+606
    Family { neighbor: i16 },             //425d69, live neighborhood owner
    Friends { family: u32 },              //410260->427219, EXECUTION, never immutable inspection
    Format { value: i32 },                //%d, signed full width
    Store { state: u32, bytes: Vec<u8> }, //46f7e5, CPState+234
    CString { state: u32 },               //4cfd9a, fresh pointer/buffer binding after Store
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumericReply {
    Boolean(bool),
    Number(u32),
    Owner(Option<u32>),
    Word(u16),
    Count(i32),
    Bytes(Vec<u8>),
    Text(CountText),
    Done,
}
impl NumericRequest {
    fn accepts(&self, reply: &NumericReply) -> bool {
        use NumericReply as P;
        match (self, reply) {
            (Self::LotQuery, P::Boolean(_))
            | (Self::SelectedPerson, P::Number(_))
            | (Self::NeighborWord { .. }, P::Word(_))
            | (Self::Friends { .. }, P::Count(_))
            | (Self::Format { .. }, P::Bytes(_))
            | (Self::Store { .. }, P::Done) => true,
            (Self::CString { state }, P::Text(text)) => text.state == *state,
            (Self::ApplicationFamily | Self::Person { .. } | Self::Family { .. }, P::Owner(id)) => {
                id.is_none_or(|p| p != 0)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowRequest {
    /// Internal synchronous4cfcdb child, consumed by Call rather than frontend.
    Count {
        state: u32,
    },
    Temporary {
        source: CountText,
    }, //5d4ab1, snapshot CString bytes here
    Equal {
        left: Vec<u8>,
        widget: u32,
    }, //5b8897 vs widget+4c
    SetText {
        widget: u32,
        bytes: Vec<u8>,
    }, // virtual+9c
    Destroy {
        temporary: u8,
    }, //5b8873, second then first
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowReply {
    Text(CountText),
    Bytes(Vec<u8>),
    Equal(bool),
    Done,
}
impl WindowRequest {
    fn accepts(&self, reply: &WindowReply) -> bool {
        matches!((self, reply),
            (Self::Count { state }, WindowReply::Text(actual)) if *state == actual.state)
            || matches!(
                (self, reply),
                (Self::Temporary { .. }, WindowReply::Bytes(_))
                    | (Self::Equal { .. }, WindowReply::Equal(_))
                    | (
                        Self::SetText { .. } | Self::Destroy { .. },
                        WindowReply::Done
                    )
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Numeric(NumericRequest),
    Window(WindowRequest),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Numeric(NumericReply),
    Window(WindowReply),
}
/// Research/host failures are typed boundaries, not original error codes or
/// successful empty/default values. A caller may retry the same pending child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Failure {
    MissingInput(String),
    Unimplemented(String),
    NativeFault(String),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Error {
    InvalidContinuation,
    FieldsUnavailable { owner: u32 },
    FieldOwnerMismatch { expected: u32, actual: u32 },
    FieldUnavailable { owner: u32, offset: u16 },
    ReplyMismatch,
    Unsupported { request: Request, failure: Failure },
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidContinuation => {
                f.write_str("family label lost its original child continuation")
            }
            Self::FieldsUnavailable { owner } => {
                write!(f, "family label ViewControl fields unavailable: {owner:#x}")
            }
            Self::FieldOwnerMismatch { expected, actual } => write!(
                f,
                "family label ViewControl owner differs: expected{expected:#x},actual{actual:#x}"
            ),
            Self::FieldUnavailable { owner, offset } => write!(
                f,
                "family label ViewControl field unavailable: {owner:#x}+{offset:#x}"
            ),
            Self::ReplyMismatch => f.write_str("family label child reply differs"),
            Self::Unsupported { request, failure } => write!(
                f,
                "family label child remains unsupported: {request:?}: {failure:?}"
            ),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum NumericStage {
    Entry,
    Lot,
    Application,
    Selected,
    Person,
    Neighbor,
    Family,
    Friends,
    Format,
    Store,
    CString,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Numeric {
    state: u32,
    stage: NumericStage,
    pending: Option<NumericRequest>,
    reply: Option<NumericReply>,
    result: Option<CountText>,
}
impl Numeric {
    pub fn new(state: u32) -> Self {
        Self {
            state,
            stage: NumericStage::Entry,
            pending: None,
            reply: None,
            result: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == NumericStage::Complete && self.pending.is_none()
    }
    pub fn result(&self) -> Option<CountText> {
        self.complete().then_some(self.result).flatten()
    }
    pub fn state(&self) -> u32 {
        self.state
    }
    pub fn request(&self) -> Option<&NumericRequest> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), Error> {
        use NumericStage as S;
        let expected = match self.stage {
            S::Entry | S::Complete => None,
            S::Lot => Some(matches!(self.pending, Some(NumericRequest::LotQuery))),
            S::Application => Some(matches!(
                self.pending,
                Some(NumericRequest::ApplicationFamily)
            )),
            S::Selected => Some(matches!(self.pending, Some(NumericRequest::SelectedPerson))),
            S::Person => Some(matches!(self.pending, Some(NumericRequest::Person { .. }))),
            S::Neighbor => Some(matches!(
                self.pending,
                Some(NumericRequest::NeighborWord { .. })
            )),
            S::Family => Some(matches!(self.pending, Some(NumericRequest::Family { .. }))),
            S::Friends => Some(matches!(self.pending, Some(NumericRequest::Friends { .. }))),
            S::Format => Some(matches!(self.pending, Some(NumericRequest::Format { .. }))),
            S::Store => Some(
                matches!(&self.pending, Some(NumericRequest::Store { state, .. }) if *state == self.state),
            ),
            S::CString => Some(
                matches!(self.pending, Some(NumericRequest::CString { state }) if state == self.state),
            ),
        };
        let valid =
            match expected {
                None => self.pending.is_none() && self.reply.is_none(),
                Some(pending_matches) => (pending_matches && self.reply.is_none())
                    || (self.pending.is_none()
                        && self.reply.as_ref().is_some_and(|r| match self.stage {
                            S::Lot => matches!(r, NumericReply::Boolean(_)),
                            S::Application | S::Person | S::Family => {
                                matches!(r, NumericReply::Owner(p) if p.is_none_or(|n| n != 0))
                            }
                            S::Selected => matches!(r, NumericReply::Number(_)),
                            S::Neighbor => matches!(r, NumericReply::Word(_)),
                            S::Friends => matches!(r, NumericReply::Count(_)),
                            S::Format => matches!(r, NumericReply::Bytes(_)),
                            S::Store => matches!(r, NumericReply::Done),
                            S::CString => {
                                matches!(r, NumericReply::Text(text) if text.state == self.state)
                            }
                            _ => false,
                        })),
            };
        let valid = valid
            && self.result.is_some() == self.complete()
            && self.result.is_none_or(|text| text.state == self.state);
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidContinuation)
        }
    }
    pub fn acknowledge(
        &mut self,
        request: &NumericRequest,
        reply: NumericReply,
    ) -> Result<(), Error> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) || !request.accepts(&reply) {
            return Err(Error::ReplyMismatch);
        }
        self.pending = None;
        self.reply = Some(reply);
        Ok(())
    }
    pub fn advance(&mut self) -> Result<Option<NumericRequest>, Error> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        use NumericReply as P;
        use NumericRequest as R;
        use NumericStage as S;
        let (next, request) = match self.stage {
            S::Entry => (S::Lot, R::LotQuery),
            S::Lot => match self.reply.take().unwrap() {
                P::Boolean(true) => (S::Application, R::ApplicationFamily),
                P::Boolean(false) => (S::Selected, R::SelectedPerson),
                _ => unreachable!(),
            },
            S::Application | S::Family => match self.reply.take().unwrap() {
                P::Owner(Some(family)) => (S::Friends, R::Friends { family }),
                P::Owner(None) => (S::Format, R::Format { value: 0 }),
                _ => unreachable!(),
            },
            S::Selected => match self.reply.take().unwrap() {
                P::Number(u32::MAX) => (S::Format, R::Format { value: 0 }),
                P::Number(index) => (S::Person, R::Person { index }),
                _ => unreachable!(),
            },
            S::Person => match self.reply.take().unwrap() {
                P::Owner(Some(person)) => (S::Neighbor, R::NeighborWord { person }),
                P::Owner(None) => (S::Format, R::Format { value: 0 }),
                _ => unreachable!(),
            },
            S::Neighbor => match self.reply.take().unwrap() {
                P::Word(word) => (
                    S::Family,
                    R::Family {
                        neighbor: word as i16,
                    },
                ),
                _ => unreachable!(),
            },
            S::Friends => match self.reply.take().unwrap() {
                P::Count(value) => (S::Format, R::Format { value }),
                _ => unreachable!(),
            },
            S::Format => match self.reply.take().unwrap() {
                P::Bytes(bytes) => (
                    S::Store,
                    R::Store {
                        state: self.state,
                        bytes,
                    },
                ),
                _ => unreachable!(),
            },
            S::Store => {
                self.reply = None;
                (S::CString, R::CString { state: self.state })
            }
            S::CString => {
                let P::Text(text) = self.reply.take().unwrap() else {
                    unreachable!()
                };
                self.result = Some(text);
                self.reply = None;
                self.stage = S::Complete;
                return Ok(None);
            }
            S::Complete => return Ok(None),
        };
        self.stage = next;
        self.pending = Some(request.clone());
        Ok(Some(request))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    FirstCount,
    FirstTemporary,
    Compared,
    SecondCount,
    SecondTemporary,
    Set,
    DestroySecond,
    DestroyFirst,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    owner: u32,
    raw_argument: u32,
    stage: Stage,
    first: Option<Vec<u8>>,
    pending: Option<WindowRequest>,
    reply: Option<WindowReply>,
}
impl Update {
    pub fn new(owner: u32, raw_argument: u32) -> Self {
        Self {
            owner,
            raw_argument,
            stage: Stage::Entry,
            first: None,
            pending: None,
            reply: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn result(&self) -> Option<u8> {
        self.complete().then_some(0)
    }
    pub fn request(&self) -> Option<&WindowRequest> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), Error> {
        use Stage as S;
        use WindowReply as P;
        use WindowRequest as R;
        let pending_kind = match (&self.pending, self.stage) {
            (Some(R::Count { .. }), S::FirstCount | S::SecondCount)
            | (Some(R::Temporary { .. }), S::FirstTemporary | S::SecondTemporary)
            | (Some(R::SetText { .. }), S::Set) => true,
            (Some(R::Equal { left, .. }), S::Compared) => self.first.as_ref() == Some(left),
            (Some(R::Destroy { temporary: 2 }), S::DestroySecond)
            | (Some(R::Destroy { temporary: 1 }), S::DestroyFirst) => true,
            _ => false,
        };
        let reply_kind = match (&self.reply, self.stage) {
            (Some(P::Text(_)), S::FirstCount | S::SecondCount)
            | (Some(P::Bytes(_)), S::FirstTemporary | S::SecondTemporary)
            | (Some(P::Equal(_)), S::Compared)
            | (Some(P::Done), S::Set | S::DestroySecond | S::DestroyFirst) => true,
            _ => false,
        };
        let fresh = matches!(self.stage, S::Entry | S::Complete)
            && self.pending.is_none()
            && self.reply.is_none();
        let has_first = matches!(
            self.stage,
            S::Compared
                | S::SecondCount
                | S::SecondTemporary
                | S::Set
                | S::DestroySecond
                | S::DestroyFirst
        );
        let valid = (fresh
            || (pending_kind && self.reply.is_none())
            || (self.pending.is_none() && reply_kind))
            && self.first.is_some() == has_first
            && (self.raw_argument as u8 != 0 || matches!(self.stage, S::Entry | S::Complete));
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidContinuation)
        }
    }
    pub fn acknowledge(
        &mut self,
        request: &WindowRequest,
        reply: WindowReply,
    ) -> Result<(), Error> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) || !request.accepts(&reply) {
            return Err(Error::ReplyMismatch);
        }
        self.pending = None;
        self.reply = Some(reply);
        Ok(())
    }
    pub fn advance(&mut self, fields: Option<&Fields>) -> Result<Option<WindowRequest>, Error> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        use Stage as S;
        use WindowReply as P;
        use WindowRequest as R;
        let fields = || {
            let fields = fields.ok_or(Error::FieldsUnavailable { owner: self.owner })?;
            if fields.owner != self.owner {
                return Err(Error::FieldOwnerMismatch {
                    expected: self.owner,
                    actual: fields.owner,
                });
            }
            Ok(fields)
        };
        // Do not consume a reply before a fallible fresh-field read. An
        // unavailable owner must preserve the exact resumable continuation.
        let state = || -> Result<WindowRequest, Error> {
            Ok(R::Count {
                state: fields()?.state.ok_or(Error::FieldUnavailable {
                    owner: self.owner,
                    offset: 0xd4,
                })?,
            })
        };
        let (next, request) = match self.stage {
            S::Entry if self.raw_argument as u8 == 0 => {
                self.stage = S::Complete;
                return Ok(None);
            }
            S::Entry => (S::FirstCount, state()?),
            S::FirstCount | S::SecondCount => {
                let P::Text(source) = self.reply.as_ref().unwrap() else {
                    unreachable!()
                };
                (
                    if self.stage == S::FirstCount {
                        S::FirstTemporary
                    } else {
                        S::SecondTemporary
                    },
                    R::Temporary { source: *source },
                )
            }
            S::FirstTemporary => {
                let label = fields()?.label.ok_or(Error::FieldUnavailable {
                    owner: self.owner,
                    offset: 0x1d0,
                })?;
                let P::Bytes(bytes) = self.reply.as_ref().unwrap() else {
                    unreachable!()
                };
                self.first = Some(bytes.clone());
                (
                    S::Compared,
                    R::Equal {
                        left: bytes.clone(),
                        widget: label,
                    },
                )
            }
            S::Compared => match self.reply.as_ref().unwrap() {
                P::Equal(true) => (S::DestroyFirst, R::Destroy { temporary: 1 }),
                P::Equal(false) => (S::SecondCount, state()?),
                _ => unreachable!(),
            },
            S::SecondTemporary => {
                let label = fields()?.label.ok_or(Error::FieldUnavailable {
                    owner: self.owner,
                    offset: 0x1d0,
                })?;
                let P::Bytes(bytes) = self.reply.as_ref().unwrap() else {
                    unreachable!()
                };
                (
                    S::Set,
                    R::SetText {
                        widget: label,
                        bytes: bytes.clone(),
                    },
                )
            }
            S::Set => (S::DestroySecond, R::Destroy { temporary: 2 }),
            S::DestroySecond => (S::DestroyFirst, R::Destroy { temporary: 1 }),
            S::DestroyFirst => {
                self.first = None;
                self.reply = None;
                self.stage = S::Complete;
                return Ok(None);
            }
            S::Complete => return Ok(None),
        };
        self.reply = None;
        self.stage = next;
        self.pending = Some(request.clone());
        Ok(Some(request))
    }
}

/// Owns the synchronous numeric child through every suspension. This combines
/// the two proven routines only, never a speculative graphics pump invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    update: Update,
    numeric: Option<Numeric>,
}
impl Call {
    /// Observational identity for an enclosing retained caller.
    pub fn owner(&self) -> u32 {
        self.update.owner
    }
    pub fn raw_argument(&self) -> u32 {
        self.update.raw_argument
    }
    pub fn new(owner: u32, raw_argument: u32) -> Self {
        Self {
            update: Update::new(owner, raw_argument),
            numeric: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.update.complete() && self.numeric.is_none()
    }
    pub fn result(&self) -> Option<u8> {
        self.complete().then_some(0)
    }
    pub fn numeric(&self) -> Option<&Numeric> {
        self.numeric.as_ref()
    }
    pub fn validate(&self) -> Result<(), Error> {
        self.update.validate()?;
        if let Some(child) = &self.numeric {
            child.validate()?;
            if self.update.request() != Some(&WindowRequest::Count { state: child.state }) {
                return Err(Error::InvalidContinuation);
            }
        }
        Ok(())
    }
    pub fn request(&self) -> Option<Request> {
        if let Some(child) = &self.numeric {
            child.request().cloned().map(Request::Numeric)
        } else {
            self.update.request().cloned().map(Request::Window)
        }
    }
    pub fn reject(&self, request: &Request, failure: Failure) -> Result<Error, Error> {
        self.validate()?;
        if self.request().as_ref() != Some(request) {
            return Err(Error::ReplyMismatch);
        }
        Ok(Error::Unsupported {
            request: request.clone(),
            failure,
        })
    }
    pub fn acknowledge(&mut self, request: &Request, reply: Reply) -> Result<(), Error> {
        self.validate()?;
        match (request, reply) {
            (Request::Numeric(r), Reply::Numeric(p)) => self
                .numeric
                .as_mut()
                .ok_or(Error::ReplyMismatch)?
                .acknowledge(r, p),
            (Request::Window(r), Reply::Window(p)) if self.numeric.is_none() => {
                self.update.acknowledge(r, p)
            }
            _ => Err(Error::ReplyMismatch),
        }
    }
    pub fn advance(&mut self, fields: Option<&Fields>) -> Result<Option<Request>, Error> {
        self.validate()?;
        loop {
            if let Some(child) = &mut self.numeric {
                if let Some(request) = child.advance()? {
                    return Ok(Some(Request::Numeric(request)));
                }
                let source = child.result().ok_or(Error::InvalidContinuation)?;
                self.update.acknowledge(
                    &WindowRequest::Count {
                        state: source.state,
                    },
                    WindowReply::Text(source),
                )?;
                self.numeric = None;
            }
            match self.update.advance(fields)? {
                Some(WindowRequest::Count { state }) => self.numeric = Some(Numeric::new(state)),
                Some(request) => return Ok(Some(Request::Window(request))),
                None => return Ok(None),
            }
        }
    }
}
