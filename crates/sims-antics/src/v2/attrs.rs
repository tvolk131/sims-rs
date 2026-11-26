//! Rust-style outer attributes: `#[bhav(id = 4096, label = "Main")]`, `#[slot(0)]`,
//! `#[temp(1)]`, `#[ret(temp = 0)]`, `#[addr(16896)]`, `#[bcon(260, 0)]`, ...
//!
//! The parser reads every `#[name]` / `#[name(args)]` in front of an item into [`Attr`]s, and
//! [`ItemAttrs::collect`] folds them into the facts the item takes. Each item kind then names
//! the attributes it accepts ([`ItemAttrs::only`]), so a misplaced attribute is an error at its
//! own span instead of a silent no-op.

use crate::error::{Error, Result, Span};
use crate::v2::ast::TreeFrame;

/// One attribute argument value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AttrValue {
    Int(i64),
    Str(String),
    Ident(String),
    /// `[a, b, ..]`: integers (`node_order = [0, 2, 1]`).
    List(Vec<i64>),
}

/// One attribute argument: `key = value` or a positional `value`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AttrArg {
    pub span: Span,
    pub key: Option<String>,
    pub value: AttrValue,
}

/// One parsed `#[name]` or `#[name(args)]`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Attr {
    pub span: Span,
    pub name: String,
    pub args: Option<Vec<AttrArg>>,
}

/// The facts a run of attributes gives an item.
#[derive(Debug, Clone, Default)]
pub(crate) struct ItemAttrs {
    /// Every attribute's name and span, in source order (for [`ItemAttrs::only`]).
    pub present: Vec<(String, Span)>,
    /// `#[bhav]` or `#[bhav(..)]`: the `fn` is a standalone BHAV, not an inline helper.
    pub bhav: Option<Span>,
    /// `#[inline]`: an explicit inline helper.
    pub inline: Option<Span>,
    /// `#[id(N)]` or `bhav(id = N)`.
    pub id: Option<(Span, u16)>,
    /// `#[guard_id(N)]`.
    pub guard_id: Option<(Span, u16)>,
    /// `bhav(label = "..")`.
    pub label: Option<(Span, String)>,
    /// `#[frame(..)]` and the header fields of `bhav(..)`.
    pub frame: Option<TreeFrame>,
    /// `#[context(inherit)]` or `bhav(context = inherit)`.
    pub inherit: Option<Span>,
    /// `#[ret(temp = N)]` or `bhav(ret_temp = N)`.
    pub ret_temp: Option<(Span, u8)>,
    /// `#[slot(N)]`.
    pub slot: Option<(Span, i64)>,
    /// `#[temp(N)]`.
    pub temp: Option<(Span, u8)>,
    /// `#[addr(N)]` or `#[bcon(T, E)]`: an encoded constant operand address.
    pub address: Option<(Span, u16)>,
}

/// Number of shared per-thread SimAntics temporaries (`temp[0..=7]`).
const TEMP_COUNT: i64 = 8;

impl ItemAttrs {
    /// Folds `attrs` into one set of facts, rejecting unknown attributes, malformed arguments
    /// and facts given twice.
    pub(crate) fn collect(attrs: Vec<Attr>) -> Result<Self> {
        let mut out = ItemAttrs::default();
        for attr in attrs {
            out.present.push((attr.name.clone(), attr.span));
            out.apply(attr)?;
        }
        Ok(out)
    }

    /// Rejects every attribute not named in `allowed`, naming `what` it was written on.
    pub(crate) fn only(&self, allowed: &[&str], what: &str) -> Result<()> {
        for (name, span) in &self.present {
            if !allowed.contains(&name.as_str()) {
                let hint = if allowed.is_empty() {
                    "takes no attributes".to_owned()
                } else {
                    format!(
                        "takes only {}",
                        allowed
                            .iter()
                            .map(|a| format!("'#[{a}]'"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                return Err(Error::at(
                    *span,
                    format!("'#[{name}]' is not allowed on {what} ({what} {hint})"),
                ));
            }
        }
        Ok(())
    }

    fn apply(&mut self, attr: Attr) -> Result<()> {
        let Attr { span, name, args } = attr;
        let needs_args = |args: Option<Vec<AttrArg>>| {
            args.ok_or_else(|| {
                Error::at(
                    span,
                    format!("'#[{name}]' needs arguments: '#[{name}(..)]'"),
                )
            })
        };
        match name.as_str() {
            "bhav" => {
                set_once(&mut self.bhav, span, span, "#[bhav]")?;
                for arg in args.unwrap_or_default() {
                    let key = arg.key.clone();
                    match key.as_deref() {
                        Some("id") => {
                            let id = arg_u16(&arg, "BHAV id")?;
                            set_once(&mut self.id, arg.span, (arg.span, id), "BHAV id")?;
                        }
                        Some("label") => {
                            let AttrValue::Str(label) = arg.value else {
                                return Err(Error::at(
                                    arg.span,
                                    "'label' takes a string (label = \"Name\")",
                                ));
                            };
                            set_once(&mut self.label, arg.span, (arg.span, label), "BHAV label")?;
                        }
                        Some("context") => {
                            expect_ident_value(&arg, "inherit", "context = inherit")?;
                            set_once(&mut self.inherit, arg.span, arg.span, "context")?;
                        }
                        Some("ret_temp") => {
                            let t = arg_temp(&arg, "return")?;
                            set_once(&mut self.ret_temp, arg.span, (arg.span, t), "return temp")?;
                        }
                        _ => self.frame_arg(span, &arg, "bhav")?,
                    }
                }
            }
            "inline" => {
                if args.is_some() {
                    return Err(Error::at(span, "'#[inline]' takes no arguments"));
                }
                set_once(&mut self.inline, span, span, "#[inline]")?;
            }
            "id" | "guard_id" => {
                let args = needs_args(args)?;
                let arg = single_positional(span, &name, args)?;
                let id = arg_u16(&arg, "BHAV id")?;
                let slot = if name == "id" {
                    &mut self.id
                } else {
                    &mut self.guard_id
                };
                set_once(slot, span, (arg.span, id), &format!("#[{name}]"))?;
            }
            "frame" => {
                for arg in needs_args(args)? {
                    self.frame_arg(span, &arg, "frame")?;
                }
                if self.frame.is_none() {
                    self.frame = Some(TreeFrame {
                        span,
                        ..TreeFrame::default()
                    });
                }
            }
            "context" => {
                let arg = single_positional(span, &name, needs_args(args)?)?;
                expect_ident_value(&arg, "inherit", "#[context(inherit)]")?;
                set_once(&mut self.inherit, span, span, "#[context]")?;
            }
            "ret" => {
                let args = needs_args(args)?;
                let [arg] = args.as_slice() else {
                    return Err(Error::at(span, "expected '#[ret(temp = N)]'"));
                };
                if arg.key.as_deref() != Some("temp") {
                    return Err(Error::at(arg.span, "expected '#[ret(temp = N)]'"));
                }
                let t = arg_temp(arg, "return")?;
                set_once(&mut self.ret_temp, span, (arg.span, t), "return temp")?;
            }
            "slot" => {
                let arg = single_positional(span, &name, needs_args(args)?)?;
                let AttrValue::Int(v) = arg.value else {
                    return Err(Error::at(arg.span, "'#[slot(N)]' takes an integer"));
                };
                set_once(&mut self.slot, span, (arg.span, v), "#[slot]")?;
            }
            "temp" => {
                let arg = single_positional(span, &name, needs_args(args)?)?;
                let t = arg_temp(&arg, "parameter")?;
                set_once(&mut self.temp, span, (arg.span, t), "#[temp]")?;
            }
            "addr" => {
                let arg = single_positional(span, &name, needs_args(args)?)?;
                let AttrValue::Int(v) = arg.value else {
                    return Err(Error::at(arg.span, "'#[addr(N)]' takes an integer"));
                };
                let address = u16::try_from(v)
                    .ok()
                    .filter(|&a| (a >> 7) <= 191)
                    .ok_or_else(|| {
                        Error::at(
                            arg.span,
                            format!(
                                "BCON address {v} is out of range 0..=24575 (table = address >> 7 must be <= 191)"
                            ),
                        )
                    })?;
                set_once(
                    &mut self.address,
                    span,
                    (arg.span, address),
                    "constant address",
                )?;
            }
            "bcon" => {
                let args = needs_args(args)?;
                let [table, entry] = args.as_slice() else {
                    return Err(Error::at(span, "expected '#[bcon(<chunk id>, <entry>)]'"));
                };
                let (AttrValue::Int(t), AttrValue::Int(e), None, None) =
                    (&table.value, &entry.value, &table.key, &entry.key)
                else {
                    return Err(Error::at(span, "expected '#[bcon(<chunk id>, <entry>)]'"));
                };
                let address = bcon_entry_address(table.span, *t, *e)?;
                set_once(&mut self.address, span, (span, address), "constant address")?;
            }
            other => {
                return Err(Error::at(
                    span,
                    format!(
                        "unknown attribute '#[{other}]' (expected bhav, inline, id, guard_id, frame, context, ret, slot, temp, addr, or bcon)"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// One BHAV header field of `#[frame(..)]` or `#[bhav(..)]`.
    fn frame_arg(&mut self, attr_span: Span, arg: &AttrArg, attr: &str) -> Result<()> {
        let frame = self.frame.get_or_insert(TreeFrame {
            span: attr_span,
            ..TreeFrame::default()
        });
        let Some(key) = arg.key.as_deref() else {
            let flag = match &arg.value {
                AttrValue::Ident(name) if name == "empty" => Some((&mut frame.empty, "empty")),
                AttrValue::Ident(name) if name == "share_tails" => {
                    Some((&mut frame.share_tails, "share_tails"))
                }
                _ => None,
            };
            if let Some((slot, name)) = flag {
                if std::mem::replace(slot, true) {
                    return Err(Error::at(
                        arg.span,
                        format!("duplicate header field '{name}'"),
                    ));
                }
                return Ok(());
            }
            return Err(Error::at(
                arg.span,
                format!("expected 'key = value', 'empty' or 'share_tails' in '#[{attr}(..)]'"),
            ));
        };
        let int = |what: &str| -> Result<i64> {
            match arg.value {
                AttrValue::Int(v) => Ok(v),
                _ => Err(Error::at(arg.span, format!("'{what}' takes an integer"))),
            }
        };
        let byte = |what: &str| -> Result<u8> {
            u8::try_from(int(what)?)
                .map_err(|_| Error::at(arg.span, format!("'{what}' must fit in a byte (0..=255)")))
        };
        let word = |what: &str| -> Result<u16> {
            u16::try_from(int(what)?)
                .map_err(|_| Error::at(arg.span, format!("'{what}' is out of u16 range")))
        };
        if matches!(key, "empty" | "share_tails") {
            return Err(Error::at(
                arg.span,
                format!("'{key}' takes no value: write '#[{attr}(.., {key})]'"),
            ));
        }
        let dup = match key {
            "args" => frame.args.replace(byte("args")?).is_some(),
            "locals" => frame.locals.replace(word("locals")?).is_some(),
            "format" => frame.format.replace(word("format")?).is_some(),
            "revision" => frame
                .revision
                .replace(
                    u32::try_from(int("revision")?)
                        .map_err(|_| Error::at(arg.span, "'revision' is out of u32 range"))?,
                )
                .is_some(),
            "kind" => frame.kind.replace(byte("kind")?).is_some(),
            "node_order" => {
                let AttrValue::List(entries) = &arg.value else {
                    return Err(Error::at(
                        arg.span,
                        "'node_order' takes a list of node numbers (node_order = [0, 2, 1])",
                    ));
                };
                let order = entries
                    .iter()
                    .map(|&n| {
                        u16::try_from(n).map_err(|_| {
                            Error::at(
                                arg.span,
                                format!("'node_order' entry {n} is not a node number"),
                            )
                        })
                    })
                    .collect::<Result<Vec<u16>>>()?;
                frame.node_order.replace(order).is_some()
            }
            other => {
                let more = if attr == "bhav" {
                    "id, label, context, ret_temp, "
                } else {
                    ""
                };
                return Err(Error::at(
                    arg.span,
                    format!(
                        "unknown '#[{attr}(..)]' field '{other}' (expected {more}args, locals, format, revision, kind, node_order, empty, or share_tails)"
                    ),
                ));
            }
        };
        if dup {
            return Err(Error::at(
                arg.span,
                format!("duplicate header field '{key}'"),
            ));
        }
        Ok(())
    }
}

/// Encodes BCON chunk `table` entry `entry` as a constant operand address (`(t << 7) | e`).
pub(crate) fn bcon_entry_address(span: Span, table: i64, entry: i64) -> Result<u16> {
    let t = u16::try_from(table)
        .ok()
        .and_then(crate::v2::parser::bcon_chunk_table)
        .ok_or_else(|| {
            Error::at(
                span,
                format!(
                    "BCON chunk id {table} is not addressable (expected 4096..=4159 private, 8192..=8255 semi-global, or 256..=319 global)"
                ),
            )
        })?;
    if !(0..=127).contains(&entry) {
        return Err(Error::at(
            span,
            format!("BCON entry index {entry} is out of range 0..=127"),
        ));
    }
    Ok((t << 7) | entry as u16)
}

fn set_once<T>(slot: &mut Option<T>, span: Span, value: T, what: &str) -> Result<()> {
    if slot.replace(value).is_some() {
        return Err(Error::at(span, format!("duplicate {what}")));
    }
    Ok(())
}

fn single_positional(span: Span, name: &str, args: Vec<AttrArg>) -> Result<AttrArg> {
    let mut args = args.into_iter();
    match (args.next(), args.next()) {
        (Some(arg), None) if arg.key.is_none() => Ok(arg),
        _ => Err(Error::at(
            span,
            format!("'#[{name}(..)]' takes exactly one positional argument"),
        )),
    }
}

fn arg_u16(arg: &AttrArg, what: &str) -> Result<u16> {
    match arg.value {
        AttrValue::Int(v) => u16::try_from(v)
            .map_err(|_| Error::at(arg.span, format!("{what} {v} out of u16 range"))),
        _ => Err(Error::at(arg.span, format!("{what} must be an integer"))),
    }
}

fn arg_temp(arg: &AttrArg, what: &str) -> Result<u8> {
    match arg.value {
        AttrValue::Int(v) if (0..TEMP_COUNT).contains(&v) => Ok(v as u8),
        AttrValue::Int(v) => Err(Error::at(
            arg.span,
            format!(
                "{what} temp slot {v} is out of range (SimAntics has temp[0]..=temp[{}])",
                TEMP_COUNT - 1
            ),
        )),
        _ => Err(Error::at(
            arg.span,
            format!("{what} temp slot must be an integer"),
        )),
    }
}

fn expect_ident_value(arg: &AttrArg, want: &str, spelling: &str) -> Result<()> {
    if arg.value == AttrValue::Ident(want.to_owned()) {
        Ok(())
    } else {
        Err(Error::at(arg.span, format!("expected '{spelling}'")))
    }
}
