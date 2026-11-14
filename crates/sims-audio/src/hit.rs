//! HIT programs: the bytecode of the sound scripts.
//!
//! A HIT file is a 16-byte header (`HIT!`, version 1, 8, `TRAX`) and then
//! code. Addresses are byte offsets into the file; the first instruction
//! is at 16. There is no table of entry points in the file: the HOT
//! file's `[TrackData]` maps each track data ID to an address.
//!
//! An instruction is one opcode byte and its operands. The opcode names
//! and numbers are the ones each HSM file lists (`note 1` to `waitle
//! 82`). The operand layouts come from public notes (the Niotso wiki's
//! HIT page and FreeSO's HIT interpreter) and were checked against every
//! HIT file in the install: decoding each file from byte 16 to its end
//! lands on every entry point its HOT file lists, and every branch and
//! call names a track data ID whose address is an instruction (the gated
//! test). Opcodes the install never uses keep the public notes' layouts,
//! **unverified**.
//!
//! Branches, calls and jumps name a track data ID, not an address. `jump`
//! has two forms: a 4-byte ID, or one register byte holding the target.
//! Nothing in the opcode tells them apart; here the 4-byte form is taken
//! when those bytes are a track data ID of the program, as they are for
//! 399 of the install's 401 jumps. The other two (in
//! `simsgeneratedhitsource.hit`) only decode as register jumps. How the
//! original decides is **unverified**.

use crate::{Error, Result};

/// The header's length; the first instruction's address.
pub const HEADER: usize = 16;

/// What one operand is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// A register number (one byte).
    Register,
    /// A signed byte.
    Byte,
    /// A signed 32-bit little-endian number.
    Long,
    /// A track data ID (four bytes).
    Target,
}

use Operand::{Byte, Long, Register as R, Target};

/// The instruction names, by opcode, as the HSM files list them, with
/// their operands. Opcodes 0, 26 and above 82 are not instructions.
pub const OPCODES: [Option<(&str, &[Operand])>; 83] = [
    None,
    Some(("note", &[R, R])),
    Some(("note_on", &[R])),
    Some(("note_off", &[R])),
    Some(("loadb", &[R, Byte])),
    Some(("loadl", &[R, Long])),
    Some(("set", &[R, R])),
    Some(("call", &[Target])),
    Some(("return", &[])),
    Some(("wait", &[R])),
    Some(("callentrypoint", &[Target])),
    Some(("wait_samp", &[])),
    Some(("end", &[])),
    Some(("jump", &[Target])),
    Some(("test", &[R])),
    Some(("nop", &[])),
    Some(("add", &[R, R])),
    Some(("sub", &[R, R])),
    Some(("div", &[R, R])),
    Some(("mul", &[R, R])),
    Some(("cmp", &[R, R])),
    Some(("less", &[R, R])),
    Some(("greater", &[R, R])),
    Some(("not", &[R])),
    Some(("rand", &[R, R, R])),
    Some(("abs", &[R])),
    None,
    Some(("error", &[R])),
    Some(("assert", &[R])),
    Some(("add_to_group", &[R, R, R])),
    Some(("remove_from_group", &[R, R, R])),
    Some(("get_var", &[R, R])),
    Some(("loop", &[])),
    Some(("set_loop", &[])),
    Some(("callback", &[R, R])),
    Some(("smart_add", &[R, R])),
    Some(("smart_remove", &[R])),
    Some(("smart_removeall", &[])),
    Some(("smart_setcrit", &[R])),
    Some(("smart_choose", &[R])),
    Some(("and", &[R, R])),
    Some(("nand", &[R, R])),
    Some(("or", &[R, R])),
    Some(("nor", &[R, R])),
    Some(("xor", &[R, R])),
    Some(("max", &[R, R])),
    Some(("min", &[R, R])),
    Some(("inc", &[R])),
    Some(("dec", &[R])),
    Some(("printreg", &[R])),
    Some(("play_trk", &[R])),
    Some(("kill_trk", &[R])),
    Some(("push", &[R])),
    Some(("push_mask", &[R, R])),
    Some(("push_vars", &[R, R])),
    Some(("call_mask", &[Target, R])),
    Some(("call_push", &[Target, R])),
    Some(("pop", &[R])),
    Some(("test1", &[])),
    Some(("test2", &[])),
    Some(("test3", &[])),
    Some(("test4", &[])),
    Some(("ifeq", &[Target])),
    Some(("ifne", &[Target])),
    Some(("ifgt", &[Target])),
    Some(("iflt", &[Target])),
    Some(("ifge", &[Target])),
    Some(("ifle", &[Target])),
    Some(("smart_setlist", &[R])),
    Some(("seqgroup_kill", &[R])),
    Some(("seqgroup_wait", &[R])),
    Some(("seqgroup_return", &[R])),
    Some(("getsrcdatafield", &[R, R, R])),
    Some(("seqgroup_trkid", &[R, R])),
    Some(("setll", &[R, R])),
    Some(("setlt", &[R, R])),
    Some(("settl", &[R, R])),
    Some(("waiteq", &[R, R])),
    Some(("waitne", &[R, R])),
    Some(("waitgt", &[R, R])),
    Some(("waitlt", &[R, R])),
    Some(("waitge", &[R, R])),
    Some(("waitle", &[R, R])),
];

/// The `jump` opcode, whose operand has two forms.
pub const JUMP: u8 = 13;

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub address: u32,
    pub opcode: u8,
    /// The operands with their kinds, in order. A register jump has one
    /// [`Operand::Register`].
    pub operands: Vec<(Operand, i64)>,
    /// The instruction's length in bytes.
    pub length: u32,
}

impl Instruction {
    pub fn name(&self) -> &'static str {
        OPCODES[usize::from(self.opcode)].map_or("?", |(name, _)| name)
    }

    /// The operand values alone.
    pub fn values(&self) -> Vec<i64> {
        self.operands.iter().map(|(_, v)| *v).collect()
    }
}

/// A HIT program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub major: u32,
    pub minor: u32,
    pub code: Vec<u8>,
}

impl Program {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER || &bytes[..4] != b"HIT!" || &bytes[12..16] != b"TRAX" {
            return Err(Error::format("HIT", 0, "not a HIT file"));
        }
        let word = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
        Ok(Self {
            major: word(4),
            minor: word(8),
            code: bytes.to_vec(),
        })
    }

    /// Decodes the instruction at `address`. `is_target` says whether a
    /// number is one of the program's track data IDs, which decides the
    /// form of a `jump`.
    pub fn decode(&self, address: u32, is_target: &dyn Fn(u32) -> bool) -> Result<Instruction> {
        let at = address as usize;
        let fail = |offset: usize, message: &str| Err(Error::format("HIT", offset, message));
        if at < HEADER || at >= self.code.len() {
            return fail(at, "address outside the program");
        }
        let opcode = self.code[at];
        let Some((_, layout)) = OPCODES.get(usize::from(opcode)).copied().flatten() else {
            return fail(at, &format!("unknown opcode {opcode}"));
        };
        let mut operands = Vec::with_capacity(layout.len());
        let mut pos = at + 1;
        let long = |pos: usize| {
            self.code
                .get(pos..pos + 4)
                .map(|b| i32::from_le_bytes(b.try_into().expect("4 bytes")))
        };
        for &kind in layout {
            let kind = if opcode == JUMP {
                match long(pos) {
                    Some(id) if is_target(id as u32) => Target,
                    _ => R,
                }
            } else {
                kind
            };
            let (value, size) = match kind {
                R => (self.code.get(pos).map(|&b| i64::from(b)), 1),
                Byte => (self.code.get(pos).map(|&b| i64::from(b as i8)), 1),
                Long => (long(pos).map(i64::from), 4),
                Target => (long(pos).map(|v| i64::from(v as u32)), 4),
            };
            let Some(value) = value else {
                return fail(pos, "operand runs past the end");
            };
            operands.push((kind, value));
            pos += size;
        }
        Ok(Instruction {
            address,
            opcode,
            operands,
            length: (pos - at) as u32,
        })
    }

    /// Decodes the whole program from its first instruction to its end.
    pub fn disassemble(&self, is_target: &dyn Fn(u32) -> bool) -> Result<Vec<Instruction>> {
        let mut out = Vec::new();
        let mut address = HEADER as u32;
        while (address as usize) < self.code.len() {
            let instruction = self.decode(address, is_target)?;
            address += instruction.length;
            out.push(instruction);
        }
        Ok(out)
    }
}
