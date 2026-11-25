//! The table of primitive builtins (`PRIM_BUILTINS`, `OP_BUILTINS`) and builtin-name lookups.

use super::*;

/// Built-in v2 call names a standalone `#[bhav] fn` identifier may not shadow.
pub(super) const BUILTIN_CALL_NAMES: &[&str] = &[
    "sleep",
    "idle",
    "play_sound",
    "stop_all_sounds",
    "debit_funds",
    "credit_funds",
    "destroy_self",
    "set_graphic",
    "route_to",
    "snap",
    "set_motive_change",
    "clear_motive_changes",
    "dialog",
    "random",
    "can_afford",
    "find_best_object",
    "create_object",
    "objects_of",
    "Some",
    "None",
];

pub(super) const U8: PrimArg = PrimArg::Int(0, 255);

pub(super) const U16: PrimArg = PrimArg::Int(0, 65535);

pub(super) const I16: PrimArg = PrimArg::Int(-32768, 32767);

pub(super) const PRIM_BUILTINS: &[PrimBuiltin] = &[
    PrimBuiltin {
        v2: "push_interaction",
        air: "push_interaction",
        keys: &[
            ("menu", "menu", U8),
            ("object", "object", U8),
            ("priority", "priority", U8),
            ("flags", "flags", U8),
            ("icon", "icon", PrimArg::Local),
        ],
    },
    PrimBuiltin {
        v2: "run_tree_by_name",
        air: "run_tree_by_name",
        keys: &[
            ("table", "table", U16),
            ("index", "index", U8),
            ("mode", "mode", U8),
            ("global", "global", PrimArg::Bool),
        ],
    },
    PrimBuiltin {
        v2: "go_to_relative",
        air: "go_to_relative",
        keys: &[
            ("location", "location", PrimArg::Int(-128, 127)),
            ("facing", "facing", U8),
            ("options", "options", U8),
        ],
    },
    PrimBuiltin {
        v2: "manage_inventory",
        air: "manage_inventory",
        keys: &[
            ("op", "op", U8),
            ("kind", "kind", U8),
            ("flags", "flags", U16),
            ("guid", "guid", PrimArg::Guid),
        ],
    },
    PrimBuiltin {
        v2: "refresh",
        air: "refresh",
        keys: &[
            ("target", "target", PrimArg::RefreshTarget),
            ("mode", "mode", U16),
        ],
    },
    PrimBuiltin {
        v2: "action_label",
        air: "action_string",
        keys: &[("table", "table", U16), ("index", "index", U8)],
    },
    PrimBuiltin {
        v2: "show_string",
        air: "show_string",
        keys: &[("table", "table", U16), ("index", "index", U16)],
    },
    PrimBuiltin {
        v2: "set_headline",
        air: "set_headline",
        keys: &[
            ("kind", "kind", U8),
            ("index", "index", U8),
            ("duration", "duration", I16),
            ("options", "options", U16),
            ("stack_obj", "stack_obj", PrimArg::Bool),
            ("local", "local", PrimArg::Local),
        ],
    },
    PrimBuiltin {
        v2: "special_effect",
        air: "special_effect",
        keys: &[
            ("duration", "duration", I16),
            ("kind", "kind", U8),
            ("size", "size", U8),
            ("flags", "flags", U8),
            ("str_index", "str_index", U8),
        ],
    },
    PrimBuiltin {
        v2: "change_suit",
        air: "change_suit",
        keys: &[
            ("index", "index", U8),
            ("scope", "scope", U8),
            ("remove", "remove", PrimArg::Bool),
        ],
    },
    PrimBuiltin {
        v2: "burn",
        air: "burn",
        keys: &[("mode", "mode", U8), ("flags", "flags", U8)],
    },
    PrimBuiltin {
        v2: "look_towards",
        air: "look_towards",
        keys: &[("mode", "mode", I16)],
    },
    PrimBuiltin {
        v2: "drop_onto",
        air: "drop_onto",
        keys: &[
            ("src", "src", PrimArg::SlotOrParam),
            ("dst", "dst", PrimArg::SlotOrParam),
        ],
    },
    // Reach (opcode 47): mode 1 reaches for the slot whose number is in parameter `slot:`.
    PrimBuiltin {
        v2: "reach",
        air: "reach",
        keys: &[
            ("mode", "mode", I16),
            ("moves", "moves", PrimArg::Bool),
            ("slot", "param", PrimArg::ParamIndex),
        ],
    },
    // Make New Character (opcode 19): skin, age, and sex are read from locals; `species` picks
    // the template (0 person, 1 dog, 2 cat).
    PrimBuiltin {
        v2: "make_new_character",
        air: "make_new_character",
        keys: &[
            ("skin", "skin", PrimArg::Local),
            ("age", "age", PrimArg::Local),
            ("sex", "sex", PrimArg::Local),
            ("species", "species", PrimArg::Int(0, 2)),
        ],
    },
    // `find_location(local:)` reads the target position from the local.
    PrimBuiltin {
        v2: "find_location",
        air: "find_location",
        keys: &[
            ("mode", "mode", U8),
            ("flags", "flags", U8),
            ("local", "local", PrimArg::Local),
        ],
    },
    // Relationship (opcode 26): `value:` is the cell the number is read into, or (flag 4) the
    // value written; `local:` holds the other person for directions 2 and 3.
    PrimBuiltin {
        v2: "relationship",
        air: "relationship",
        keys: &[
            ("column", "column", U8),
            ("dir", "dir", U8),
            ("flags", "flags", U8),
            ("local", "local", PrimArg::Local),
            ("value", "value", PrimArg::RelValue),
        ],
    },
    // Old Relationship (opcode 24): the number goes through parameter `param`.
    PrimBuiltin {
        v2: "old_relationship",
        air: "old_relationship",
        keys: &[
            ("write", "write", PrimArg::Bool),
            ("column", "column", U8),
            ("param", "param", PrimArg::Int(-128, 127)),
            ("dir", "dir", U8),
            ("flags", "flags", U8),
        ],
    },
];

/// Argless low-level opcodes exposed as `name()`; each lowers to `op 0xNNNN (0, ..., 0)` and
/// leaves `stack_obj` exactly as the primitive does (so `gosub_found_action()` sees
/// `find_best_interaction()`'s pick).
pub(super) const OP_BUILTINS: &[(&str, u16)] = &[
    ("find_best_interaction", 0x0003),
    ("grab", 0x0004),
    ("drop", 0x0005),
    ("gosub_found_action", 0x001E),
    ("notify_out_of_idle", 0x0031),
];

/// Bitflags key of `self.flags`: an untyped bitfield that accepts any `bitflags` value.
pub(super) const ANY_BITFLAGS: &str = "builtin::AnyFlags";

/// The person-data word behind `sim.personality.<trait>` (the same words the character file
/// stores the five personality traits in).
pub(super) fn personality_trait(span: Span, name: &str) -> Result<u8> {
    match name {
        "nice" => Ok(2),
        "active" => Ok(3),
        "playful" => Ok(5),
        "outgoing" => Ok(6),
        "neat" => Ok(7),
        other => Err(Error::at(
            span,
            format!(
                "unknown personality trait '{other}' (expected nice, active, playful, outgoing, or neat)"
            ),
        )),
    }
}

/// Builtins added after user helpers could already use these names: a user `fn`, `#[bhav] fn`, or
/// `extern` with the same name takes precedence.
pub(super) fn is_prim_builtin_name(name: &str) -> bool {
    PRIM_BUILTINS.iter().any(|b| b.v2 == name)
        || OP_BUILTINS.iter().any(|(n, _)| *n == name)
        || matches!(
            name,
            "animate"
                | "breakpoint"
                | "random_into"
                | "inc_lt"
                | "dec_gt"
                | "set_dynamic_sprite"
                | "generic_call"
                | "set_to_next"
                | "remove_object"
        )
}
