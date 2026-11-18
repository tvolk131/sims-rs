//! Where a person's head looks: the head fields of the animator (+0x210
//! to +0x224) and the arithmetic that keeps them (0x590768, 0x5904d2).
//!
//! A script can make a person look at something (Look Towards sets person
//! data words 41 to 45). While bit 0 of word 42 is set, each animator tick
//! (and each draw of the person) measures two angles and turns the head a
//! little towards the target:
//!
//! - The head's heading ([`Head::heading`]): the angle, seen from above,
//!   from the point on top of the head (the HEAD bone's "Top" anchor) to
//!   a point 100 units in front of it (the "HeadFake" anchor), in eighths
//!   of a turn. It is measured with the head already turned, so it moves
//!   as the head turns.
//! - The target's heading ([`Head::target_heading`]): the angle from the
//!   top of the head to the target.
//!
//! The turn ([`Head::turn`]) moves by at most 0.2 eighths per tick towards
//! the difference, up to 1.5 eighths either way ([`Head::follow`]). With
//! bit 0 clear the turn shrinks by a fifth each tick ([`Head::relax`]).
//! The turn is applied to the HEAD bone only for the measurement and the
//! draw, and taken off again afterwards (0x590b27), so the pose the clips
//! give is never changed by it ([`turned`]).
//!
//! The game runs the x87 FPU at 24-bit precision, so every add, subtract,
//! multiply, divide and square root rounds to single precision
//! ([`Ext::single`]). The arc tangent, sine and cosine are not rounded by
//! that control; they come from libm in double precision
//! ([`Ext::atan2`], [`Ext::cos`], [`Ext::sin`]), which is **unverified**
//! at rounding boundaries. Each function below follows the original's
//! order of operations, instruction by instruction.
//!
//! How the original places a point of a bone in the lot: the bone's
//! matrix (0x595f32) is its rotation, as a matrix, followed by its
//! position; an anchor (0x5a1631) is an offset from the bone; and the
//! person's model matrix (0x592132) turns the skeleton to the person's
//! direction and moves it to the person's position. Matrices are 4 by 4,
//! row-vector style (a point is a row on the left), and multiplied by
//! 0x462af3. Only the last row of a product is needed here, which comes
//! from the last row of the left matrix alone ([`transform`]).

use crate::pose::{self, Bone};
use crate::{FULL_SIZE, World};
use serde::{Deserialize, Serialize};
use sims_core::{Ext, F32};

/// The head fields of a person's animator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    /// +0x210: the head's heading, in eighths of a turn from 0 to 8.
    pub heading: F32,
    /// +0x214: the target's heading, in eighths of a turn from 0 to 8.
    pub target_heading: F32,
    /// +0x218: how far the head is turned, in eighths of a turn, from
    /// -1.5 to 1.5.
    pub turn: F32,
    /// +0x21c, +0x220, +0x224: where the target is in the lot, or None
    /// while nothing has written it. The load restores the saved point
    /// (0x5918ba); a person made in play has the constructor's, which
    /// leaves these words as the memory held them, so reading them before
    /// a target has written them fails.
    pub target: Option<[F32; 3]>,
}

impl Head {
    /// The head fields after the load: 0 for the angles and the turn, and
    /// no target. In a capture of the original loading the Goth house,
    /// persons 16, 18 and 259 hold 0 in the three angles after the first
    /// lot update.
    pub const LOADED: Self = Self {
        heading: F32::ZERO,
        target_heading: F32::ZERO,
        turn: F32::ZERO,
        target: None,
    };

    /// One step towards the target (0x5907c5), with bit 0 of word 42 set:
    /// the difference between the two headings, put into -4 to 4, is
    /// added to the turn, at most 0.2 either way. A turn past 1.5 either
    /// way stops at 1.5 and returns true: the head can turn no further,
    /// and the caller changes word 42 (to 5 when word 44 is 0, to 4 when
    /// it is 1).
    pub fn follow(&mut self) -> bool {
        let mut d = s(self.heading.ext() - self.target_heading.ext());
        let (four, eight) = (Ext::from_i32(4), Ext::from_i32(8));
        while d < -four {
            d = s(d + eight);
        }
        while d >= four {
            d = s(d - eight);
        }
        if d > STEP.ext() {
            d = STEP.ext();
        } else if d < -STEP.ext() {
            d = -STEP.ext();
        }
        let turn = s(d + self.turn.ext());
        self.turn = turn.to_f32();
        if turn < -LIMIT.ext() {
            self.turn = (-LIMIT.ext()).to_f32();
            true
        } else if turn > LIMIT.ext() {
            self.turn = LIMIT;
            true
        } else {
            false
        }
    }

    /// With bit 0 of word 42 clear (0x590886): the turn times 0.8.
    ///
    /// The code reads 0.8 as a double (`fmul qword [0x672030]`), but the
    /// original's runs follow the product with 0.8 as a float, rounded
    /// once: in every one of the five steps of the fifth capture where the
    /// two differ (person 21, ticks 814 to 833), and no step anywhere
    /// follows the double. Why is **unknown**; the captures run the game
    /// under an emulator, whose x87 may round in its own way. The product
    /// of two floats is exact here, so it is rounded once.
    pub fn relax(&mut self) {
        self.turn = s(self.turn.ext() * RELAX.ext()).to_f32();
    }

    /// The end of the step (0x5908a0): a turn of less than 0.01 either way
    /// becomes 0 and returns false; any other turn returns true, and the
    /// head is turned by it ([`turned`]) until the measurement is done.
    pub fn settle(&mut self) -> bool {
        let turn = self.turn.ext();
        if turn < SETTLED.ext() && turn > -SETTLED.ext() {
            self.turn = F32::ZERO;
            false
        } else {
            true
        }
    }
}

/// How far the turn moves per step, 0.2 (0x666a28, 0x67203c).
const STEP: F32 = F32::from_bits(0x3e4c_cccd);
/// The largest turn, 1.5 (0x668654, 0x672038; 0x3fc00000 and 0xbfc00000
/// are written).
const LIMIT: F32 = F32::from_bits(0x3fc0_0000);
/// 0.8, as the original's runs multiply by it (see [`Head::relax`]; the
/// code holds it as a double at 0x672030).
const RELAX: F32 = F32::from_bits(0x3f4c_cccd);
/// Below this either way the turn is none, 0.01 (0x666a4c, 0x67202c).
const SETTLED: F32 = F32::from_bits(0x3c23_d70a);
/// Pi as the original's floats hold it (0x671ed8, 0x673438).
const PI: F32 = F32::from_bits(0x4049_0fdb);
/// How far the head's bone may be turned from straight, in radians, 1.2
/// (0x671ea0, 0x672028).
const REACH: F32 = F32::from_bits(0x3f99_999a);
/// One eighth (0x668220).
const EIGHTH: F32 = F32::from_bits(0x3e00_0000);
/// 1/180, as the placement's float holds it (0x66742c).
const DEGREE: F32 = F32::from_bits(0x3bb6_0b61);
/// A 16th, the size of a position step in tiles (0x667278).
const SIXTEENTH: F32 = F32::from_bits(0x3d80_0000);

fn s(value: Ext) -> Ext {
    value.single()
}

fn one() -> Ext {
    Ext::from_i32(1)
}

/// A 4 by 4 matrix, row by row.
pub type Matrix = [F32; 16];

/// The anchors a person's animator binds (0x5924a4, made by 0x5a1631):
/// a named point at an offset from a bone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// No name: the bone's own origin (the identity, 0x6fe720).
    Origin,
    Palm,
    Top,
    /// "HeadFake": 100 units in front of the top of the head.
    HeadFake,
    Shoulder,
}

impl Anchor {
    /// The anchor of a name, matched exactly; an empty name is the
    /// origin. Another name binds no anchor.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "" => Some(Self::Origin),
            "Palm" => Some(Self::Palm),
            "Top" => Some(Self::Top),
            "HeadFake" => Some(Self::HeadFake),
            "Shoulder" => Some(Self::Shoulder),
            _ => None,
        }
    }

    /// The anchor's offset from its bone (0x5a1631). The anchor also turns
    /// about the third axis, which changes where its own axes point but
    /// not where it is, and so is left out.
    pub fn offset(self) -> [F32; 3] {
        let (z, point_eight, half, hundred, unit) = (
            F32::ZERO,
            F32::from_bits(0x3f4c_cccd),
            F32::from_bits(0x3f00_0000),
            F32::from_bits(0x42c8_0000),
            F32::from_bits(0x3f80_0000),
        );
        match self {
            Self::Origin => [z, z, z],
            Self::Palm => [half, z, z],
            Self::Top => [point_eight, z, z],
            Self::HeadFake => [point_eight, z, hundred],
            Self::Shoulder => [z, z, unit],
        }
    }
}

/// A rotation as a matrix (0x59601b), the quaternion x, y, z, w taken as
/// it is: no scaling to length 1.
pub fn rotation(q: [F32; 4]) -> Matrix {
    let [x, y, z, w] = q.map(F32::ext);
    let two = Ext::from_i32(2);
    let xx = s(s(x * x) * two);
    let y2 = s(y * two);
    let yy = s(y2 * y);
    let z2 = s(z * two);
    let zz = s(z2 * z);
    let xy = s(y2 * x);
    let xz = s(z2 * x);
    let w2 = s(w * two);
    let wx = s(x * w2);
    let yz = s(z2 * y);
    let wy = s(w2 * y);
    let wz = s(w2 * z);
    let o = Ext::ZERO;
    [
        s(s(one() - yy) - zz),
        s(xy - wz),
        s(wy + xz),
        o,
        s(wz + xy),
        s(s(one() - xx) - zz),
        s(yz - wx),
        o,
        s(xz - wy),
        s(yz + wx),
        s(s(one() - xx) - yy),
        o,
        o,
        o,
        o,
        one(),
    ]
    .map(Ext::to_f32)
}

/// A bone's matrix (0x595f32): the rotation of the opposite turn (x, y
/// and z negated) as a matrix, followed by the bone's position. The
/// original multiplies the two (0x49b87c); the rotation's last column and
/// row are those of the identity, so the product is the rotation's rows
/// with the position as the last row, exactly.
pub fn bone_matrix(bone: Bone) -> Matrix {
    let [x, y, z, w] = bone.rotation;
    let mut m = rotation([
        (-x.ext()).to_f32(),
        (-y.ext()).to_f32(),
        (-z.ext()).to_f32(),
        w,
    ]);
    m[12..15].copy_from_slice(&bone.translation);
    m
}

/// The last row of the product of a matrix whose last row is `row` with
/// `m` (0x462af3, which works the entries of that row out in this order).
pub fn transform(row: [F32; 4], m: &Matrix) -> [F32; 4] {
    let [a0, a1, a2, a3] = row.map(F32::ext);
    let b = m.map(F32::ext);
    let column = |i: usize| {
        if i == 0 {
            s(s(s(s(a3 * b[12]) + s(a2 * b[8])) + s(a1 * b[4])) + s(a0 * b[0]))
        } else {
            s(s(s(s(a3 * b[12 + i]) + s(a1 * b[4 + i])) + s(a2 * b[8 + i])) + s(a0 * b[i]))
        }
    };
    [column(0), column(1), column(2), column(3)].map(Ext::to_f32)
}

/// Where an anchor of a posed bone is in the lot (0x59640d, 0x59638f):
/// the anchor's matrix times the bone's times the model's, of which the
/// position is the last row. The anchor's last row is its offset and 1
/// (0x5a1631 multiplies a turn by the offset; the turn's last row is the
/// identity's).
pub fn anchor_point(bone: Bone, anchor: Anchor, model: &Matrix) -> [F32; 3] {
    let [x, y, z] = anchor.offset();
    let unit = F32::from_bits(0x3f80_0000);
    let on_bone = transform([x, y, z, unit], &bone_matrix(bone));
    let [px, py, pz, _] = transform(on_bone, model);
    [px, py, pz]
}

/// The model matrix of a person placed on the ground level (0x592132):
/// turned about the second axis by its direction, then moved to its
/// position. `position` is the animator's (+0x80, +0x84 and +0x88, the
/// direction in eighths of a turn) and `height` the terrain's height
/// there ([`terrain_height`]).
///
/// The original adds to the position three more words (+0x8c to +0x94),
/// which the start of a route sets to 0 and which hold 0 for every person
/// in the capture; they are taken as 0. On another level it adds an
/// offset from the view's table ([`placement_on_level`]).
/// In the lot the first coordinate is 3/16 of the position's second and
/// the third 3/16 of its first.
pub fn placement(position: [F32; 3], height: F32) -> Matrix {
    let [p0, p1, direction] = position.map(F32::ext);
    let three = Ext::from_i32(3);
    let k = s(three * SIXTEENTH.ext());
    let x = s(k * p1);
    let z = s(k * p0);
    let angle =
        s(s(PI.ext() * s(s(direction + Ext::from_i32(6)) * Ext::from_i32(45))) * DEGREE.ext())
            .to_f32();
    let c = Ext::cos(angle).to_f32();
    let sine = Ext::sin(angle).to_f32();
    let minus = (-sine.ext()).to_f32();
    let (o, i) = (F32::ZERO, F32::from_bits(0x3f80_0000));
    [
        c,
        o,
        minus,
        o,
        o,
        i,
        o,
        o,
        sine,
        o,
        c,
        o,
        x.to_f32(),
        height,
        z.to_f32(),
        i,
    ]
}

/// What each level above the ground adds to a person's model position
/// (0x5922ad): the view's table holds one vector per zoom (+0xb8 of the
/// house view, less 12 bytes a zoom step), and the view's setup fills
/// every one with the same, a half, 9.300511 and a half (0x4ca054). The
/// view object is not in the frozen image; its class is at 0x4a6b15
/// (vtable 0x668744: +0x14 reads the zoom 0x6f00d8, +0x18 the rotation
/// 0x6f00dc, +0x24 is +0xb8).
const LEVEL_RISE: [F32; 3] = [
    F32::from_bits(0x3f00_0000),
    F32::from_bits(0x4114_cee5),
    F32::from_bits(0x3f00_0000),
];

/// The model matrix of a person on `level` (0x592132), with the view
/// turned to `view_rotation` (0x6f00dc, which every turn of the view
/// sets with the scene's +0x84: 0x4a6d58, 0x4a7f28, 0x4a87d7). On the
/// ground level it is [`placement`]'s.
///
/// Above it the original adds to the position the level's rise: the
/// levels above the ground, as a float, times each of [`LEVEL_RISE`]'s
/// three (0x5922d9 to 0x592315), turned about the second axis by minus
/// the rotation times pi times a half (0x592318, 0x5c3a20, 0x5c39a0). At
/// rotation 0 the turn is the identity and leaves the rise as it is; any
/// other turn is not ported, and gives `None`. The first coordinate is
/// then the rise's plus 3/16 of the position's second, unrounded
/// (0x59237b); the height the rise's plus the terrain's (0x592385); the
/// third the rise's plus 3/16 of the position's first, rounded to a float
/// first (0x592375, 0x59238e).
pub fn placement_on_level(
    position: [F32; 3],
    height: F32,
    level: i8,
    view_rotation: u8,
) -> Option<Matrix> {
    let mut matrix = placement(position, height);
    if level == 1 {
        return Some(matrix);
    }
    if view_rotation & 3 != 0 {
        return None;
    }
    let above = Ext::from_i32(i32::from(level) - 1);
    let [rx, ry, rz] = LEVEL_RISE.map(|r| s(above * r.ext()));
    let [p0, p1] = [position[0].ext(), position[1].ext()];
    let k = s(Ext::from_i32(3) * SIXTEENTH.ext());
    matrix[12] = s(rx + k * p1).to_f32();
    matrix[13] = s(ry + height.ext()).to_f32();
    matrix[14] = s(rz + s(k * p0)).to_f32();
    Some(matrix)
}

/// The lot's altitude array: one byte per tile corner, four to a tile.
const ALTITUDE: u16 = 0;

/// Why a person cannot be placed by [`ground_height`] or [`model`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unplaced {
    /// No object with the person's ID.
    Missing,
    /// The person is on a level other than the ground (1). Above it the
    /// original adds an offset from the view's table (0x5922ad), which is
    /// not ported; level 0 and below are refused too.
    Level(i8),
    /// The rounded position falls off the lot (0x45ee1d).
    OffLot,
    /// The lot has no altitude array of the usual shape.
    NoAltitude,
}

/// The terrain's height under a person's position, in the model matrix's
/// units ([`placement`]'s `height`), as the original finds it for the
/// model matrix (0x592132, 0x592242, 0x478dfe, 0x59226f). `position` is
/// the animator's first two words, in sixteenths of a tile.
///
/// Each coordinate is rounded by adding a half and truncating, in single
/// precision (NaN gives 0, as the original's conversion does); the tile is each rounded coordinate divided by 16, cut to a
/// signed byte. The tile's cell is at the first coordinate's tile times 64
/// plus the second's, and [`altitude_at`] takes the second coordinate's
/// offset first. The same lookup as `sims_vm`'s head model, whose
/// captures pin the order (ticks 334 and 356 on, persons 16 and 259).
/// Only reads the world.
pub fn ground_height(world: &World, position: [F32; 2]) -> Result<F32, Unplaced> {
    let half = Ext::from_f64_bits(0x3fe0_0000_0000_0000);
    let round = |v: F32| (v.ext() + half).single().to_i32_truncate();
    let point = position.map(round);
    let full = FULL_SIZE as usize;
    // The arrays are FULL_SIZE on a side whatever the lot's size.
    let size = i32::from(world.size).min(FULL_SIZE.into());
    let [first, second] = point.map(|p| (p >> 4) as i8 as i32);
    if !(0..size).contains(&first) || !(0..size).contains(&second) {
        return Err(Unplaced::OffLot);
    }
    let altitude = world
        .arrays
        .get(&ALTITUDE)
        .filter(|a| a.stride == 4 && a.data.len() == full * full * 4)
        .ok_or(Unplaced::NoAltitude)?;
    let at = (first as usize * full + second as usize) * 4;
    let corners = [0, 1, 2, 3].map(|i| altitude.data[at + i]);
    Ok(terrain_height(altitude_at(
        corners,
        point[1] & 15,
        point[0] & 15,
    )))
}

/// Person `id`'s model matrix (0x592132) with the animator at `position`
/// (+0x80 to +0x88: the two coordinates and the direction): turned to the
/// direction and standing on the terrain there ([`ground_height`],
/// [`placement`]). Fails for a person on any level but the ground: the
/// upper levels' offset is not ported. Only reads the world.
pub fn model(world: &World, id: u16, position: [F32; 3]) -> Result<Matrix, Unplaced> {
    let object = world.objects.get(&id).ok_or(Unplaced::Missing)?;
    let level = World::tile(object.position)[2];
    if level != 1 {
        return Err(Unplaced::Level(level));
    }
    let height = ground_height(world, [position[0], position[1]])?;
    Ok(placement(position, height))
}

/// The terrain's altitude, in the terrain array's units, at a point of a
/// tile (0x478dfe). `corners` is the tile's cell of the altitude array, in
/// the order of its bytes; `first` and `second` are the offsets into the
/// tile, from 0 to 15, of the point's first and second words as the
/// original passes it (a person's placement passes its second coordinate
/// first).
///
/// Byte 1 is the corner at offsets (0, 0), byte 0 the one at (16, 0),
/// byte 2 the one at (0, 16) and byte 3 the one at (16, 16). The tile is
/// split along its diagonal into two flat triangles. On the diagonal the
/// altitude is byte 1 plus the climb to byte 3; on either side it goes
/// from the edge to the diagonal along a line of constant offset. Every
/// step is rounded to single precision, as the x87 runs at 24 bits. The
/// original also returns byte 1 for an offset below 0 or above 16, which
/// cannot happen for offsets taken modulo 16. The same as the old
/// prototype's `Terrain::height`
/// (legacy/crates/sims-sim/src/pose/transform.rs).
pub fn altitude_at(corners: [u8; 4], first: i32, second: i32) -> F32 {
    let [b0, b1, b2, b3] = corners.map(i32::from);
    let base = Ext::from_i32(b1);
    let (f0, f1) = (Ext::from_i32(first), Ext::from_i32(second));
    let k = SIXTEENTH.ext();
    let climb = |to: i32, offset: Ext| s(s(Ext::from_i32(to - b1) * offset) * k);
    let value = if first == second {
        s(climb(b3, f1) + base)
    } else if second > first {
        let along = climb(b2, f1);
        let across = climb(b3, f1);
        s(s(s(s(s(across - along) * f0) / f1) + along) + base)
    } else {
        let along = climb(b0, f0);
        let across = climb(b3, f0);
        s(s(s(s(s(across - along) * f1) / f0) + along) + base)
    };
    value.to_f32()
}

/// The height of the terrain at an altitude ([`altitude_at`]), as the
/// placement scales it (0x59226f): the altitude times three times
/// 1/(2 sqrt 6).
pub fn terrain_height(altitude: F32) -> F32 {
    let root = s(Ext::from_i32(6).sqrt());
    let scale = s(one() / s(root + root));
    s(s(scale * Ext::from_i32(3)) * altitude.ext()).to_f32()
}

/// The heading from a point to another (0x5904d2), seen from above, in
/// eighths of a turn from 0 to 8, given the differences of their first
/// (`dx`) and third (`dz`) coordinates; None when both are 0, in which
/// case the original keeps the heading it had.
pub fn heading(dx: F32, dz: F32) -> Option<F32> {
    let (x, z) = (dx.ext(), dz.ext());
    if s(s(z * z) + s(x * x)) <= Ext::ZERO {
        return None;
    }
    let angle = Ext::atan2(dz, (-x).to_f32());
    let eight = Ext::from_i32(8);
    let mut v = s(s(angle / s(PI.ext() + PI.ext())) * eight);
    while v < Ext::ZERO {
        v = s(v + eight);
    }
    while v >= eight {
        v = s(v - eight);
    }
    Some(v.to_f32())
}

/// The difference of two coordinates, as the measurement keeps it.
pub fn difference(a: F32, b: F32) -> F32 {
    s(a.ext() - b.ext()).to_f32()
}

/// The head bone's rotation, relative to its parent, turned by `turn`
/// (0x5908cf to 0x590a3c). The rotation is scaled to length 1 (0x5c4a20;
/// length 0 gives no turn). The head's present angle about the first
/// axis is read from the rotation's matrix (0x5c4a50); the turn, a
/// quarter of a turn per 2 eighths, is cut so that angle and turn
/// together stay within 1.2 radians either way. The turn becomes a
/// rotation about the first axis (0x5c4bb0, with sine and cosine of half
/// the angle) which is multiplied with the scaled rotation (0x590a7f).
pub fn turned(rotation_q: [F32; 4], turn: F32) -> [F32; 4] {
    let mut add = s(s(PI.ext() + PI.ext()) * s(turn.ext() * EIGHTH.ext()));
    let [x, y, z, w] = rotation_q.map(F32::ext);
    let length = s(s(s(s(s(x * x) + s(y * y)) + s(z * z)) + s(w * w)).sqrt());
    let q = if length == Ext::ZERO {
        pose::ORIGIN.rotation
    } else {
        let r = s(one() / length);
        [x, y, z, w].map(|v| s(v * r).to_f32())
    };
    let [x, y, z, w] = q.map(F32::ext);
    let two = Ext::from_i32(2);
    let xx = s(s(x * x) * two);
    let zz = s(s(z * z) * two);
    let yz = s(s(z * y) * two);
    let wx = s(s(w * x) * two);
    let m5 = s(s(one() - xx) - zz).to_f32();
    let m6 = s(yz - wx);
    let angle = Ext::atan2((-m6).to_f32(), m5);
    let total = s(angle + add);
    if total < -REACH.ext() {
        add = s(-REACH.ext() - angle);
    } else if total > REACH.ext() {
        add = s(REACH.ext() - angle);
    }
    let half = s(add * Ext::from_f64_bits(0x3fe0_0000_0000_0000)).to_f32();
    let r = [
        Ext::sin(half).to_f32(),
        F32::ZERO,
        F32::ZERO,
        Ext::cos(half).to_f32(),
    ];
    multiply(r, q)
}

/// The product of two rotations (0x590a7f, with `r` the object it is
/// called on and `q` its argument), in the original's order.
pub fn multiply(r: [F32; 4], q: [F32; 4]) -> [F32; 4] {
    let [rx, ry, rz, rw] = r.map(F32::ext);
    let [qx, qy, qz, qw] = q.map(F32::ext);
    let w = s(s(s(s(rw * qw) - s(qx * rx)) - s(ry * qy)) - s(rz * qz));
    let z = s(s(s(s(rx * qy) + s(rw * qz)) - s(ry * qx)) + s(rz * qw));
    let y = s(s(s(s(rw * qy) - s(qz * rx)) + s(ry * qw)) + s(rz * qx));
    let x = s(s(s(s(qw * rx) + s(rw * qx)) + s(ry * qz)) - s(rz * qy));
    [x, y, z, w].map(Ext::to_f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lot whose every tile has altitude corners `corners`, with person
    /// 7 standing at `position` (sixteenths of a tile, and the level).
    fn lot(corners: [u8; 4], position: [i32; 3]) -> World {
        let mut world = World::empty();
        let full = FULL_SIZE as usize;
        world.arrays.insert(
            ALTITUDE,
            sims_formats::lot::Array {
                width: full,
                height: full,
                stride: 4,
                encoding: 0,
                data: corners.repeat(full * full),
            },
        );
        world.objects.insert(
            7,
            crate::Object {
                id: 7,
                guid: 0,
                footprint: [0; 4],
                position,
                unknown_short: 0,
                attributes: Vec::new(),
                temps: Vec::new(),
                data: Vec::new(),
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: Vec::new(),
                relationship_flag: 0,
                relationships: Vec::new(),
                slots: Vec::new(),
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: None,
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            },
        );
        world
    }

    #[test]
    fn a_person_stands_on_the_terrain_under_their_rounded_position() {
        // Corners: byte 1 at offsets (0, 0) is 2, byte 0 at (16, 0) is 10,
        // byte 2 at (0, 16) is 6, byte 3 at (16, 16) is 2.
        let world = lot([10, 2, 6, 2], [40 * 16, 20 * 16, 1]);
        let at = |p: [f32; 2]| ground_height(&world, p.map(F32::from_f32));
        // Position (12.4, 3.6) rounds to (12, 4): the second coordinate's
        // offset goes first.
        let expected = terrain_height(altitude_at([10, 2, 6, 2], 4, 12));
        assert_eq!(at([16.0 * 5.0 + 12.4, 16.0 * 9.0 + 3.6]), Ok(expected));
        assert_ne!(expected, terrain_height(altitude_at([10, 2, 6, 2], 12, 4)));
        // Half a sixteenth rounds up into the next tile, onto its corner.
        assert_eq!(
            at([15.5, 0.0]),
            Ok(terrain_height(altitude_at([10, 2, 6, 2], 0, 0)))
        );
        // Off the lot, beyond the arrays, or with no altitude array.
        assert_eq!(at([-20.0, 0.0]), Err(Unplaced::OffLot));
        assert_eq!(at([0.0, 64.0 * 16.0]), Err(Unplaced::OffLot));
        // NaN converts to 0, as the original's `__ftol` gives it.
        assert_eq!(
            at([f32::NAN, 0.0]),
            Ok(terrain_height(altitude_at([10, 2, 6, 2], 0, 0)))
        );
        let mut wide = world.clone();
        wide.size = FULL_SIZE + 1;
        assert_eq!(
            ground_height(&wide, [F32::from_f32(64.0 * 16.0), F32::ZERO]),
            Err(Unplaced::OffLot)
        );
        assert_eq!(
            ground_height(&World::empty(), [F32::ZERO; 2]),
            Err(Unplaced::NoAltitude)
        );
    }

    /// The cell is the first coordinate's tile times 64 plus the
    /// second's; the rounding is in single precision; a tile is cut to a
    /// signed byte.
    #[test]
    fn the_terrain_lookup_follows_the_originals_cell_rounding_and_wrap() {
        let mut world = lot([0, 0, 0, 0], [0, 0, 1]);
        let full = FULL_SIZE as usize;
        let cell = |world: &mut World, [a, b]: [usize; 2], corners: [u8; 4]| {
            let at = (a * full + b) * 4;
            world.arrays.get_mut(&ALTITUDE).unwrap().data[at..at + 4].copy_from_slice(&corners);
        };
        cell(&mut world, [5, 9], [4, 4, 4, 4]);
        cell(&mut world, [9, 5], [8, 8, 8, 8]);
        // Byte 1 is the corner at offsets (0, 0), byte 3 at (16, 16).
        cell(&mut world, [0, 0], [0, 0, 0, 16]);
        let at = |world: &World, p: [F32; 2]| ground_height(world, p).unwrap();
        let height = |altitude: f32| terrain_height(F32::from_f32(altitude));
        let tile = |a: f32, b: f32| [a * 16.0 + 8.0, b * 16.0 + 8.0].map(F32::from_f32);
        assert_eq!(at(&world, tile(5.0, 9.0)), height(4.0));
        assert_eq!(at(&world, tile(9.0, 5.0)), height(8.0));
        // Just under a half rounds up once added in single precision (it
        // would not in double): offsets (1, 1), one sixteenth of the climb.
        let under_half = F32::from_bits(0x3eff_ffff);
        assert_eq!(at(&world, [under_half; 2]), height(1.0));
        // 4096 is tile 256, which the signed byte makes tile 0.
        assert_eq!(at(&world, [4096.0, 0.0].map(F32::from_f32)), height(0.0));
        assert_eq!(
            at(&world, [4096.0 + 1.0, 1.0].map(F32::from_f32)),
            height(1.0)
        );
    }

    #[test]
    fn a_persons_model_matrix_stands_them_on_the_terrain() {
        let position = [100.0, 50.0, 3.0].map(F32::from_f32);
        let world = lot([3, 3, 3, 3], [100, 50, 1]);
        let height = terrain_height(F32::from_f32(3.0));
        assert_eq!(model(&world, 7, position), Ok(placement(position, height)));
        assert_eq!(model(&world, 8, position), Err(Unplaced::Missing));
        let upstairs = lot([3, 3, 3, 3], [100, 50, 2]);
        assert_eq!(model(&upstairs, 7, position), Err(Unplaced::Level(2)));
    }

    #[test]
    fn a_level_above_the_ground_raises_the_model_by_the_views_rise() {
        let position = [100.0, 50.0, 3.0].map(F32::from_f32);
        let height = terrain_height(F32::from_f32(3.0));
        let ground = placement(position, height);
        for rotation in 0..4 {
            assert_eq!(
                placement_on_level(position, height, 1, rotation),
                Some(ground)
            );
        }
        let at = |level| placement_on_level(position, height, level, 0).unwrap();
        let rise = 9.300_511_f32;
        // 3/16 of 50 is 9.375, of 100 18.75; the turn stays the ground's.
        let two = at(2);
        assert_eq!(two[..12], ground[..12]);
        assert_eq!(
            two[12..15],
            [f(0.5 + 9.375), f(rise + height.to_f32()), f(0.5 + 18.75)]
        );
        let three = at(3);
        assert_eq!(
            three[12..15],
            [
                f(1.0 + 9.375),
                f(2.0 * rise + height.to_f32()),
                f(1.0 + 18.75)
            ]
        );
        // A turned view turns the rise: not ported.
        assert_eq!(placement_on_level(position, height, 2, 1), None);
        assert_eq!(placement_on_level(position, height, 2, 7), None);
        assert_eq!(placement_on_level(position, height, 2, 4), Some(two));
    }

    fn f(v: f32) -> F32 {
        F32::from_f32(v)
    }

    fn head(heading: f32, target: f32, turn: f32) -> Head {
        Head {
            heading: f(heading),
            target_heading: f(target),
            turn: f(turn),
            target: None,
        }
    }

    #[test]
    fn the_turn_moves_at_most_a_fifth_of_an_eighth_towards_the_target() {
        let mut h = head(0.5, 1.5, 0.0);
        assert!(!h.follow());
        assert_eq!(h.turn, (-STEP.ext()).to_f32());
        // A small difference is taken whole.
        let mut h = head(1.0, 0.9, 0.0);
        assert!(!h.follow());
        assert_eq!(h.turn, s(f(1.0).ext() - f(0.9).ext()).to_f32());
        // The difference goes the shorter way round: 0.5 less 7.5 is 1.
        let mut h = head(0.5, 7.5, 0.0);
        h.follow();
        assert_eq!(h.turn, STEP);
    }

    #[test]
    fn the_turn_stops_at_one_and_a_half_eighths() {
        let mut h = head(0.0, 2.0, -1.4);
        assert!(h.follow());
        assert_eq!(h.turn.to_bits(), 0xbfc0_0000);
        let mut h = head(2.0, 0.0, 1.4);
        assert!(h.follow());
        assert_eq!(h.turn.to_bits(), 0x3fc0_0000);
    }

    #[test]
    fn a_turn_left_alone_shrinks_and_then_settles_at_none() {
        let mut h = head(0.0, 0.0, 0.0125);
        h.relax();
        // 0.0125 times the float 0.8 is one unit in the last place above
        // 0.01, which is not less than 0.01.
        assert_eq!(h.turn.to_bits(), 0x3c23_d70b);
        assert!(h.settle());
        h.relax();
        assert!(!h.settle());
        assert_eq!(h.turn, F32::ZERO);
    }

    #[test]
    fn headings_are_eighths_of_a_turn_from_the_first_axis_backwards() {
        // atan2(dz, -dx): straight along -x is 0, along +z a quarter.
        assert_eq!(heading(f(-1.0), f(0.0)), Some(F32::ZERO));
        assert_eq!(heading(f(0.0), f(1.0)), Some(f(2.0)));
        assert_eq!(heading(f(1.0), f(0.0)), Some(f(4.0)));
        assert_eq!(heading(f(0.0), f(-1.0)), Some(f(6.0)));
        assert_eq!(heading(F32::ZERO, F32::ZERO), None);
    }

    #[test]
    fn a_bone_matrix_is_the_opposite_turn_then_the_position() {
        // A quarter turn about the third axis: (0, 0, sin 45, cos 45).
        let h = f(core::f32::consts::FRAC_1_SQRT_2);
        let bone = Bone {
            translation: [f(1.0), f(2.0), f(3.0)],
            rotation: [F32::ZERO, F32::ZERO, h, h],
        };
        let m = bone_matrix(bone);
        let p = anchor_point(bone, Anchor::Palm, &{
            let mut identity = [F32::ZERO; 16];
            for i in 0..4 {
                identity[i * 5] = f(1.0);
            }
            identity
        });
        // The offset (0.5, 0, 0) turned a quarter turn onto the second
        // axis, plus the position.
        assert_eq!(m[12..15], [f(1.0), f(2.0), f(3.0)]);
        assert!((p[0].to_f32() - 1.0).abs() < 1e-6);
        assert!((p[1].to_f32() - 2.5).abs() < 1e-6);
        assert_eq!(p[2], f(3.0));
    }

    #[test]
    fn the_model_matrix_faces_the_direction_and_scales_the_position() {
        // Direction 2 (east, 90 degrees past the base of 6 eighths): the
        // angle is (2 + 6) * 45 degrees, a whole turn.
        let m = placement([f(16.0), f(32.0), f(2.0)], F32::ZERO);
        assert_eq!(m[12], f(6.0));
        assert_eq!(m[14], f(3.0));
        assert!((m[0].to_f32() - 1.0).abs() < 1e-6);
        assert_eq!(m[5], f(1.0));
    }

    #[test]
    fn a_turn_beyond_the_reach_is_cut_to_it() {
        // No present angle: a turn of 2 eighths (a quarter turn) is cut
        // to 1.2 radians about the first axis.
        let angle_of = |q: [F32; 4]| 2.0 * f64::from(Ext::atan2(q[0], q[3]).to_f32().to_f32());
        let q = turned(pose::ORIGIN.rotation, f(2.0));
        let angle = angle_of(q);
        assert!((angle - 1.2).abs() < 1e-6, "{angle}");
        assert_eq!((q[1], q[2]), (F32::ZERO, F32::ZERO));
        // A small turn is taken whole: 0.1 eighths is pi/40 radians.
        let q = turned(pose::ORIGIN.rotation, f(0.1));
        let angle = angle_of(q);
        assert!(
            (angle - core::f64::consts::PI / 40.0).abs() < 1e-6,
            "{angle}"
        );
    }

    #[test]
    fn the_altitude_is_flat_on_each_half_of_the_tile() {
        let at = |c, a, b| altitude_at(c, a, b).to_f32();
        // A level tile is its height everywhere.
        assert_eq!(at([3, 3, 3, 3], 5, 9), 3.0);
        // The corners: byte 1 at (0, 0); a step of 1 is a 16th of the climb.
        let c = [8, 0, 4, 16];
        assert_eq!(at(c, 0, 0), 0.0);
        // On the diagonal it climbs to byte 3.
        assert_eq!(at(c, 8, 8), 8.0);
        // Along the second coordinate's edge it climbs to byte 2, along
        // the first's to byte 0.
        assert_eq!(at(c, 0, 8), 2.0);
        assert_eq!(at(c, 8, 0), 4.0);
        // Inside a half it is on the plane through its three corners:
        // (0, 0) 0, (0, 16) 4 and (16, 16) 16 give 3/4 a + 1/4 b.
        assert_eq!(at(c, 4, 12), 3.0 * 4.0 / 4.0 + 12.0 / 4.0);
        // (0, 0) 0, (16, 0) 8 and (16, 16) 16 give a/2 + b/2.
        assert_eq!(at(c, 12, 4), 8.0);
        // A corner lower than byte 1 goes down.
        assert_eq!(at([0, 4, 4, 4], 8, 0), 2.0);
    }

    #[test]
    fn terrain_height_is_three_halves_over_root_six_per_step() {
        assert_eq!(terrain_height(F32::ZERO), F32::ZERO);
        let one = f64::from(terrain_height(F32::from_bits(0x3f80_0000)).to_f32());
        assert!((one - 1.5 / 6f64.sqrt()).abs() < 1e-6);
    }
}
