//! Native private Dialog primitive (0x451552). The decision protocol is separate
//! from widget construction, text formatting and presentation event delivery.
use crate::ObjectId;
use crate::{Frame, Location, Program, Status, Thread, Vm, World};
use serde::{Deserialize, Serialize};
mod career;
pub use career::format_careers;
mod neighbor;
pub use neighbor::format_neighbor;
mod time;
pub use time::{format_time, format_times};
mod construction;
pub use construction::{Construction, Description, NeighborHost, describe_resources};
pub mod presentation;
pub use presentation::Presentation;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Image {
    /// One owned image wrapper, transferred from the construction continuation.
    pub handle: u32,
    pub rect: [i32; 4],
}

/// 4368b6: `gz` selects the first of four button cells; `gzi` keeps the
/// complete image. Both resolve the same global UI resource namespace.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobalBitmap {
    pub id: i32,
    pub button_cell: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedIcon {
    Job(i32),
    Global(GlobalBitmap),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconLookup {
    Live(ObjectId),
    Neighbor(i16),
}

/// The formatter can select a live object's resource or an already resolved
/// neighborhood resource. Later text passes replace that selection; a failed
/// lookup leaves it intact. Catalog handles are not live object IDs.
#[derive(Default)]
struct FormattedIcon {
    live: Option<ObjectId>,
    resource: Option<u32>,
}

/// 436279..43634d resource modes. Bitmap and named-image modes use their
/// separate loaders. The formatter's selection belongs only to mode zero;
/// mode two interprets Stack Object in the neighborhood namespace.
pub fn resource_icon<R>(
    flags: u8,
    formatted: Option<R>,
    caller_present: bool,
    stack: ObjectId,
    mut lookup: impl FnMut(IconLookup) -> Option<R>,
) -> Result<Option<R>, String> {
    Ok(match (flags >> 1) & 7 {
        0 => formatted.or_else(|| {
            (caller_present && stack > 0)
                .then(|| lookup(IconLookup::Live(stack)))
                .flatten()
        }),
        2 => (stack > 0)
            .then(|| lookup(IconLookup::Neighbor(stack)))
            .flatten(),
        1 | 5 | 6 | 7 => None,
        _ => return Err("bitmap/named icon requires its dedicated selector".into()),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Button {
    pub label: String,
    /// Native result 2 (primary), 3 (secondary), 4 (third).
    pub result: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Record {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<Presentation>,
    pub id: u64,
    pub caller: ObjectId,
    pub origin: Location,
    pub source: String,
    pub operands: [u8; 8],
    pub stack_object: ObjectId,
    pub title: String,
    pub body: String,
    pub buttons: Vec<Button>,
    /// Resource identity selected by the supported $Object formatter pass.
    pub icon_object: Option<ObjectId>,
    /// Named "job" icon from STR#301. -1 selects the native school row.
    pub job_icon: Option<i32>,
    /// Explicit BMP_ chunk in the executing frame's resource (`source`).
    pub bitmap_icon: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_bitmap_icon: Option<GlobalBitmap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portrait: Option<Image>,
    pub status: u32,
}
impl Record {
    pub fn key_result(&self, key: u32) -> Option<u32> {
        (self.status == 1)
            .then(|| presentation::keys(self.operands[5]))?
            .into_iter()
            .find(|(k, _)| *k == key)
            .map(|(_, result)| result)
            .filter(|result| self.buttons.iter().any(|b| b.result == *result))
    }
    pub fn modal(&self) -> bool {
        self.operands[7] & 128 == 0
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct State {
    pub protocol: Protocol,
    pub records: Vec<Record>,
    /// Suspension inside the modal presentation call, at the registry UI pass.
    pub modal: Option<u64>,
    pub modal_busy: bool,
    pub special_busy: bool,
    pub input: String,
}

/// 0x4369de: one-based STR# index, an optional comment marker, then fallback.
pub fn select_text(text: Option<&str>, marked: bool, fallback: Option<&str>) -> String {
    let value = text
        .filter(|s| !s.is_empty())
        .or(fallback)
        .unwrap_or("Missing string!!!");
    format!("{}{value}", if marked { "*" } else { "" })
}

fn display_name(p: &Program, w: &World, id: ObjectId) -> Result<String, String> {
    let def = w
        .objects
        .get(&id)
        .and_then(|o| o.definition.as_ref())
        .ok_or("Dialog display-name object definition absent")?;
    p.modules
        .get(&def.source)
        .and_then(|m| m.catalog_names.get(&def.object))
        .cloned()
        .ok_or_else(|| format!("Dialog catalog name/fallback unavailable for #{id}"))
}

/// The readable 0x44bf1a processes names, locals and career tokens in passes.
/// Other token families
/// remain explicit boundaries; unresolved substitutions are never shown as names.
pub(crate) fn format_text(
    p: &Program,
    w: &World,
    f: &Frame,
    text: String,
    icon: &mut Option<ObjectId>,
) -> Result<String, String> {
    let mut selected = FormattedIcon {
        live: *icon,
        resource: None,
    };
    let result = format_text_using(p, w, f, text, &mut selected, false, false, &mut |_, _| {
        Err("Dialog neighbor resource host absent".into())
    })?;
    *icon = selected.live;
    Ok(result)
}

/// Change Action String invokes the same native formatter as Dialog. Preserve
/// recognized tokens whose native pass intentionally leaves them unchanged
/// (for example an out-of-range $Local), while unsupported families still fail.
pub(crate) fn format_action_text(
    p: &Program,
    w: &World,
    f: &Frame,
    text: String,
) -> Result<String, String> {
    format_text_using(
        p,
        w,
        f,
        text,
        &mut FormattedIcon::default(),
        false,
        true,
        &mut |_, _| Err("action-string neighbor resource host absent".into()),
    )
}

fn format_text_using(
    p: &Program,
    w: &World,
    f: &Frame,
    text: String,
    icon: &mut FormattedIcon,
    neighbor_available: bool,
    allow_unexpanded: bool,
    neighbor: &mut impl FnMut(&str, i16) -> Result<(String, Option<u32>), String>,
) -> Result<String, String> {
    let caller = text
        .contains("$Me")
        .then(|| display_name(p, w, f.caller))
        .transpose()?;
    let object = (text.contains("$Object")
        || caller.as_ref().is_some_and(|n| n.contains("$Object")))
    .then(|| display_name(p, w, f.stack_object))
    .transpose()?;
    let (result, used_object) = format_names(&text, caller.as_deref(), object.as_deref())?;
    if used_object {
        *icon = FormattedIcon {
            live: Some(f.stack_object),
            resource: None,
        };
    }
    let result = replace_locals(&result, |index| {
        if f.opaque_locals.contains(&index) {
            return Err("Dialog formatting reads opaque local".into());
        }
        Ok(f.locals.get(index).copied())
    })?;
    let result = if neighbor_available {
        let (text, resource) = neighbor(&result, f.stack_object)?;
        if let Some(resource) = resource {
            *icon = FormattedIcon {
                live: None,
                resource: Some(resource),
            };
        }
        text
    } else {
        result
    };
    let result = time::replace(&result, p.text_language, |index| {
        if f.opaque_locals.contains(&index) {
            return Err("Dialog time formatting reads opaque local".into());
        }
        Ok(f.locals.get(index).copied())
    })?;
    let result = if result.contains("$Job") {
        let person = w
            .objects
            .get(&f.caller)
            .and_then(|o| o.person.as_ref())
            .ok_or("Dialog career substitution requires native person storage")?;
        let fields = [56, 57, 65].map(|i| person.get(&i).copied());
        let [Some(job), Some(level), Some(gender)] = fields else {
            return Err("Dialog career person fields absent".into());
        };
        career::replace(
            &result,
            p.careers
                .as_ref()
                .ok_or("Dialog career catalog unavailable")?,
            [job, level, gender],
            f.locals.len(),
            |index| {
                if f.opaque_locals.contains(&index) {
                    return Err("Dialog career formatting reads opaque local".into());
                }
                f.locals
                    .get(index)
                    .copied()
                    .ok_or("Dialog career local absent".into())
            },
        )?
    } else {
        result
    };
    let unresolved = if neighbor_available || allow_unexpanded {
        // A supported pass may intentionally leave a token intact, or a later
        // name replacement may introduce a token whose pass already ran.
        let text = if neighbor_available {
            result.replace("$Neighbor", "")
        } else {
            result.clone()
        };
        [
            "$TimeLocal",
            "$JobOffer",
            "$JobDesc",
            "$Object",
            "$Local",
            "$Me",
            "$Job",
        ]
        .iter()
        .fold(text, |s, token| s.replace(token, ""))
    } else {
        result.clone()
    };
    if let Some(at) = unresolved
        .as_bytes()
        .windows(2)
        .position(|w| w[0] == b'$' && w[1].is_ascii_alphabetic())
    {
        let token: String = unresolved[at..]
            .chars()
            .take_while(|c| *c == '$' || c.is_ascii_alphanumeric() || *c == ':')
            .collect();
        return Err(format!(
            "Dialog text substitution requires unsupported native token {token}"
        ));
    }
    Ok(result)
}

/// Name-token prefix of 0x44bf1a. A missing resource leaves that token intact.
/// Incrementing the search position by one after replacement is intentional.
pub fn format_names(
    text: &str,
    caller: Option<&str>,
    object: Option<&str>,
) -> Result<(String, bool), String> {
    let mut result = text.to_owned();
    let mut used_object = false;
    for (token, name) in [("$Me", caller), ("$Object", object)] {
        let Some(name) = name else {
            continue;
        };
        let mut at = 0;
        let mut iterations = 0;
        while at < result.len() {
            // Native searches bytes. Advance to a UTF-8 boundary before using
            // Rust string search; no ASCII token can begin inside a codepoint.
            while at < result.len() && !result.is_char_boundary(at) {
                at += 1;
            }
            let Some(found) = result[at..].find(token) else {
                break;
            };
            at += found;
            result.replace_range(at..at + token.len(), name);
            if token == "$Object" {
                used_object = true;
            }
            at += 1;
            iterations += 1;
            if iterations > 65536 || result.len() > 65536 {
                return Err("Dialog name expansion exceeds limit".into());
            }
        }
    }
    Ok((result, used_object))
}

/// Native parameterized-token scan (0x44caf6) and the $Local pass of 0x44bf1a.
/// Only unsigned decimal indices are parsed. A second decimal parameter is
/// consumed, although this formatter ignores its value. CRT integer overflow
/// wraps; invalid/out-of-range locals leave the original token untouched.
pub fn format_locals(text: &str, locals: Option<&[i16]>) -> Result<String, String> {
    replace_locals(text, |i| Ok(locals.and_then(|v| v.get(i)).copied()))
}
fn replace_locals(
    text: &str,
    mut read: impl FnMut(usize) -> Result<Option<i16>, String>,
) -> Result<String, String> {
    let mut result = text.to_owned();
    let mut start = 0;
    while start < result.len() {
        while start < result.len() && !result.is_char_boundary(start) {
            start += 1;
        }
        let Some(at) = result[start..].find("$Local").map(|n| start + n) else {
            break;
        };
        let (end, [index, _]) = parameters(&result, at + 6);
        if let Ok(index) = usize::try_from(index) {
            if let Some(value) = read(index)? {
                result.replace_range(at..end, &value.to_string());
            }
        }
        start = at + 1;
    }
    Ok(result)
}

/// 44caf6 consumes at most two colon-prefixed unsigned decimal parameters.
/// Absent/empty parameters are -1; CRT integer overflow wraps to signed i32.
fn parameters(text: &str, mut end: usize) -> (usize, [i32; 2]) {
    let mut values = [-1; 2];
    let bytes = text.as_bytes();
    for value in &mut values {
        if bytes.get(end) != Some(&b':') {
            break;
        }
        end += 1;
        let first = end;
        let mut number = 0u32;
        while let Some(d) = bytes.get(end).filter(|d| d.is_ascii_digit()) {
            number = number.wrapping_mul(10).wrapping_add(u32::from(*d - b'0'));
            end += 1;
        }
        if end != first {
            *value = number as i32;
        }
    }
    (end, values)
}

/// The named icon constructor splits at the first ASCII space and ignores
/// subsequent spaces. Unknown icon categories remain host boundaries.
pub fn named_icon(text: &str) -> Result<Option<NamedIcon>, String> {
    if text.is_empty() {
        return Ok(None);
    }
    let (category, value) = text.split_once(' ').unwrap_or((text, ""));
    // 46f874/46f891 compares ASCII letters without case; only literal spaces
    // separate the two buffers. Numeric conversion is CRT atoi (636029).
    let category = category.to_ascii_lowercase();
    if !matches!(category.as_str(), "job" | "gz" | "gzi") {
        return Err(format!(
            "Dialog named icon category {category:?} unsupported"
        ));
    }
    let value = value.replace(' ', "");
    let bytes = value
        .trim_start_matches(['\t', '\n', '\r', '\u{b}', '\u{c}'])
        .as_bytes();
    let negative = bytes.first() == Some(&b'-');
    let mut at = usize::from(negative || bytes.first() == Some(&b'+'));
    let mut number = 0u32;
    while let Some(d) = bytes.get(at).filter(|d| d.is_ascii_digit()) {
        number = number.wrapping_mul(10).wrapping_add(u32::from(*d - b'0'));
        at += 1;
    }
    let number = if negative {
        number.wrapping_neg()
    } else {
        number
    } as i32;
    Ok(match category.as_str() {
        "job" => (-1..=21)
            .contains(&number)
            .then_some(NamedIcon::Job(number)),
        _ => (number != 0).then_some(NamedIcon::Global(GlobalBitmap {
            id: number,
            button_cell: category.len() == 2,
        })),
    })
}

pub fn named_job_icon(text: &str) -> Result<Option<i32>, String> {
    match named_icon(text)? {
        None => Ok(None),
        Some(NamedIcon::Job(job)) => Ok(Some(job)),
        Some(NamedIcon::Global(_)) => Err("named icon requires global bitmap loader".into()),
    }
}

/// 0x530a7b selects one of 22 rows from the job popup atlas. The school
/// sentinel -1 maps to row 11. Rectangles use native left/top/right/bottom.
pub fn job_icon_rect(job: i32, bounds: [i32; 4]) -> Option<[i32; 4]> {
    let index = if job == -1 { 11 } else { job };
    if !(0..=21).contains(&index) {
        return None;
    }
    let height = bounds[3] / 22;
    let offset = height.wrapping_sub(bounds[1]).wrapping_mul(index);
    Some([
        bounds[0],
        bounds[1].wrapping_add(offset),
        bounds[2],
        height.wrapping_add(offset),
    ])
}

/// Selector 3 in 0x43601c looks up the frame's code resource in the caller's
/// resource manager, then requests BMP_ at this one-byte operand plus 5000.
pub fn private_bitmap_id(index: u8) -> u16 {
    5000 + u16::from(index)
}

pub fn response_status(current: u32, result: u32, immediate: bool) -> u32 {
    if (2..5).contains(&result) {
        if immediate { 5 } else { result }
    } else {
        current
    }
}

fn describe(p: &Program, w: &World, f: &Frame, b: [u8; 8], id: u64) -> Result<Record, String> {
    describe_using(p, w, f, b, id, false, &mut |_, _| {
        Err("Dialog neighbor resource host absent".into())
    })
    .map(|(record, _)| record)
}

fn describe_using(
    p: &Program,
    w: &World,
    f: &Frame,
    b: [u8; 8],
    id: u64,
    neighbor_available: bool,
    neighbor: &mut impl FnMut(&str, i16) -> Result<(String, Option<u32>), String>,
) -> Result<(Record, Option<u32>), String> {
    let mode = b[5] & 127;
    if !presentation::supported(mode) {
        return Err(format!("Dialog presentation mode {mode} unsupported"));
    }
    let presentation = Presentation::capture(
        Some(f.caller),
        w.ui_owner.owner,
        w.objects
            .get(&f.caller)
            .and_then(|o| o.definition.as_ref())
            .map(|d| d.guid),
        b[5],
    );
    let icon_mode = (b[7] >> 1) & 7;
    if icon_mode == 2 && !neighbor_available {
        return Err("Dialog explicit icon selection unsupported".into());
    }
    let module = p
        .modules
        .get(&f.code_owner)
        .ok_or("Dialog private resource absent")?;
    let strings = module.strings.get(&301);
    let entry = |index: u8| {
        strings
            .and_then(|s| usize::from(index).checked_sub(1).and_then(|i| s.get(i)))
            .map(String::as_str)
    };
    let select = |index: u8, fallback| {
        select_text(
            entry(index),
            module.dialog_marked.contains(&usize::from(index)),
            fallback,
        )
    };
    let title_fallback = if entry(b[6]).is_none_or(str::is_empty) {
        Some(display_name(p, w, f.caller)?)
    } else {
        None
    };
    let mut title = select(b[6], title_fallback.as_deref());
    let mut body = select(b[2], None);
    let labels = p
        .modules
        .values()
        .filter(|m| m.source.to_ascii_lowercase().ends_with("uitext.iff"))
        .filter_map(|m| m.strings.get(&142))
        .collect::<Vec<_>>();
    let default = |index: usize| -> Result<&str, String> {
        if labels.len() != 1 {
            return Err("Dialog default button resource absent/ambiguous".into());
        }
        labels[0]
            .get(index - 1)
            .map(String::as_str)
            .ok_or("Dialog default button string absent".into())
    };
    let mut buttons = presentation::buttons(mode, [b[3], b[4], b[0]])
        .into_iter()
        .map(|spec| {
            Ok(Button {
                label: select(spec.index, Some(default(usize::from(spec.fallback))?)),
                result: spec.result,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    buttons.sort_by_key(|button| button.result);
    let mut icon = FormattedIcon::default();
    for button in &mut buttons {
        button.label = format_text_using(
            p,
            w,
            f,
            std::mem::take(&mut button.label),
            &mut icon,
            neighbor_available,
            false,
            neighbor,
        )?;
    }
    title = format_text_using(
        p,
        w,
        f,
        title,
        &mut icon,
        neighbor_available,
        false,
        neighbor,
    )?;
    body = format_text_using(
        p,
        w,
        f,
        body,
        &mut icon,
        neighbor_available,
        false,
        neighbor,
    )?;
    let named = if icon_mode == 4 {
        icon = FormattedIcon::default();
        let text = select_text(entry(b[1]), false, Some(""));
        let text = format_text_using(
            p,
            w,
            f,
            text,
            &mut FormattedIcon::default(),
            neighbor_available,
            false,
            neighbor,
        )?;
        named_icon(&text)?
    } else {
        None
    };
    let job_icon = match named {
        Some(NamedIcon::Job(job)) => Some(job),
        _ => None,
    };
    let global_bitmap_icon = match named {
        Some(NamedIcon::Global(image)) => Some(image),
        _ => None,
    };
    let bitmap_icon = if icon_mode == 3 {
        icon = FormattedIcon::default();
        Some(private_bitmap_id(b[1]))
    } else {
        None
    };
    // 436279..43634d starts with a null resource. Mode 1 and the unused
    // selectors 5..7 keep it null, even when $Object selected a resource
    // during formatting. Reuse the native-verified selector for those modes.
    if !matches!(icon_mode, 2 | 3 | 4) && !(icon_mode == 0 && icon.resource.is_some()) {
        icon.resource = None;
        icon.live = resource_icon(b[7], icon.live, true, f.stack_object, |lookup| {
            let IconLookup::Live(id) = lookup else {
                return None;
            };
            w.objects
                .get(&id)
                .filter(|o| o.definition.is_some())
                .map(|_| id)
        })?;
    }
    if icon_mode == 2 {
        icon = FormattedIcon::default();
    }
    Ok((
        Record {
            presentation: Some(presentation),
            id,
            caller: f.caller,
            origin: f.location.clone(),
            source: f.code_owner.clone(),
            operands: b,
            stack_object: f.stack_object,
            title,
            body,
            buttons,
            icon_object: icon.live,
            job_icon,
            bitmap_icon,
            global_bitmap_icon,
            portrait: None,
            status: 0,
        },
        icon.resource,
    ))
}

impl World {
    /// Apply native synchronous closes before any later primitive can inspect
    /// a dialog result. Signal 0x107 remains in the presentation effect queue.
    pub fn apply_dialog_closes(&mut self) -> Result<(), String> {
        if self.dialogs.records.is_empty() {
            return Ok(());
        }
        if self.ui_owner.dialog_owners
            != self
                .dialogs
                .records
                .iter()
                .map(|r| r.caller)
                .collect::<Vec<_>>()
        {
            return Err("Dialog owner registry differs from UI ownership registry".into());
        }
        for effect in &self.ui_owner.effects {
            if let crate::ui_owner::Effect::CloseDialog(i) = effect {
                self.dialogs
                    .records
                    .get_mut(*i)
                    .ok_or("Dialog close index absent")?
                    .status = 5;
            }
        }
        self.ui_owner
            .effects
            .retain(|e| !matches!(e, crate::ui_owner::Effect::CloseDialog(_)));
        if self.dialogs.modal.is_some_and(|id| {
            self.dialogs
                .records
                .iter()
                .any(|r| r.id == id && r.status == 5)
        }) {
            self.dialogs.modal = None;
        }
        Ok(())
    }
}

impl Vm {
    pub(crate) fn dialog_primitive(
        &mut self,
        p: &Program,
        t: &mut Thread,
        ins: &sims_formats::behavior::Instruction,
    ) -> Result<String, String> {
        self.dialog_primitive_with_observation(p, t, ins, &mut crate::action_observation::Observation::disabled())
    }

    pub(crate) fn dialog_primitive_with_observation(
        &mut self,
        p: &Program,
        t: &mut Thread,
        ins: &sims_formats::behavior::Instruction,
        observation: &mut crate::action_observation::Observation<'_>,
    ) -> Result<String, String> {
        let f = t.stack.last().ok_or("Dialog frame absent")?;
        let mode = ins.operands[5] & 127;
        if !presentation::supported(mode) {
            return Err(format!(
                "Dialog specialized/input host mode {mode} unsupported"
            ));
        }
        if t.check && ins.operands[7] & 1 == 0 {
            return Err("Dialog yielding synchronous callback unsupported".into());
        }
        self.world.apply_dialog_closes()?;
        let caller = self
            .world
            .objects
            .get(&f.caller)
            .ok_or("Dialog caller absent")?;
        let mut temps = caller.temps;
        let mut protocol = self.world.dialogs.protocol.clone();
        let index = self
            .world
            .dialogs
            .records
            .iter()
            .position(|d| d.caller == f.caller);
        let mut status = index.map(|i| self.world.dialogs.records[i].status);
        let step = protocol.step(
            Input {
                caller: f.caller,
                caller_kind: 0,
                caller_neighbor: 0,
                stack_person: None,
                operands: ins.operands,
                modal_busy: self.world.dialogs.modal_busy,
                special_busy: self.world.dialogs.special_busy,
            },
            &mut temps,
            &mut status,
        )?;
        if step.effects.contains(&Effect::Create) && matches!((ins.operands[7] >> 1) & 7, 0 | 2) {
            if self.world.dialogs.records.len() >= 4096 {
                return Err("Dialog registry exceeds limit".into());
            }
            // In modes 0..2 and 4, Create only closes an old record before calling
            // the constructor. No protocol/register copy may be committed
            // after reentrant construction, where callbacks can change them.
            if protocol != self.world.dialogs.protocol || temps != caller.temps {
                return Err("Dialog creation unexpectedly changed protocol/registers".into());
            }
            let id = self.next_request;
            self.next_request = id.checked_add(1).ok_or("request ID overflow")?;
            if let Some(i) = index {
                self.world.dialogs.records[i].status = status.unwrap();
            }
            t.status = Status::AwaitingDialog(Box::new(Construction::new(
                id,
                self.tick,
                f.clone(),
                ins.operands,
                step.returned,
            )));
            return Ok("Dialog construction pending".into());
        }
        let created = if step.effects.contains(&Effect::Create) {
            if self.world.dialogs.records.len() >= 4096 {
                return Err("Dialog registry exceeds limit".into());
            }
            Some(describe(
                p,
                &self.world,
                f,
                ins.operands,
                self.next_request,
            )?)
        } else {
            None
        };
        // Unsupported text/presentation dependencies above stop before protocol mutation.
        if created.is_some() {
            self.next_request = self
                .next_request
                .checked_add(1)
                .ok_or("request ID overflow")?;
        }
        self.world.dialogs.protocol = protocol;
        self.world.objects.get_mut(&f.caller).unwrap().temps = temps;
        if let Some(i) = index {
            self.world.dialogs.records[i].status = status.unwrap();
        }
        if let Some(record) = created {
            self.world.ui_owner.dialog_owners.push(record.caller);
            self.world.dialogs.records.push(record);
        }
        if step.returned == 2 {
            t.status = Status::Sleeping {
                wake_tick: self.tick.checked_add(1).ok_or("Dialog tick overflow")?,
            };
        } else {
            crate::branch_observed(
                p,
                t,
                if step.returned == 1 {
                    ins.on_true
                } else {
                    ins.on_false
                },
                Some(&self.world),
                observation,
            )?;
        }
        Ok(format!("Dialog result {}", step.returned))
    }

    /// Registry UI prefix of 0x4412d1. A modal prompt suspends that pass until
    /// an explicit response; it does not advance the simulation clock.
    pub fn pump_dialogs(&mut self) -> Result<Option<u64>, String> {
        self.world.apply_dialog_closes()?;
        if let Some(id) = self.world.dialogs.modal {
            return Ok(Some(id));
        }
        let mut i = 0;
        while i < self.world.dialogs.records.len() {
            let record = &mut self.world.dialogs.records[i];
            if record.status == 5 {
                let removed = self.world.dialogs.records.remove(i);
                if let Some(image) = removed.portrait {
                    self.world.image_cache.assign_image(
                        &mut Some(image.handle),
                        None,
                        &mut |_, _| {},
                    )?;
                }
                self.world.ui_owner.dialog_owners.remove(i);
            } else if record.status == 0 {
                record.status = 1;
                if record.modal() {
                    self.world.dialogs.modal = Some(record.id);
                    return Ok(Some(record.id));
                }
            } else {
                i += 1;
            }
        }
        Ok(None)
    }

    /// The button result is a player command. No default answer is generated.
    /// 0x4370be retains 2..4, except fire-and-forget records close immediately.
    pub fn answer_dialog(&mut self, id: u64, result: u32) -> Result<(), String> {
        let record = self
            .world
            .dialogs
            .records
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("Dialog response request absent")?;
        if record.status != 1 || !record.buttons.iter().any(|b| b.result == result) {
            return Err("Dialog response is stale or not an offered button".into());
        }
        if record.modal() && self.world.dialogs.modal != Some(id) {
            return Err("Dialog response is not the current modal prompt".into());
        }
        record.status = response_status(record.status, result, record.operands[7] & 1 != 0);
        if self.world.dialogs.modal == Some(id) {
            self.world.dialogs.modal = None;
        }
        Ok(())
    }

    pub(crate) fn validate_dialogs(&self, p: &Program) -> Result<(), String> {
        self.validate_dialog_image_ownership(None)?;
        let d = &self.world.dialogs;
        let mut ids = std::collections::BTreeSet::new();
        if d.records.len() > 4096
            || d.input.len() > 65536
            || d.protocol
                .owners
                .iter()
                .flatten()
                .any(|id| !self.world.objects.contains_key(id))
        {
            return Err("invalid saved Dialog registry".into());
        }
        if !d.records.is_empty()
            && self.world.ui_owner.dialog_owners
                != d.records.iter().map(|r| r.caller).collect::<Vec<_>>()
        {
            return Err("saved Dialog ownership registry mismatch".into());
        }
        for r in &d.records {
            let (source, routine) = p.resolve(&r.source, r.origin.bhav)?;
            let ins = routine
                .body
                .instructions
                .get(r.origin.instruction)
                .ok_or("saved Dialog instruction absent")?;
            let mode = r.operands[5] & 127;
            if r.id == 0
                || r.id >= self.next_request
                || !ids.insert(r.id)
                || r.status > 5
                || !self.world.objects.contains_key(&r.caller)
                || !p.modules.contains_key(&r.source)
                || source != r.origin.source
                || ins.opcode != 0x24
                || ins.operands != r.operands
                || !presentation::supported(mode)
                || r.job_icon
                    .is_some_and(|job| !(-1..=21).contains(&job) || (r.operands[7] >> 1) & 7 != 4)
                || r.global_bitmap_icon.is_some_and(|icon| {
                    icon.id == 0 || (r.operands[7] >> 1) & 7 != 4 || r.job_icon.is_some()
                })
                || (r.operands[7] >> 1) & 7 != 0 && r.icon_object.is_some()
                || r.portrait.as_ref().is_some_and(|image| {
                    !matches!((r.operands[7] >> 1) & 7, 0 | 2)
                        || !matches!(image.rect, [0, 0, 45, 45] | [0, 0, 120, 120])
                        || self
                            .world
                            .image_cache
                            .refs
                            .get(image.handle.wrapping_sub(1) as usize)
                            .is_none_or(|r| *r == 0)
                })
                || r.bitmap_icon
                    != (((r.operands[7] >> 1) & 7 == 3).then(|| private_bitmap_id(r.operands[1])))
                || r.title.len() > 65536
                || r.body.len() > 65536
                || r.buttons.len() != presentation::button_count(mode)
                || r.presentation
                    .is_some_and(|p| p.large != (r.operands[5] & 128 != 0))
                || r.buttons
                    .iter()
                    .enumerate()
                    .any(|(i, b)| b.result != i as u32 + 2 || b.label.len() > 65536)
            {
                return Err("invalid saved Dialog record".into());
            }
        }
        if d.modal.is_some_and(|id| {
            d.records
                .iter()
                .find(|r| r.id == id)
                .is_none_or(|r| r.status != 1 || !r.modal())
        }) {
            return Err("invalid saved modal Dialog suspension".into());
        }
        Ok(())
    }
}

/// Registry words at e8, ea, ec, ee, f0, f2, f4, f6, 100, 108 respectively.
/// These are shared selection channels, not fields of an individual dialog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Protocol {
    pub words: [i16; 10],
    /// Registry pointers f8, fc, 104, represented by stable object identities.
    pub owners: [Option<ObjectId>; 3],
}

#[derive(Debug, Clone, Copy)]
pub struct Input {
    pub caller: ObjectId,
    pub caller_kind: u16,
    pub caller_neighbor: i16,
    pub stack_person: Option<ObjectId>,
    pub operands: [u8; 8],
    pub modal_busy: bool,
    pub special_busy: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Close,
    Create,
    Acknowledge,
    CopyInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Original primitive result: 0 false, 1 true, 2 yield.
    pub returned: u8,
    pub effects: Vec<Effect>,
}

impl Protocol {
    /// `status` is the first owner-matching record, even when it is closed.
    /// Looking up the record and removing it are separate registry operations.
    pub fn step(
        &mut self,
        input: Input,
        temps: &mut [i16; 8],
        status: &mut Option<u32>,
    ) -> Result<Step, String> {
        let mut effects = vec![];
        let mut dialog = *status;
        if dialog == Some(1) {
            effects.push(Effect::Close);
            *status = Some(5);
            dialog = None;
        } else if dialog == Some(5) {
            dialog = None;
        }
        let finish = |returned, effects| Ok(Step { returned, effects });
        if input.operands[7] & 1 != 0 {
            effects.push(Effect::Create);
            return finish(1, effects);
        }
        let mode = input.operands[5] & 0x7f;
        match mode {
            5 | 7 | 8 | 11 | 13 => {
                let index = match mode {
                    5 => 1,
                    7 => 2,
                    8 => 5,
                    11 => 3,
                    _ => 4,
                };
                if self.words[index] == 0 {
                    if mode != 13 || !input.special_busy {
                        self.words[index] = -1;
                    }
                    return finish(2, effects);
                }
                temps[0] = self.words[index];
                self.words[index] = 0;
                finish(1, effects)
            }
            6 | 14 => {
                if input.caller_kind != 2 {
                    return Err("Dialog requires person (native error 0x1c)".into());
                }
                let owner = usize::from(mode == 14);
                let word = if mode == 14 { 7 } else { 6 };
                let Some(existing) = self.owners[owner] else {
                    self.words[word] = temps[0];
                    self.owners[owner] = Some(input.caller);
                    return finish(2, effects);
                };
                temps[0] = if existing == input.caller {
                    self.words[7]
                } else {
                    0
                };
                let accepted = existing == input.caller && self.words[6] == -2;
                self.words[word] = 0;
                self.owners[owner] = None;
                finish(u8::from(accepted), effects)
            }
            10 => {
                if input.caller_kind != 2 {
                    return Err("Dialog requires person (native error 0x1c)".into());
                }
                let result = self.words[0];
                if result == 0 {
                    self.words[0] = input.caller_neighbor.wrapping_neg();
                    return finish(2, effects);
                }
                if result > 0 {
                    temps[0] = result;
                }
                self.words[0] = 0;
                finish(u8::from(result > 0), effects)
            }
            9 => {
                let person = input
                    .stack_person
                    .ok_or("Dialog stack person absent (native error 0x1c)")?;
                let Some(owner) = self.owners[2] else {
                    self.owners[2] = Some(person);
                    return finish(2, effects);
                };
                temps[0] = if owner == person { self.words[8] } else { 0 };
                let accepted = owner == person && self.words[9] == -2;
                if !accepted {
                    self.words[8] = -1;
                }
                self.owners[2] = None;
                finish(u8::from(accepted), effects)
            }
            12 | 15 => Err(format!("Dialog specialized panel mode {mode} unsupported")),
            _ => {
                let Some(result) = dialog else {
                    if !input.modal_busy {
                        effects.push(Effect::Create);
                    }
                    return finish(2, effects);
                };
                effects.push(Effect::Acknowledge);
                if (2..5).contains(&result) {
                    *status = Some(5);
                }
                let temp = usize::from((input.operands[7] & 0x70) >> 4);
                let returned = match result {
                    0 | 1 => 2,
                    2 => {
                        if mode == 3 {
                            effects.push(Effect::CopyInput);
                        }
                        1
                    }
                    3 => {
                        if mode == 2 {
                            temps[temp] = 0;
                        }
                        self.words[8] = -1;
                        0
                    }
                    4 => {
                        if mode == 2 {
                            temps[temp] = 1;
                        }
                        0
                    }
                    _ => 1,
                };
                finish(returned, effects)
            }
        }
    }
}
