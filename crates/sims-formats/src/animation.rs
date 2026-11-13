//! TS1 animation-only binary BCF containers and their separate CFP sample streams.
//! Native coordinates are preserved; renderer-specific axis conversion belongs elsewhere.
use crate::{
    Result,
    reader::{Reader, western},
};
use serde::Serialize;

const MAX_SAMPLES: usize = 1_000_000;
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Motion {
    pub bone: String,
    pub frames: u32,
    pub duration_ms: f32,
    pub translation: Option<u32>,
    pub rotation: Option<u32>,
    pub properties: Vec<Vec<(String, String)>>,
    /// Motion-owned optional timed-property list in original file order (0 or 1
    /// lists). The file uses a nonzero presence word, not a list count.
    pub event_lists: Vec<Vec<TimedProperty>>,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TimedProperty {
    pub time_ms: i32,
    pub bone: String,
    pub pairs: Vec<(String, String)>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Animation {
    pub name: String,
    pub skill: String,
    pub duration_ms: f32,
    pub distance: f32,
    pub moving: bool,
    pub translation_count: u32,
    pub rotation_count: u32,
    pub frames: u32,
    pub motions: Vec<Motion>,
    /// Legacy nonnegative, stably time-sorted view. Native layer playback uses
    /// each motion's raw event list and native replacement/ordering semantics.
    pub events: Vec<TimedProperty>,
}
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Samples {
    pub translations: Vec<[f32; 3]>,
    pub rotations: Vec<[f32; 4]>,
}

pub(crate) fn string(r: &mut Reader<'_>) -> Result<String> {
    let prefix = r.u8()?;
    let n = if prefix == 255 {
        r.u32()? as usize
    } else {
        prefix as usize
    };
    // 5b9201's escape is shared with NDX. These record readers use 256-byte
    // native buffers, including the terminator; do not mimic their overflow.
    if n >= 256 {
        return r.fail("BCF string exceeds supported native buffer");
    }
    let bytes = r.take(n)?;
    if bytes.contains(&0) {
        return r.fail("embedded NUL in BCF string");
    }
    Ok(western(bytes))
}
pub(crate) fn count(r: &mut Reader<'_>, limit: usize, min: usize) -> Result<usize> {
    let n = r.u32()? as usize;
    if n > limit {
        return r.fail("BCF count exceeds limit");
    }
    r.count(n, min)?;
    Ok(n)
}
pub(crate) fn finite(r: &mut Reader<'_>) -> Result<f32> {
    let n = r.f32()?;
    if !n.is_finite() {
        return r.fail("non-finite animation value");
    }
    Ok(n)
}
pub(crate) fn pairs(r: &mut Reader<'_>) -> Result<Vec<(String, String)>> {
    (0..count(r, 4096, 2)?)
        .map(|_| Ok((string(r)?, string(r)?)))
        .collect()
}

/// Reads an animation-only BCF. Motions with zero frames are kept, as the
/// original catalog keeps them (596818); playback checks a motion's channels
/// before sampling, so an empty motion animates nothing.
pub fn read_bcf(data: &[u8]) -> Result<Vec<Animation>> {
    let mut r = Reader::new(data);
    if r.u32()? != 0 || r.u32()? != 0 {
        return r.fail("only animation-only binary BCF containers are supported");
    }
    let mut animations = Vec::new();
    for _ in 0..count(&mut r, 4096, 26)? {
        animations.push(read_animation(&mut r)?);
    }
    if r.remaining() != 0 {
        return r.fail("trailing bytes in BCF container");
    }
    Ok(animations)
}

pub(crate) fn read_animation(r: &mut Reader<'_>) -> Result<Animation> {
    let name = string(r)?;
    let skill = string(r)?;
    if name.is_empty() || skill.is_empty() {
        return r.fail("empty animation or CFP skill name");
    }
    let duration_ms = finite(r)?;
    let distance = finite(r)?;
    if !(0.0..=3_600_000.0).contains(&duration_ms) {
        return r.fail("animation duration outside bounds");
    }
    let moving = match r.u32()? {
        0 => false,
        1 => true,
        _ => return r.fail("invalid animation moving flag"),
    };
    let translation_count = r.u32()?;
    let rotation_count = r.u32()?;
    if translation_count as usize > MAX_SAMPLES || rotation_count as usize > MAX_SAMPLES {
        return r.fail("animation sample count exceeds limit");
    }
    let mut motions = Vec::new();
    let mut events = Vec::new();
    let mut event_total = 0;
    let mut frames = 0;
    for _ in 0..count(r, 1024, 33)? {
        let bone = string(r)?;
        let n = r.u32()?;
        let duration = finite(r)?;
        if n > 108_001 || !(0.0..=3_600_000.0).contains(&duration) {
            return r.fail("invalid motion frames or duration");
        }
        frames = frames.max(n);
        let ht = r.u32()?;
        let hr = r.u32()?;
        let first_t = r.i32()?;
        let first_r = r.i32()?;
        let channel = |flag: u32, index: i32, total: u32| -> Result<Option<u32>> {
            if flag > 1 {
                return r.fail("invalid motion channel flag");
            }
            if flag == 0 {
                return Ok(None);
            }
            if index < 0 || (index as u32).checked_add(n).is_none_or(|end| end > total) {
                return r.fail("motion samples outside CFP channel");
            }
            Ok(Some(index as u32))
        };
        let translation = channel(ht, first_t, translation_count)?;
        let rotation = channel(hr, first_r, rotation_count)?;
        let mut properties = Vec::new();
        if r.u32()? != 0 {
            properties.push(pairs(r)?);
        }
        let mut event_lists = Vec::new();
        if r.u32()? != 0 {
            let mut list = Vec::new();
            for _ in 0..count(r, 16384, 8)? {
                let time = r.i32()?;
                if event_total >= 16384 {
                    return r.fail("animation events exceed limit");
                }
                event_total += 1;
                let event = TimedProperty {
                    time_ms: time,
                    bone: bone.clone(),
                    pairs: pairs(r)?,
                };
                list.push(event);
            }
            // Legacy playback consumes nonnegative sorted events. Native
            // cursor services retain signed times in the motion-owned list.
            events.extend(list.iter().filter(|e| e.time_ms >= 0).cloned());
            event_lists.push(list);
        }
        motions.push(Motion {
            bone,
            frames: n,
            duration_ms: duration,
            translation,
            rotation,
            properties,
            event_lists,
        });
    }
    // Stable ordering retains original motion/list order for simultaneous events.
    events.sort_by_key(|e| e.time_ms);
    Ok(Animation {
        name,
        skill,
        duration_ms,
        distance,
        moving,
        translation_count,
        rotation_count,
        frames,
        motions,
        events,
    })
}

pub fn read_cfp(data: &[u8], translations: u32, rotations: u32) -> Result<Samples> {
    if translations as usize > MAX_SAMPLES || rotations as usize > MAX_SAMPLES {
        return Err(crate::reader::Error::new(
            0,
            "CFP sample count exceeds limit",
        ));
    }
    let mut r = Reader::new(data);
    fn channel(r: &mut Reader<'_>, n: usize) -> Result<Vec<f32>> {
        let mut out = Vec::with_capacity(n);
        let mut last = 0.0f32;
        while out.len() < n {
            let code = r.u8()?;
            match code {
                255 => last = finite(r)?,
                254 => {
                    // The count is additional copies; the opcode also emits one value.
                    let copies = usize::from(r.u16()?) + 1;
                    if copies > n - out.len() {
                        return r.fail("CFP repetition crosses a component boundary");
                    }
                    out.resize(out.len() + copies, last);
                    continue;
                }
                253 => return r.fail("reserved CFP opcode 0xFD"),
                value => {
                    let x = i64::from(value) - 126;
                    let delta = (3.9676e-10_f64 * (x * x * x * x.abs()) as f64) as f32;
                    last += delta;
                    if !last.is_finite() {
                        return r.fail("non-finite CFP delta result");
                    }
                }
            }
            out.push(last);
        }
        Ok(out)
    }
    let mut result = Samples {
        translations: vec![[0.0; 3]; translations as usize],
        rotations: vec![[0.0; 4]; rotations as usize],
    };
    for axis in 0..3 {
        for (point, value) in result
            .translations
            .iter_mut()
            .zip(channel(&mut r, translations as usize)?)
        {
            point[axis] = value;
        }
    }
    for axis in 0..4 {
        for (point, value) in result
            .rotations
            .iter_mut()
            .zip(channel(&mut r, rotations as usize)?)
        {
            point[axis] = value;
        }
    }
    if r.remaining() != 0 {
        return r.fail("trailing CFP bytes");
    }
    Ok(result)
}
