//! Small `.air` text helpers: canonical cells, operator spelling, exit swaps, condition negation,
//! and string escaping.

use super::*;

/// Normalizes `.air` operand spellings to their canonical SimAntics VM storage cell so alias
/// analysis treats `self.prep_score` (`stack.prep_score`) and `self.data[31]` (`stack.data[31]`)
/// as the exact same cell. An `#[slot(N)] let` local is frame word `N`, so it is `local[N]`.
pub(super) fn canonical_air_cell(air: &str) -> String {
    if let Some(n) = slot_word(air) {
        return format!("local[{n}]");
    }
    if let Some((prefix, rest)) = air.split_once('.')
        && matches!(prefix, "my" | "stack")
    {
        match rest {
            "id" => return format!("{prefix}.data[11]"),
            "in_use" => return format!("{prefix}.data[25]"),
            "room" => return format!("{prefix}.data[29]"),
            "prep_score" => return format!("{prefix}.data[31]"),
            "cook_score" => return format!("{prefix}.data[32]"),
            "priority" => return format!("{prefix}.person[33]"),
            _ => {}
        }
        if let Some(mname) = rest.strip_prefix("motive.")
            && let Some(id) = motive_by_name(mname)
        {
            return format!("{prefix}.motive[{id}]");
        }
        if let Some(sname) = rest.strip_prefix("skill.")
            && let Some(id) = skill_by_name(sname)
        {
            return format!("{prefix}.person[{id}]");
        }
        if let Some(pname) = rest.strip_prefix("person.")
            && let Some(id) = person_field_by_name(pname)
        {
            return format!("{prefix}.person[{id}]");
        }
    }
    air.to_owned()
}

/// The frame word an `#[slot(N)] let` local's `.air` name (`__slot<N>_...`) fixes.
pub(super) fn slot_word(air_name: &str) -> Option<u8> {
    let rest = air_name.strip_prefix("__slot")?;
    let (n, _) = rest.split_once('_')?;
    n.parse().ok()
}

pub(super) fn is_arith_or_bit_binop(op: V2BinOp) -> bool {
    matches!(
        op,
        V2BinOp::Add
            | V2BinOp::Sub
            | V2BinOp::Mul
            | V2BinOp::Div
            | V2BinOp::Mod
            | V2BinOp::BitAnd
            | V2BinOp::BitOr
            | V2BinOp::BitXor
    )
}

pub(super) fn is_compound_arith_or_neg(expr: &V2Expr) -> bool {
    match expr {
        V2Expr::Binary { op, .. } => is_arith_or_bit_binop(*op),
        V2Expr::Unary {
            op: V2UnaryOp::Neg,
            inner,
            ..
        } => !matches!(inner.as_ref(), V2Expr::Int(_, _)),
        _ => false,
    }
}

pub(super) fn flatten_arith_chain(expr: &V2Expr) -> (&V2Expr, Vec<(V2BinOp, &V2Expr)>) {
    let mut ops = Vec::new();
    let mut cur = expr;
    while let V2Expr::Binary { lhs, op, rhs, .. } = cur
        && is_arith_or_bit_binop(*op)
    {
        ops.push((*op, rhs.as_ref()));
        cur = lhs.as_ref();
    }
    ops.reverse();
    (cur, ops)
}

pub(super) fn single_bit_of_mask(mask: i16) -> Option<u8> {
    let bits = mask as u16;
    if bits != 0 && (bits & (bits - 1)) == 0 {
        Some((bits.trailing_zeros() as u8) + 1)
    } else {
        None
    }
}

/// An `.air` simple condition with its exits swapped: the same test node (and so the same
/// last result), taking the other branch. `.air` `!c` swaps a node's exits.
pub(super) fn swap_exits(cond: &str) -> String {
    match cond.strip_prefix('!') {
        Some(inner) => inner.to_owned(),
        None => format!("!{cond}"),
    }
}

pub(super) fn cmp_op_str(op: V2BinOp) -> &'static str {
    match op {
        V2BinOp::Eq => "==",
        V2BinOp::Ne => "!=",
        V2BinOp::Lt => "<",
        V2BinOp::Le => "<=",
        V2BinOp::Gt => ">",
        V2BinOp::Ge => ">=",
        V2BinOp::FlagTest => "flag_test",
        _ => unreachable!("non-comparison op"),
    }
}

pub(super) fn expr_starts_with_not_self_field(expr: &V2Expr, field_name: &str) -> bool {
    match expr {
        V2Expr::Unary {
            op: V2UnaryOp::Not,
            inner,
            ..
        } => matches!(
            inner.as_ref(),
            V2Expr::Field { base, field, .. }
                if field == field_name
                    && matches!(base.as_ref(), V2Expr::Ident(_, b) if b == "self")
        ),
        V2Expr::Binary {
            lhs,
            op: V2BinOp::And,
            ..
        } => expr_starts_with_not_self_field(lhs, field_name),
        _ => false,
    }
}

/// `text` with `pat` replaced by `with` outside `"..."` string literals (a dialog string may
/// spell anything).
pub(super) fn replace_outside_strings(text: &str, pat: &str, with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_str = false;
    let mut escaped = false;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if !in_str && rest.starts_with(pat) {
            out.push_str(with);
            i += pat.len();
            continue;
        }
        let c = rest.chars().next().expect("non-empty rest");
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// The operands of a flat `a op b op ..` chain (`op` is `&&` or `||`), in evaluation order.
pub(super) fn flatten_logic_chain<'e>(expr: &'e V2Expr, op: V2BinOp, out: &mut Vec<&'e V2Expr>) {
    match expr {
        V2Expr::Binary {
            lhs, op: o, rhs, ..
        } if *o == op => {
            flatten_logic_chain(lhs, op, out);
            flatten_logic_chain(rhs, op, out);
        }
        other => out.push(other),
    }
}

/// The `.air` condition with its exits swapped: `!c`, or `c` for `!c`.
pub(super) fn negate_air_cond(text: &str) -> String {
    match text.strip_prefix('!') {
        Some(rest) => rest.to_owned(),
        None => format!("!{text}"),
    }
}

pub(super) fn make_slug(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "object".to_owned()
    } else {
        out
    }
}

pub(super) fn camel_label(name: &str) -> String {
    let mut out = String::new();
    let mut cap = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if cap {
                out.push(ch.to_ascii_uppercase());
                cap = false;
            } else {
                out.push(ch);
            }
        } else {
            cap = true;
        }
    }
    if out.is_empty() {
        "Object".to_owned()
    } else {
        out
    }
}

pub(super) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => {
            let mut out = first.to_uppercase().to_string();
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

pub(super) fn sanitize_tree_ident(label: &str, idx: usize) -> String {
    let camel = camel_label(label);
    if camel.is_empty() {
        format!("Interaction_{idx}")
    } else {
        camel
    }
}

pub(super) fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => {
                for b in c.encode_utf8(&mut [0u8; 4]).as_bytes() {
                    let _ = write!(out, "\\x{b:02x}");
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[allow(dead_code)]
pub(super) fn _keep_furniture_shape_import(_: FurnitureShape) {}

#[cfg(test)]
mod tests {
    use super::{canonical_air_cell, raw_op_may_write_stack_obj};

    /// The raw clobber check follows the VM: operand stores only through scope 10, and the
    /// primitives that set the stack object outright.
    #[test]
    fn raw_ops_that_write_the_stack_object() {
        let none = [0u8; 8];
        for opcode in [
            0x0001, 0x0006, 0x0009, 0x000B, 0x0010, 0x0017, 0x002B, 0x002D, 0x0100,
        ] {
            assert!(!raw_op_may_write_stack_obj(opcode, &none), "{opcode:#06x}");
        }
        for opcode in [0x0003, 0x000E, 0x0013, 0x001E, 0x002A, 0x0034, 0x00FF] {
            assert!(raw_op_may_write_stack_obj(opcode, &none), "{opcode:#06x}");
        }
        // Expression: left scope byte 6.
        assert!(raw_op_may_write_stack_obj(
            0x0002,
            &[0, 0, 0, 0, 0, 5, 10, 7]
        ));
        assert!(!raw_op_may_write_stack_obj(
            0x0002,
            &[0, 0, 0, 0, 0, 5, 8, 10]
        ));
        // Random Number / Get Direction To: destination scope word 2.
        for opcode in [0x0008, 0x000C] {
            assert!(raw_op_may_write_stack_obj(
                opcode,
                &[0, 0, 10, 0, 0, 0, 0, 0]
            ));
            assert!(!raw_op_may_write_stack_obj(
                opcode,
                &[10, 0, 8, 0, 0, 0, 0, 0]
            ));
        }
        // Relationship: value scope byte 4.
        assert!(raw_op_may_write_stack_obj(
            0x001A,
            &[0, 0, 0, 0, 10, 0, 0, 0]
        ));
        assert!(!raw_op_may_write_stack_obj(
            0x001A,
            &[0, 0, 0, 0, 25, 0, 0, 0]
        ));
        // Set To Next: the stack object unless flag 0x80 names another destination.
        assert!(raw_op_may_write_stack_obj(0x001F, &none));
        assert!(!raw_op_may_write_stack_obj(
            0x001F,
            &[0, 0, 0, 0, 0x80, 25, 0, 1]
        ));
        assert!(raw_op_may_write_stack_obj(
            0x001F,
            &[0, 0, 0, 0, 0x80, 10, 0, 0]
        ));
    }

    /// Pins the `person.<field>` spellings to the same indices the `.air` parser, the lifter,
    /// and the UI use, so alias analysis treats both spellings as one VM cell.
    #[test]
    fn person_field_cells_match_air_indices() {
        for (name, index) in [
            ("cooking", 10),
            ("charisma", 11),
            ("mechanical", 12),
            ("mechanic", 12),
            ("creativity", 15),
            ("body", 17),
            ("logic", 18),
            ("priority", 33),
        ] {
            assert_eq!(
                canonical_air_cell(&format!("my.person.{name}")),
                format!("my.person[{index}]"),
                "{name}"
            );
            assert_eq!(
                canonical_air_cell(&format!("my.skill.{name}")),
                if name == "priority" {
                    format!("my.skill.{name}")
                } else {
                    format!("my.person[{index}]")
                },
                "skill.{name}"
            );
        }
        assert_eq!(canonical_air_cell("stack.priority"), "stack.person[33]");
        assert_eq!(canonical_air_cell("my.person.unknown"), "my.person.unknown");
    }
}
