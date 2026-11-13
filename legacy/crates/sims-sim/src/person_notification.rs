//! Notification dispatch from original 0x4a69aa and animation labels (0x4a698d).
//!
//! This is the child controller, not a presenter. Currency formatting must return
//! the actual locale bytes; sound and display must be delivered by their owners before
//! acknowledgment. The parent person Update may clear its field only after this
//! controller completes. Display cache/timing is a separate child service;
//! acknowledging it here does not certify visible notification delivery.
use serde::{Deserialize, Serialize};

pub mod currency;
pub mod display;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    NoOwner,
    NoPresenter,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Input {
    Person(i16),
    AnimationName(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    Default,
    /// Original 0x4d3516: yellow (255, 203, 23) background, black text.
    Yellow,
    /// Original 0x4d356a: magenta (255, 0, 255) background, black text.
    Magenta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// The locale formatter receives a sign-extended 64-bit monetary amount.
    FormatMoney { amount: i64 },
    /// Play on the owning person's current original object ID.
    Sound { name: Vec<u8> },
    /// Display on the owning person through the captured active view.
    Display { text: Vec<u8>, style: Style },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Text(Vec<u8>),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    FormatMoney(i64),
    Sound,
    Display,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    stage: Stage,
    text: Vec<u8>,
    style: Style,
    pending: Option<Request>,
}

impl Execution {
    pub fn pending_request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        validate_text(&self.text)?;
        let valid = match &self.pending {
            Some(Request::FormatMoney { .. }) => {
                self.stage == Stage::Display && self.style == Style::Default
            }
            Some(Request::Sound { name }) => {
                self.stage == Stage::Display
                    && self.style == Style::Yellow
                    && name == b"sting_char_winner"
            }
            Some(Request::Display { text, style }) => {
                self.stage == Stage::Complete && text == &self.text && *style == self.style
            }
            None => true,
        };
        if valid {
            Ok(())
        } else {
            Err("invalid notification dispatch continuation".into())
        }
    }
    pub fn new(input: Input, view: View) -> Result<Self, String> {
        let mut result = Self {
            stage: Stage::Complete,
            text: Vec::new(),
            style: Style::Default,
            pending: None,
        };
        match input {
            Input::AnimationName(text) => {
                // Unlike the numeric entry point, 0x4a698d dereferences the view
                // owner without checking null. Do not turn invalid ownership
                // into successful suppression.
                if view == View::NoOwner {
                    return Err("animation notification requires a view owner".into());
                }
                validate_text(&text)?;
                if view == View::Active {
                    result.text = text;
                    result.stage = Stage::Display;
                }
            }
            Input::Person(value) if view == View::Active => {
                let value = i32::from(value);
                if value < -20000 {
                    result.text = (-20000 - value).to_string().into_bytes();
                    result.style = Style::Magenta;
                    result.stage = Stage::Display;
                } else if value < -10000 {
                    result.text = (-10000 - value).to_string().into_bytes();
                    result.style = Style::Yellow;
                    result.stage = Stage::Sound;
                } else {
                    result.stage = Stage::FormatMoney(i64::from(value));
                }
            }
            Input::Person(_) => (),
        }
        Ok(result)
    }

    pub fn is_complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }

    pub fn next(&mut self) -> Option<Request> {
        if let Some(request) = &self.pending {
            return Some(request.clone());
        }
        let (request, next) = match self.stage {
            Stage::FormatMoney(amount) => (Request::FormatMoney { amount }, Stage::Display),
            Stage::Sound => (
                Request::Sound {
                    name: b"sting_char_winner".to_vec(),
                },
                Stage::Display,
            ),
            Stage::Display => (
                Request::Display {
                    text: self.text.clone(),
                    style: self.style,
                },
                Stage::Complete,
            ),
            Stage::Complete => return None,
        };
        self.stage = next;
        self.pending = Some(request.clone());
        Some(request)
    }

    /// Invalid responses leave the request and all state intact for retry.
    pub fn acknowledge(&mut self, response: Response) -> Result<(), String> {
        match (&self.pending, response) {
            (Some(Request::FormatMoney { .. }), Response::Text(text)) => {
                validate_text(&text)?;
                self.text = text;
            }
            (Some(Request::Sound { .. } | Request::Display { .. }), Response::Done) => (),
            _ => return Err("incorrect native notification response".into()),
        }
        self.pending = None;
        Ok(())
    }
}

fn validate_text(text: &[u8]) -> Result<(), String> {
    if text.contains(&0) {
        return Err("notification text contains an interior NUL".into());
    }
    Ok(())
}
