//! Capability descriptions for inventory, deliberately separate from execution coverage.
//! Reference names: FreeSO VMContext at 4c6b3e8f5835b228723caea3c9f683c62f244f73,
//! TS1 registrations. Native dispatch addresses/acceptance are independently probed.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PrimitiveSupport {
    pub opcode: u16,
    pub name: &'static str,
    pub rust_support: &'static str,
    pub limits: &'static str,
}
pub fn primitive(opcode: u16) -> PrimitiveSupport {
    let name = match opcode {
        0 => "sleep",
        1 => "generic Sims call",
        2 => "expression",
        3 => "find best interaction",
        4 => "grab",
        5 => "drop",
        6 => "change suit/accessory",
        7 => "refresh",
        8 => "random number",
        9 => "burn",
        10 => "UI-event ownership (native research label)",
        11 => "get distance to",
        12 => "get direction to",
        13 => "push interaction",
        14 => "find best object for function",
        15 => "breakpoint",
        16 => "find location for",
        17 => "idle for input",
        18 => "remove object instance",
        19 => "make new character",
        20 => "run functional tree",
        21 => "show string",
        22 => "look towards",
        23 => "play sound",
        24 => "old relationship",
        25 => "budget",
        26 => "relationship",
        27 => "go to relative position",
        28 => "run tree by name",
        29 => "set motive deltas",
        30 => "gosub found action",
        31 => "set to next",
        32 => "test object type",
        33 => "find five worst motives (reference label)",
        34 => "UI effect",
        35 => "special effect",
        36 => "dialog private",
        37 => "test Sim interacting with",
        41 => "set balloon headline",
        42 => "create object instance",
        43 => "drop onto",
        44 => "animate Sim",
        45 => "go to routing slot",
        46 => "snap",
        47 => "reach",
        48 => "stop all sounds",
        49 => "notify out of idle",
        50 => "change action string",
        51 => "manage inventory",
        _ => "unidentified/unassigned",
    };
    let (rust_support, limits) = match opcode {
        33 => (
            "partial",
            "statically recovered synchronous executor/stack-person selection, raw motive ranking with native shortsort ties, and five caller temporaries; accepted operand words and native error order retained; isolated original-handler parity not executed",
        ),
        34 => (
            "partial",
            "native operand reads, ordered synchronous control/portrait/relationship calls, recursive widget lookup and highlight placement compared in 1894 cases; saved host boundaries and original tutorial control bindings; positive portrait/relationship hosts, application wait-event delivery and highlight animation remain unsupported",
        ),
        9 => (
            "partial",
            "native selection/control flow and live base fire factory/placement/cleanup; ordinary person Reset and nested shared-vector callbacks remain explicit boundaries",
        ),
        10 => (
            "partial",
            "native acquire/release/invalid-mode and signal ordering validated; private-dialog closes applied synchronously; original widget/event subscribers remain a presentation boundary",
        ),
        36 => (
            "partial",
            "native result/yield protocol compared in 840 cases; modes 0..2 and tutorial mode 4 use STR#301, catalog names, $Local, native button defaults, explicit responses and saved registry UI suspension; constructor flags, button selection and widget key/settings requests compared in 325 additional cases; resource icon selection compared in 768 cases: selector 0 uses formatted/live resources, 2 constructs a saved neighbor portrait, 1/5..7 select no icon; named job/school icons use the original atlas and selector 3 requests BMP_ from the frame resource; text-input mode 3, widget timer/panel behavior and other substitutions remain boundaries",
        ),
        0 => (
            "partial",
            "native Sleep counters and ordinary active-object scheduler visits validated; pause modes, full factories and full lot scheduling remain gaps",
        ),
        1 => (
            "partial",
            "native active-action icon owner 2, fire registration/unregistration 10/11, camera-relative tile depth 12, lot zoning 28 and temporary footprint request fields 15; original fire UI subscribers, rotated depth outside the native 64x64 table, actual footprint extension and other subcalls unsupported",
        ),
        3 => (
            "partial",
            "native residential gates, cached advertisement registry/load-reset tails, zoning scan policy, person menus/active-action joins, eligibility callbacks, all 23 advertisement modifiers and selection; original or authored positions; nonresidential permissions unsupported",
        ),
        14 => (
            "partial",
            "native selector/filter/score/distance search including synchronous condition callbacks, caller score register and Temp propagation; supplied registry/positions; indoor/outdoor filters and registry mutation during checks unsupported",
        ),
        4 | 5 => (
            "partial",
            "native Grab/Drop target selection; bounded ordinary-object placement; full placement rules/subclasses/callbacks unsupported",
        ),
        6 => (
            "partial",
            "native global/private STR#304 accessory lookup, ASCII case conversion and add/remove selection; ordered loaded appearance layers saved; built-in outfit fields and random variants validated; arbitrary imported-person skins and appearance load failures unsupported",
        ),
        16 => (
            "partial",
            "native local-reference spiral/fallback, directional modes 3/4 with final recheck, mode-one ordinary detachment; imported virtual base/person/multipart acceptance and ordered placement with saved callbacks; camera-relative/random modes and multipart detachment splitting remain boundaries",
        ),
        18 => (
            "partial",
            "native ordinary-object physical detachment, synchronous placement callbacks, ancestor/duplicate suppression and deferred queue; self-yield and child-first destruction with saved callback/slot continuations; person/multiple-part/special-class deletion, live animator clearing and dependent-frame resets unsupported",
        ),
        20 => (
            "partial",
            "native OBJf selector, object/person child frames, use counts and icon flags/return; synchronous check use and icon selection without an active action remain unsupported",
        ),
        24 | 26 => (
            "partial",
            "native directed object/neighbor tables, legacy argument operands, locals, scope resolution, growth and dirty notifications; family/neighbor import and all deletion side effects remain gaps",
        ),
        22 => (
            "partial",
            "native head-seek fields and standing modes 1–3; adult turn selection, clock and pose validated; head-bone seeking, carried objects and other profiles unsupported",
        ),
        42 => (
            "partial",
            "single-tile slot-zero/off-lot, in-front modes 0/3, caller/argument/local and relative-floor modes, occupied-tile flag, Init/immediate Main, saved placement and failure cleanup; native multipart off-lot factory registers all parts before ordered Init/Main, preserves pre-Init catalog price, and runs bounded original group rotation; other group placement/subclass callbacks remain explicit boundaries",
        ),
        41 => (
            "partial",
            "native sprite and portrait headline state, priority, flags, lifetime, catalog/neighbor/raw-local resource selection; portrait wrapper statically recovered with fresh native probe pending; visible balloon compositing unsupported",
        ),
        27 => (
            "partial",
            "native relative descriptor decoding; shared Stand/route host and native standing candidates; bounded on-point routes and original door callbacks; terrain flags and native path search remain unsupported",
        ),
        28 => (
            "partial",
            "native yielding mode 2 and synchronous mode 0 named callbacks, temp arguments, reused frame storage and person use records; global STR#303; mode 1 and non-ASCII directory rules remain gaps",
        ),
        29 => (
            "partial",
            "native ordered append/clear and operand decoding validated; applied by the person update service",
        ),
        30 => (
            "partial",
            "native found-action target/menu binding, ordinary child frame, priority and reset; broader factory/context cases remain gaps",
        ),
        37 => (
            "partial",
            "native use-record scan across live action/functional frames; mode 1 resolves another person, other mode bytes use the caller; exact original suspended-stack import remains separate",
        ),
        31 => (
            "partial",
            "native object/person/non-person/neighbor/category, cyclic same-family person and adjacent-tile iteration, multi-tile next/head links, lazy numeric-ID resource-instance index with append/delete maintenance, tile wrap and supplied career-vector iteration; career file loading and other special modes unsupported",
        ),
        51 => (
            "partial",
            "native inventory remove-by-key operation 1 and query operation 3, including signed quantities/counts, first-match ordering and UI dirty publication; other operations unsupported",
        ),
        2 => (
            "partial",
            "native-validated integer operators; integer and common float-motive operators validated; external scopes, side effects and exceptional x87 paths remain gaps",
        ),
        7 => (
            "partial",
            "native drawing/room invalidation modes and F1/F2/F3 Commander effects; full-viewport redraw replaces rectangle invalidation; room scoring consumer absent",
        ),
        8 => (
            "partial",
            "original 32-bit state/32-bit output stream and operand order validated; unsupported memory scopes and world-side effects remain gaps",
        ),
        13 => (
            "partial",
            "native queue insertion, priority/icon operands, table lookup and immediate action/check callbacks with temp guards; function-4 cancellation callbacks run on the target frame arena; nested immediate/cancellation callbacks and cancellation during deletion remain bounded",
        ),
        17 => (
            "partial",
            "native idle gates, timer, eligibility callback, action frames and return continuation validated; full autonomy and visitor permission services remain gaps",
        ),
        23 => (
            "partial",
            "native current-resource/global FWAV fallback and missing-name success; queued presentation events, no audio playback",
        ),
        35 => (
            "partial",
            "native three-phase redraw/effect continuation, signed duration and local lookup; queued effects, text mode and particle rendering unsupported",
        ),
        43 => (
            "partial",
            "native Drop Onto literal/parameter slot decoding and source-slot lookup; bounded ordinary containment transfer",
        ),
        25 => (
            "partial",
            "native lot budget checks, debit/credit, categories, mode and signal queue; family import and signal consumers absent",
        ),
        50 => (
            "partial",
            "native dynamic menu entries and active-action renaming through the shared name/local/time/career formatter; neighbor and other formatter families remain unsupported",
        ),
        47 => (
            "partial",
            "native Reach continuation and height selection; xevt-zero timed pickup/put and synchronous CT - Grab/Put callbacks through bounded placement; person bone slot heights and full placement rules unsupported",
        ),
        11 | 12 => (
            "partial",
            "native distance/direction decisions with supplied object coordinates and writable VM destinations",
        ),
        48 => (
            "partial",
            "native source selection and ordered mixer stop commands; no audio playback",
        ),
        32 => (
            "partial",
            "native own/catalog-parent GUID comparison and missing-object error; linked private catalog resources",
        ),
        44 => (
            "partial",
            "adult foreground and eventless background clips, numbered/independent events, missing-event substitution and retained completion pose; household catalog and default posture; pickup start, automatic and explicit script carry layers, timed property dispatch; clock still approximate, general blending and profiles unsupported",
        ),
        45 => (
            "partial",
            "native standing/seated candidate scores and random cap, current-seat reuse, Sit/Stand services, failure feedback and retry after eight-subtile progress; host path search/classification; full shoo/wait service unsupported",
        ),
        46 => (
            "partial",
            "SLOT containment/exit plus native first-slot and front destination decoding; authored placement, no general footprint or shoo",
        ),
        49 => (
            "partial",
            "native ordinary-frame inspection below supported callback boundaries and argument-zero reset; complete person/lot scheduling remains a gap",
        ),
        _ => (
            "unsupported",
            "VM faults explicitly; native handler presence does not establish Rust support",
        ),
    };
    PrimitiveSupport {
        opcode,
        name,
        rust_support,
        limits,
    }
}

pub fn read_scope_supported(scope: u8) -> bool {
    matches!(scope, 0 | 1 | 3 | 4 | 6..=32 | 34..=37)
}
pub fn write_scope_supported(scope: u8) -> bool {
    matches!(scope,0|1|3|4|6|8..=15|17..=19|22|25|27..=31|36|37)
}
