//! Native 434657 draw-state propagation and 4aacb9 presenter classification.
//! Draw-order cache construction and preceding F6 camera/floor work belong to
//! their callers. The invalidation service completes before the flag store.
use crate::{ObjectId, refresh::DrawState};
use serde::{Deserialize, Serialize};
pub mod redraw;

pub trait OrderView {
    fn rotation(&self) -> u32;
    fn size(&self) -> i32;
    fn registered(&self) -> &[ObjectId];
    fn cached_tile(&self, id: ObjectId) -> Result<[i8; 3], String>;
}

/// 540809/541346/541389 use the same compiler sort and 44054c signed-byte
/// position comparator. Equal positions retain the compiler's tie behavior.
pub fn sort_wall_records(ids: &mut [usize], positions: &[[i8; 3]]) -> Result<(), String> {
    if ids
        .iter()
        .any(|&id| id >= positions.len() || id > i32::MAX as usize)
    {
        return Err("wall sort references an absent position".into());
    }
    let key = |id: i32| {
        let [x, y, _] = positions[id as usize];
        (i32::from(x) + i32::from(y), i32::from(x) - i32::from(y))
    };
    let mut native: Vec<_> = ids.iter().map(|&id| id as i32).collect();
    crate::nearby_houses::sort_by(&mut native, &|a, b| key(a) < key(b));
    for (dest, id) in ids.iter_mut().zip(native) {
        *dest = id as usize;
    }
    Ok(())
}

/// ObjectModule +44/+48/+4c. Invalidation belongs to the registry caller;
/// requesting an already current cache does not inspect live objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub dirty: bool,
    pub rotation: u32,
    pub objects: Vec<ObjectId>,
}
impl Order {
    pub fn validate(&self) -> Result<(), String> {
        if self.rotation > 3 || self.objects.iter().any(|id| *id <= 0) {
            return Err("invalid native presenter order cache".into());
        }
        Ok(())
    }
    pub fn get(&mut self, view: &impl OrderView) -> Result<&[ObjectId], String> {
        self.validate()?;
        let rotation = view.rotation();
        if rotation > 3 {
            return Err("invalid native presenter rotation".into());
        }
        if self.dirty || self.rotation != rotation {
            self.dirty = false;
            self.rotation = rotation;
            rebuild_order(&mut self.objects, view, rotation)?;
        }
        Ok(&self.objects)
    }
}

///Registry+44/+48/+4c, independent of any viewer's lifetime.43f3e6 sets dirty
///and constructs an empty vector but leaves the cached rotation unwritten.
///After the first440349 rebuild, rotation is an actual retained native value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryOrder {
    pub dirty: bool,
    pub rotation: Option<u32>,
    pub objects: Vec<ObjectId>,
}
impl crate::World {
    ///Successful registry membership and actual SetObject XY publication share
    ///this native writer. Missing historical ownership is never initialized.
    pub fn invalidate_registry_order(&mut self) {
        if let Some(cache) = &mut self.registry_order {
            cache.invalidate();
        }
    }
    pub fn validate_registry_order(&self) -> Result<(), String> {
        if let Some(cache) = &self.registry_order {
            cache.validate()?;
            if !cache.dirty
                && cache
                    .objects
                    .iter()
                    .any(|id| !self.objects.contains_key(id))
            {
                return Err("clean registry spatial cache references an absent object".into());
            }
        }
        Ok(())
    }
}
impl RegistryOrder {
    pub fn constructed() -> Self {
        Self {
            dirty: true,
            rotation: None,
            objects: vec![],
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.rotation.is_some_and(|r| r > 3)
            || self.objects.iter().any(|id| *id <= 0)
            || self.rotation.is_none() && (!self.dirty || !self.objects.is_empty())
        {
            return Err("invalid registry spatial-order cache".into());
        }
        Ok(())
    }
    ///A writer marks stale without changing the last vector or rotation. The
    ///next actual getter, rather than invalidation or deserialization, rebuilds.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
    pub fn get(&mut self, view: &impl OrderView) -> Result<&[ObjectId], String> {
        self.validate()?;
        let rotation = view.rotation();
        if rotation > 3 {
            return Err("invalid native presenter rotation".into());
        }
        if self.dirty || self.rotation != Some(rotation) {
            self.dirty = false;
            self.rotation = Some(rotation);
            rebuild_order(&mut self.objects, view, rotation)?;
        }
        Ok(&self.objects)
    }
}

fn rebuild_order(
    objects: &mut Vec<ObjectId>,
    view: &impl OrderView,
    rotation: u32,
) -> Result<(), String> {
    objects.clear();
    let mut keys = std::collections::BTreeMap::new();
    for &id in view.registered() {
        if id <= 0 {
            return Err("null native registry object".into());
        }
        let tile = view.cached_tile(id)?;
        if !outside(tile, view.size()) {
            objects.push(id);
            keys.insert(i32::from(id), draw_key(tile, rotation)?);
        }
    }
    let mut ids: Vec<_> = objects.iter().map(|id| i32::from(*id)).collect();
    crate::nearby_houses::sort_by(&mut ids, &|a, b| keys[&a] < keys[&b]);
    *objects = ids.into_iter().map(|id| id as ObjectId).collect();
    Ok(())
}

/// 433087 uses signed cached tile bytes, not continuous placement coordinates.
pub fn outside([x, y, _]: [i8; 3], size: i32) -> bool {
    let end = size.wrapping_sub(1);
    i32::from(x) < 1 || i32::from(y) < 1 || i32::from(x) >= end || i32::from(y) >= end
}
fn draw_key([x, y, _]: [i8; 3], rotation: u32) -> Result<(i32, i32), String> {
    let (x, y) = (i32::from(x), i32::from(y));
    if rotation != 0 && (!(0..64).contains(&x) || !(0..64).contains(&y)) {
        return Err("native presenter tile outside rotation table".into());
    }
    let (x, y) = match rotation {
        0 => (x, y),
        1 => (63 - y, x),
        2 => (63 - x, 63 - y),
        3 => (y, 63 - x),
        _ => return Err("invalid native presenter rotation".into()),
    };
    Ok((x + y, x - y))
}

pub trait DrawView {
    fn slots(&self, id: ObjectId) -> Result<&[Option<ObjectId>], String>;
    fn exists(&self, id: ObjectId) -> bool;
    fn definition_flags(&self, id: ObjectId) -> Result<u16, String>;
    fn force_dynamic(&self) -> bool;
    fn visible_level(&self) -> i32;
    fn clock(&self) -> u32;
    fn draw(&self, id: ObjectId) -> Result<DrawState, String>;
    fn write_draw(&mut self, id: ObjectId, draw: DrawState) -> Result<(), String>;
}

pub trait DurationView: DrawView {
    fn is_person(&self, id: ObjectId) -> Result<bool, String>;
    /// Base object byte +188. This is distinct from the current dynamic flag.
    fn keep_dynamic(&self, id: ObjectId) -> Result<bool, String>;
}
/// Base 43458c recurses through live slots; person virtual4226b6 returns MAX.
pub fn duration(view: &impl DurationView, id: ObjectId) -> Result<i32, String> {
    fn visit(
        view: &impl DurationView,
        id: ObjectId,
        ancestors: &mut std::collections::BTreeSet<ObjectId>,
    ) -> Result<i32, String> {
        if !ancestors.insert(id) {
            return Err("cyclic native draw duration".into());
        }
        if view.is_person(id)? {
            ancestors.remove(&id);
            return Ok(i32::MAX);
        }
        let mut maximum = if view.keep_dynamic(id)? {
            i32::MAX
        } else {
            i32::MIN
        };
        for &child in view.slots(id)?.iter().flatten() {
            if view.exists(child) {
                maximum = maximum.max(visit(view, child, ancestors)?);
            }
        }
        ancestors.remove(&id);
        Ok(maximum.max(1000))
    }
    visit(view, id, &mut Default::default())
}

fn forcing_child(view: &impl DrawView, id: ObjectId) -> Result<bool, String> {
    for child in view.slots(id)?.iter().flatten() {
        if view.exists(*child) && (view.definition_flags(*child)? & 1 != 0 || view.force_dynamic())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Start,
    Invalidate,
    Store,
    Stamp,
    Children,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Frame {
    id: ObjectId,
    dynamic: bool,
    no_children: bool,
    phase: Phase,
    index: usize,
    end: usize,
}
impl Frame {
    fn new(id: ObjectId, dynamic: bool, no_children: bool) -> Self {
        Self {
            id,
            dynamic,
            no_children,
            phase: Phase::Start,
            index: 0,
            end: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawUpdate {
    root: ObjectId,
    requested: bool,
    no_children: bool,
    frames: Vec<Frame>,
}
impl DrawUpdate {
    pub fn new(root: ObjectId, dynamic: bool, no_children: bool) -> Self {
        Self {
            root,
            requested: dynamic,
            no_children,
            frames: vec![Frame::new(root, dynamic, no_children)],
        }
    }
    pub fn root(&self) -> ObjectId {
        self.root
    }
    pub fn request(&self) -> (ObjectId, bool, bool) {
        (self.root, self.requested, self.no_children)
    }
    pub fn complete(&self) -> bool {
        self.frames.is_empty()
    }
    pub fn pending(&self) -> Option<ObjectId> {
        self.frames
            .last()
            .filter(|f| f.phase == Phase::Invalidate)
            .map(|f| f.id)
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut ancestors = std::collections::BTreeSet::new();
        if self.root <= 0
            || self.frames.first().is_some_and(|f| f.id != self.root)
            || self
                .frames
                .first()
                .is_some_and(|f| f.no_children != self.no_children)
            || self.frames.first().is_some_and(|f| {
                (self.requested && !f.dynamic)
                    || (f.phase == Phase::Start && f.dynamic != self.requested)
            })
            || self.frames.iter().enumerate().any(|(n, f)| {
                f.id <= 0
                    || !ancestors.insert(f.id)
                    || f.index > f.end
                    || f.end > i16::MAX as usize
                    || (f.phase != Phase::Children && (f.index != 0 || f.end != 0))
                    || (n > 0 && (f.no_children || self.frames[n - 1].dynamic && !f.dynamic))
                    || (n + 1 < self.frames.len() && (f.phase != Phase::Children || f.index == 0))
            })
        {
            return Err("invalid native draw update continuation".into());
        }
        Ok(())
    }
    /// Returns the object whose rectangle must be invalidated. State at that
    /// boundary still contains the old dynamic flag. Polling repeats no work.
    pub fn advance(&mut self, view: &mut impl DrawView) -> Result<Option<ObjectId>, String> {
        self.validate()?;
        loop {
            let Some(f) = self.frames.last_mut() else {
                return Ok(None);
            };
            match f.phase {
                Phase::Start => {
                    if forcing_child(view, f.id)? {
                        f.dynamic = true;
                    }
                    let draw = view.draw(f.id)?;
                    f.phase = if draw.dynamic != f.dynamic && draw.level <= view.visible_level() {
                        Phase::Invalidate
                    } else {
                        Phase::Stamp
                    };
                }
                Phase::Invalidate => return Ok(Some(f.id)),
                Phase::Store => {
                    let mut draw = view.draw(f.id)?;
                    draw.dynamic = f.dynamic;
                    view.write_draw(f.id, draw)?;
                    f.phase = Phase::Stamp;
                }
                Phase::Stamp => {
                    let mut draw = view.draw(f.id)?;
                    if draw.dynamic {
                        draw.expiry = i32::MAX;
                        draw.refreshed_at = view.clock();
                        view.write_draw(f.id, draw)?;
                    }
                    if f.no_children {
                        self.frames.pop();
                        continue;
                    }
                    // Capture after invalidation; child callbacks can change
                    // live members but cannot extend this frame's loop end.
                    f.end = view.slots(f.id)?.len();
                    f.phase = Phase::Children;
                }
                Phase::Children => {
                    if f.index == f.end {
                        self.frames.pop();
                        continue;
                    }
                    let child = view.slots(f.id)?.get(f.index).copied().flatten();
                    f.index += 1;
                    if let Some(id) = child.filter(|id| view.exists(*id)) {
                        let dynamic = f.dynamic;
                        if self.frames.iter().any(|a| a.id == id) {
                            return Err("cyclic native draw containment".into());
                        }
                        self.frames.push(Frame::new(id, dynamic, false));
                    }
                }
            }
        }
    }
    pub fn acknowledge(&mut self, id: ObjectId) -> Result<(), String> {
        self.validate()?;
        if self.pending() != Some(id) {
            return Err("stale native draw invalidation".into());
        }
        self.frames.last_mut().unwrap().phase = Phase::Store;
        Ok(())
    }
}

/// These are native vector memberships, not a replacement rendering order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lists {
    /// Viewer +1c8.
    pub dynamic: Vec<ObjectId>,
    /// Viewer +1d4; virtual +28 true, definition flag 2 clear.
    pub ordinary: Vec<ObjectId>,
    /// Viewer +1e0; virtual +28 true, definition flag 2 set.
    pub special: Vec<ObjectId>,
}
pub trait ClassificationView: DrawView {
    /// 440349's current cached order. The caller must service a stale cache.
    fn order(&self) -> &[ObjectId];
    fn word34(&self, id: ObjectId) -> Result<i16, String>;
    fn graphic_ready(&self, id: ObjectId) -> Result<bool, String>;
    /// 457588: resource +60's OBJD word 10 != 0 or word 4 != 0.
    fn has_drawings(&self, id: ObjectId) -> Result<bool, String>;
    /// Physical capture43265e can normalize a zero-slot predecessor.
    fn parent(&mut self, id: ObjectId) -> Result<Option<ObjectId>, String>;
    fn duration(&self, id: ObjectId) -> Result<i32, String>;
    fn outside_interior(&self, id: ObjectId) -> Result<bool, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dynamic {
    pub object: ObjectId,
    pub enabled: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Next,
    Dynamic,
    Skip,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classify {
    stage: Stage,
    index: usize,
    pending: Option<Dynamic>,
}
impl Default for Classify {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            index: 0,
            pending: None,
        }
    }
}
impl Classify {
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn pending(&self) -> Option<Dynamic> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.index > i16::MAX as usize
            || (self.stage == Stage::Start && self.index != 0)
            || (self.stage == Stage::Dynamic) != self.pending.is_some()
            || self.pending.is_some_and(|r| r.object <= 0)
        {
            return Err("invalid native presenter classification".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        view: &mut impl ClassificationView,
        lists: &mut Lists,
    ) -> Result<Option<Dynamic>, String> {
        self.validate()?;
        loop {
            match self.stage {
                Stage::Start => {
                    *lists = Lists::default();
                    self.stage = Stage::Next;
                }
                Stage::Next => {
                    if self.index > view.order().len() {
                        return Err("presenter callback invalidated native order cursor".into());
                    }
                    let Some(&id) = view.order().get(self.index) else {
                        self.stage = Stage::Done;
                        continue;
                    };
                    if id <= 0 {
                        return Err("null native presenter object".into());
                    }
                    self.stage = Stage::Skip;
                    if view.word34(id)? != 0 {
                        continue;
                    }
                    if view.graphic_ready(id)? {
                        if view.definition_flags(id)? & 2 != 0 {
                            lists.special.push(id);
                        } else {
                            lists.ordinary.push(id);
                        }
                    }
                    let child = forcing_child(view, id)?;
                    let own = view.definition_flags(id)? & 1 != 0 || view.force_dynamic();
                    // Preserve both physical captures when the first branch
                    // resolves a parent and the child branch also runs.
                    if (own && view.has_drawings(id)? && view.parent(id)?.is_none())
                        || (child && view.parent(id)?.is_none())
                    {
                        self.pending = Some(Dynamic {
                            object: id,
                            enabled: true,
                        });
                        self.stage = Stage::Dynamic;
                        continue;
                    }
                    if view.word34(id)? != 0
                        || !view.has_drawings(id)?
                        || view.parent(id)?.is_some()
                        || !view.draw(id)?.dynamic
                        || view.definition_flags(id)? & 1 != 0
                        || view.force_dynamic()
                        || child
                    {
                        continue;
                    }
                    // Signed x86 subtraction is captured before virtual +18.
                    let elapsed = view.clock().wrapping_sub(view.draw(id)?.refreshed_at) as i32;
                    if elapsed > view.duration(id)? && view.draw(id)?.expiry != i32::MAX {
                        self.pending = Some(Dynamic {
                            object: id,
                            enabled: false,
                        });
                        self.stage = Stage::Dynamic;
                    } else if !view.outside_interior(id)? {
                        lists.dynamic.push(id);
                    }
                }
                Stage::Dynamic => return Ok(self.pending),
                Stage::Skip => {
                    self.index += 1;
                    self.stage = Stage::Next;
                }
                Stage::Done => return Ok(None),
            }
        }
    }
    pub fn acknowledge(&mut self, request: Dynamic, lists: &mut Lists) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("stale presenter dynamic callback".into());
        }
        if request.enabled {
            lists.dynamic.push(request.object);
        }
        self.pending = None;
        self.stage = Stage::Skip;
        Ok(())
    }
}

/// Owns 4aacb9's recursive434657 call until it returns. Only rectangle
/// invalidation escapes; the classification cursor cannot overtake that child.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    classify: Classify,
    draw: Option<DrawUpdate>,
}
impl Classification {
    pub fn pending(&self) -> Option<ObjectId> {
        self.draw.as_ref().and_then(DrawUpdate::pending)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.classify.validate()?;
        if let Some(draw) = &self.draw {
            draw.validate()?;
            let request = self
                .classify
                .pending()
                .ok_or("orphan presenter draw update")?;
            if draw.complete() || draw.request() != (request.object, request.enabled, false) {
                return Err("presenter draw update belongs to another request".into());
            }
        } else if self.classify.pending().is_some() {
            return Err("presenter draw update absent".into());
        }
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.classify.complete() && self.draw.is_none()
    }
    pub fn advance(
        &mut self,
        view: &mut impl ClassificationView,
        lists: &mut Lists,
    ) -> Result<Option<ObjectId>, String> {
        self.validate()?;
        loop {
            if let Some(draw) = &mut self.draw {
                if let Some(id) = draw.advance(view)? {
                    return Ok(Some(id));
                }
                self.classify
                    .acknowledge(self.classify.pending().unwrap(), lists)?;
                self.draw = None;
            }
            let Some(request) = self.classify.advance(view, lists)? else {
                return Ok(None);
            };
            self.draw = Some(DrawUpdate::new(request.object, request.enabled, false));
        }
    }
    pub fn acknowledge(&mut self, id: ObjectId) -> Result<(), String> {
        self.validate()?;
        self.draw
            .as_mut()
            .ok_or("presenter invalidation absent")?
            .acknowledge(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Deserialize)]
    struct Input {
        rotation: u32,
        size: i32,
        tiles: Vec<[i8; 3]>,
        order: Vec<ObjectId>,
        cached: Vec<ObjectId>,
        dirty: u8,
        cached_rotation: u32,
        #[serde(default)]
        heap: bool,
    }
    impl OrderView for Input {
        fn rotation(&self) -> u32 {
            self.rotation
        }
        fn size(&self) -> i32 {
            self.size
        }
        fn registered(&self) -> &[ObjectId] {
            &self.order
        }
        fn cached_tile(&self, id: ObjectId) -> Result<[i8; 3], String> {
            Ok(self.tiles[id as usize - 1])
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_cached_order_rotation_partition_ties_and_heap() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/presenter-order.json")).unwrap();
        assert_eq!(
            report["capture_sha256"],
            "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
        );
        assert_eq!(report["cases"].as_array().unwrap().len(), 288);
        for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
            let input: Input = serde_json::from_value(row["input"].clone()).unwrap();
            let mut cache = Order {
                dirty: input.dirty != 0,
                rotation: input.cached_rotation,
                objects: input.cached.clone(),
            };
            if input.heap {
                let mut ids: Vec<_> = cache.objects.iter().map(|v| i32::from(*v)).collect();
                crate::nearby_houses::sort_with_depth(&mut ids, 0, &|a, b| {
                    draw_key(input.tiles[a as usize - 1], input.rotation).unwrap()
                        < draw_key(input.tiles[b as usize - 1], input.rotation).unwrap()
                });
                cache.objects = ids.into_iter().map(|v| v as ObjectId).collect();
            } else {
                cache.get(&input).unwrap();
            }
            assert_eq!(
                serde_json::json!({"order":cache.objects,"dirty":cache.dirty,"rotation":cache.rotation}),
                row["original"],
                "case {n}"
            );
            let saved = serde_json::to_vec(&cache).unwrap();
            let restored: Order = serde_json::from_slice(&saved).unwrap();
            restored.validate().unwrap();
            assert_eq!(cache, restored);
        }
    }
    #[test]
    fn valid_cache_does_not_read_registry_or_tiles() {
        struct Unavailable;
        impl OrderView for Unavailable {
            fn rotation(&self) -> u32 {
                2
            }
            fn size(&self) -> i32 {
                panic!("unexpected size read")
            }
            fn registered(&self) -> &[ObjectId] {
                panic!("unexpected registry read")
            }
            fn cached_tile(&self, _: ObjectId) -> Result<[i8; 3], String> {
                panic!("unexpected tile read")
            }
        }
        let mut cache = Order {
            dirty: false,
            rotation: 2,
            objects: vec![3, 1],
        };
        assert_eq!(cache.get(&Unavailable).unwrap(), [3, 1]);
    }
}
