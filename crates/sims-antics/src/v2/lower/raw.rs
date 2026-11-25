//! Raw `.air` blocks: parsing, validation, label collection, and `stack_obj` clobber facts of raw
//! statements.

use super::*;

/// Parses a `raw { ... }` block's `.air` text the same way the lowering validates it.
pub(super) fn parse_raw_air_stmts(code: &str) -> Option<Vec<crate::ast::Stmt>> {
    let wrapped = format!("tree 4096 \"__raw_check\" {{\n{code}\n}}\n");
    let parsed = crate::parser::parse_module(&wrapped).ok()?;
    match parsed.items.into_iter().next() {
        Some(crate::ast::Item::Tree(t)) => Some(t.body),
        _ => None,
    }
}

/// Whether a `raw { ... }` block may overwrite the frame's `stack_obj` (or names `__self_obj`
/// itself), which forces the `__self_obj` save in a Sim-on-object tree. Text that does not parse
/// is assumed to clobber.
pub(super) fn raw_air_may_clobber_stack_obj(code: &str) -> bool {
    if code.contains("__self_obj") {
        return true;
    }
    match parse_raw_air_stmts(code) {
        Some(stmts) => raw_air_stmts_clobber_stack_obj(&stmts),
        None => true,
    }
}

pub(super) fn is_stack_obj_operand(op: &crate::ast::OperandRef) -> bool {
    matches!(
        op,
        crate::ast::OperandRef::Direct(crate::ast::Operand {
            scope: 10,
            index: 0
        })
    )
}

/// Whether a raw `op <opcode> (bytes)` may write the running frame's `stack_obj`, decoded the
/// way `sims-vm` runs it. BHAV calls never do (`stack_obj` is per frame and never copied back).
/// Of the primitives, only these write it: the ones that store into an operand the bytes name
/// (Expression, Random Number, Get Direction To, Relationship, Set To Next) when that operand is
/// the stack object (scope 10), and the ones that set it outright (Find Best Interaction, Find
/// Best Object, Make New Character, Gosub Found Action, Create Object). Unknown opcodes may.
pub(super) fn raw_op_may_write_stack_obj(opcode: u16, ops: &[u8; 8]) -> bool {
    const STACK_OBJ_SCOPE: u16 = 10;
    let word = |i: usize| u16::from_le_bytes([ops[i], ops[i + 1]]);
    match opcode {
        0x0100.. => false,
        // Expression: left operand scope byte 6.
        0x0002 => u16::from(ops[6]) == STACK_OBJ_SCOPE,
        // Random Number / Get Direction To: destination scope word 2.
        0x0008 | 0x000C => word(2) == STACK_OBJ_SCOPE,
        // Relationship: value operand scope byte 4 (it is stored when reading).
        0x001A => u16::from(ops[4]) == STACK_OBJ_SCOPE,
        // Set To Next: the stack object unless flag 0x80 names a destination (scope byte 5).
        0x001F => ops[4] & 0x80 == 0 || u16::from(ops[5]) == STACK_OBJ_SCOPE,
        0x0003 | 0x000E | 0x0013 | 0x001E | 0x002A => true,
        0x0000..=0x0033 => false,
        _ => true,
    }
}

pub(super) fn raw_condition_clobbers_stack_obj(cond: &crate::ast::Condition) -> bool {
    match cond {
        // `inc_lt` / `dec_gt` store the stepped value back into their left operand.
        crate::ast::Condition::Compare { lhs, op, .. } => {
            matches!(
                op,
                crate::ast::CompareOp::IncLt | crate::ast::CompareOp::DecGt
            ) && is_stack_obj_operand(lhs)
        }
        crate::ast::Condition::Call { .. } => false,
        crate::ast::Condition::Primitive { prim, .. } => raw_primitive_clobbers_stack_obj(prim),
        crate::ast::Condition::RawOp {
            opcode, operands, ..
        } => raw_op_may_write_stack_obj(*opcode, operands),
        crate::ast::Condition::Not(inner) => raw_condition_clobbers_stack_obj(inner),
    }
}

pub(super) fn raw_primitive_clobbers_stack_obj(prim: &crate::ast::PrimitiveCall) -> bool {
    use crate::ast::PrimitiveCall as P;
    match prim {
        // These set the running frame's stack object (or may, through an autonomy choice).
        P::FindBestInteraction
        | P::FindBestObject { .. }
        | P::CreateObject { .. }
        | P::MakeNewCharacter { .. }
        | P::GosubFoundAction => true,
        P::SetToNext { dest, .. } => dest.as_ref().is_none_or(is_stack_obj_operand),
        P::ExprRaw { lhs, .. } => is_stack_obj_operand(lhs),
        P::Random { dst, .. } | P::GetDirectionTo { dst, .. } => is_stack_obj_operand(dst),
        P::Relationship { value, .. } => is_stack_obj_operand(value),
        _ => false,
    }
}

pub(super) fn raw_air_stmts_clobber_stack_obj(stmts: &[crate::ast::Stmt]) -> bool {
    use crate::ast::Stmt;
    stmts.iter().any(|s| match s {
        Stmt::Assign { lhs, .. } => is_stack_obj_operand(lhs),
        Stmt::RawOp {
            opcode, operands, ..
        }
        | Stmt::InlineRawOp {
            opcode, operands, ..
        } => raw_op_may_write_stack_obj(*opcode, operands),
        Stmt::Primitive { prim, .. } => raw_primitive_clobbers_stack_obj(prim),
        Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            raw_condition_clobbers_stack_obj(cond)
                || raw_air_stmts_clobber_stack_obj(then_body)
                || raw_air_stmts_clobber_stack_obj(else_body)
        }
        Stmt::While { cond, body, .. } => {
            raw_condition_clobbers_stack_obj(cond) || raw_air_stmts_clobber_stack_obj(body)
        }
        Stmt::Loop { body, .. } => raw_air_stmts_clobber_stack_obj(body),
        Stmt::Label { .. }
        | Stmt::Let { .. }
        | Stmt::Call { .. }
        | Stmt::Idle { .. }
        | Stmt::Break { .. }
        | Stmt::Continue { .. }
        | Stmt::Return { .. } => false,
    })
}

pub(super) fn collect_raw_air_labels<'a>(
    stmts: &'a [crate::ast::Stmt],
    labels: &mut BTreeSet<&'a str>,
) {
    for s in stmts {
        match s {
            crate::ast::Stmt::Label { name, .. } => {
                labels.insert(name.as_str());
            }
            crate::ast::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_raw_air_labels(then_body, labels);
                collect_raw_air_labels(else_body, labels);
            }
            crate::ast::Stmt::While { body, .. } => {
                collect_raw_air_labels(body, labels);
            }
            _ => {}
        }
    }
}

pub(super) fn validate_raw_air_target(
    target: &crate::ast::Target,
    defined_labels: &BTreeSet<&str>,
    span: Span,
) -> Result<()> {
    match target {
        crate::ast::Target::Label(name) if defined_labels.contains(name.as_str()) => Ok(()),
        other => Err(Error::at(
            span,
            format!(
                "'raw {{ ... }}' cannot branch to '{other:?}' while cleanups, inline helpers, or cancel_on_abort loops are active"
            ),
        )),
    }
}

pub(super) fn validate_raw_air_stmts(
    stmts: &[crate::ast::Stmt],
    in_raw_loop: bool,
    defined_labels: &BTreeSet<&str>,
    span: Span,
) -> Result<()> {
    for s in stmts {
        match s {
            crate::ast::Stmt::Return { .. } => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot contain 'return' while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::Break { .. } | crate::ast::Stmt::Continue { .. } if !in_raw_loop => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot break/continue out of its block while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::RawOp { branches, .. } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Primitive {
                branches: Some(branches),
                ..
            } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Call {
                branches: Some(branches),
                ..
            } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Call { branches: None, .. } => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot invoke an unbranched subroutine call (whose false branch exits the tree) while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                validate_raw_air_stmts(then_body, in_raw_loop, defined_labels, span)?;
                validate_raw_air_stmts(else_body, in_raw_loop, defined_labels, span)?;
            }
            crate::ast::Stmt::While { body, .. } => {
                validate_raw_air_stmts(body, true, defined_labels, span)?;
            }
            _ => {}
        }
    }
    Ok(())
}
