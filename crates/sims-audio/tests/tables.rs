//! The sound tables' parsers, on hand-written files.

use sims_audio::fsc::Score;
use sims_audio::hit::{HEADER, Operand, Program};
use sims_audio::hot::{Hot, Value, number};
use sims_audio::hsm::Symbols;
use sims_audio::module::Module;

fn header() -> Vec<u8> {
    let mut bytes = b"HIT!".to_vec();
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(8u32.to_le_bytes());
    bytes.extend(b"TRAX");
    bytes
}

/// Two routines. Track data 0x100 at 16: `loadb v1,2; cmp v1,SimSpeed;
/// ifne 0x101; end`. Track data 0x101 at 30: `note_on v1; wait_samp;
/// jump 0x100; jump v2; end`.
fn program() -> Vec<u8> {
    let mut bytes = header();
    bytes.extend([4, 5, 2]); // loadb v1, 2
    bytes.extend([20, 5, 100]); // cmp v1, SimSpeed
    bytes.push(63); // ifne
    bytes.extend(0x101u32.to_le_bytes());
    bytes.push(12); // end
    assert_eq!(bytes.len(), 28);
    bytes.extend([0, 0]); // padding, to put the second routine at 30
    bytes.extend([2, 5]); // note_on v1
    bytes.push(11); // wait_samp
    bytes.push(13); // jump 0x100
    bytes.extend(0x100u32.to_le_bytes());
    bytes.extend([13, 6]); // jump v2
    bytes.push(12); // end
    bytes
}

#[test]
fn hit_programs_decode_by_the_opcode_table() {
    let program = Program::parse(&program()).unwrap();
    assert_eq!((program.major, program.minor), (1, 8));
    let is_target = |id: u32| id == 0x100 || id == 0x101;
    let first = program.decode(HEADER as u32, &is_target).unwrap();
    assert_eq!(
        (first.name(), first.values(), first.length),
        ("loadb", vec![5, 2], 3)
    );
    let branch = program.decode(22, &is_target).unwrap();
    assert_eq!(branch.name(), "ifne");
    assert_eq!(branch.operands, [(Operand::Target, 0x101)]);

    let code = program.decode(30, &is_target).unwrap();
    assert_eq!(code.name(), "note_on");
    // Both forms of jump: a track data ID, then a register.
    let long = program.decode(33, &is_target).unwrap();
    assert_eq!(
        (long.operands[0], long.length),
        ((Operand::Target, 0x100), 5)
    );
    let short = program.decode(38, &is_target).unwrap();
    assert_eq!(
        (short.operands[0], short.length),
        ((Operand::Register, 6), 2)
    );

    // Opcode 0 (the padding) is not an instruction.
    assert!(program.decode(28, &is_target).is_err());
    assert!(program.decode(8, &is_target).is_err());
}

#[test]
fn hit_programs_reject_bad_input() {
    assert!(Program::parse(b"HIT!\x01\0\0\0\x08\0\0\0XXXX").is_err());
    let mut cut = header();
    cut.extend([5, 5, 1, 2]); // loadl v1 with three of four bytes
    let program = Program::parse(&cut).unwrap();
    assert!(program.decode(16, &|_| false).is_err());
    assert!(program.disassemble(&|_| false).is_err());
    let mut bytes = header();
    bytes.extend([12, 12]);
    let code = Program::parse(&bytes)
        .unwrap()
        .disassemble(&|_| false)
        .unwrap();
    assert_eq!(code.len(), 2);
}

const HSM: &str = "note 1 (0x1)\r\nv1 5 (0x5)\r\nSimSpeed 100 (0x64)\r\n\
                   tkd_Generic 256 (0x100)\r\nGeneric 16 (0x10)\r\n\
                   tkd_Generic 999 (0x3e7)\r\nPchRing 7 (0x7)\r\n";

#[test]
fn hsm_symbols_keep_every_line_and_find_the_first() {
    let symbols = Symbols::parse(HSM.as_bytes()).unwrap();
    assert_eq!(symbols.lines.len(), 7);
    assert_eq!(symbols.get("tkd_Generic"), Some(256));
    assert_eq!(symbols.get("SimSpeed"), Some(100));
    assert_eq!(symbols.get("simspeed"), None);
    assert!(Symbols::parse(b"lonely\n").is_err());
    assert!(Symbols::parse(b"name value\n").is_err());
}

const HOT: &str = "[Options]\r\nVersion=1\r\n\r\n\
[EventMappingEquate]\r\nkSndobPlay=1\r\nkSndobKill=4\r\n\
[TrackData]\r\n0x100=0x10\r\n0x101=0x1e\r\n\
[Patch]\r\n7=0,Ring,\"sounddata\\phone\\ring.xa\",0,0\r\n\
8=0,Step,Step2,\"sounddata\\step.xa\",0\r\n\
[GlobalHitList]\r\n20=7,8\r\n21=0x7-9, 12\r\n22=\r\n23=8,a_name,1.2\r\n\
[Track]\r\n30=0,ring,0x10,0,10,0,0,@tkd_Generic,@PchRing\r\n\
31=0,step,0x10,0,10,0,0,0x101,0\r\n\
[EventMapping]\r\nPhone_Ring=kSndobPlay,30\r\nstep=1,31,5\r\nstop=kSndobKill\r\nphone_ring=1,31\r\n";

#[test]
fn hot_tables_read_every_section() {
    let hot = Hot::parse(HOT.as_bytes()).unwrap();
    assert_eq!(hot.options["Version"], "1");
    assert_eq!(hot.equates["kSndobKill"], 4);
    assert_eq!(hot.track_data[&0x101], 0x1e);
    assert_eq!(hot.patches[&7].path, "sounddata\\phone\\ring.xa");
    assert_eq!(hot.patches[&8].path, "sounddata\\step.xa");
    assert_eq!(hot.hit_lists[&20], [7, 8]);
    assert_eq!(hot.hit_lists[&21], [7, 8, 9, 12]);
    assert!(hot.hit_lists[&22].is_empty());
    assert_eq!(hot.hit_lists[&23], [8]);
    assert_eq!(
        hot.hit_list_unread,
        [(23, "a_name".to_owned()), (23, "1.2".to_owned())]
    );
    let ring = &hot.tracks[&30];
    assert_eq!(ring.name, "ring");
    assert_eq!(ring.volume, Value::Number(16));
    assert_eq!(ring.entry, Value::Symbol("tkd_Generic".into()));
    assert_eq!(hot.events.len(), 4);
    // Lookups ignore case and the first event of a name wins.
    let event = hot.event("PHONE_RING").unwrap();
    assert_eq!(event.kind, Value::Name("kSndobPlay".into()));
    assert_eq!(event.track, Some(Value::Number(30)));
    assert_eq!(hot.event("step").unwrap().rest, [Value::Number(5)]);
    assert_eq!(hot.event("stop").unwrap().track, None);

    let symbols = Symbols::parse(HSM.as_bytes()).unwrap();
    assert_eq!(hot.number(&event.kind, &symbols), Some(1));
    assert_eq!(hot.number(&ring.patch, &symbols), Some(7));
    assert_eq!(hot.number(&Value::Name("nope".into()), &symbols), None);
}

#[test]
fn hot_tables_reject_bad_lines() {
    for bad in [
        "[Track\n",
        "[Options]\nno equals sign\n",
        "[Mystery]\na=b\n",
        "[TrackData]\n0x100=far\n",
        "[Patch]\n1=0,name,unquoted\n",
        "[Track]\n1=0,short,1,2\n",
        "[EventMappingEquate]\nk=one\n",
    ] {
        assert!(Hot::parse(bad.as_bytes()).is_err(), "{bad:?}");
    }
}

#[test]
fn numbers_read_decimal_hex_and_signs() {
    assert_eq!(number("42"), Some(42));
    assert_eq!(number(" 0x1F "), Some(31));
    assert_eq!(number("-7"), Some(-7));
    assert_eq!(number("-0x10"), Some(-16));
    assert_eq!(number(""), None);
    assert_eq!(number("1.2"), None);
    assert_eq!(number("0x"), None);
}

#[test]
fn modules_resolve_tracks_through_symbols() {
    let module = Module::parse("test", &program(), HSM.as_bytes(), HOT.as_bytes()).unwrap();
    assert_eq!(module.entry(30), Some(0x100));
    assert_eq!(module.entry(31), Some(0x101));
    assert_eq!(module.address(0x101), Some(30));
    assert!(module.is_target(0x100) && !module.is_target(0x10));
    let code = module.program.disassemble(&|id| module.is_target(id));
    // The padding at 28 is not an instruction, so a linear walk fails.
    assert!(code.is_err());
}

fn score(cells: &[&str]) -> String {
    let mut text = String::from("version\t7\n# vol\tpri\tmin\tmax\trows\tcols\n");
    text.push_str("\t100\t5\t1\t4\t1\t2\t120\n# cell fields\n");
    for cell in cells {
        text.push_str(cell);
        text.push('\n');
    }
    text
}

#[test]
fn fsc_scores_read_the_score_and_its_cells() {
    let full = "\t1\t2\t3\t4\t5\t6\t7\t8\t9\t10\t11\t12\t13\t14\t15\t16\t17\t18\tbird.xa\t0";
    let empty = "\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\tNONE";
    let parsed = Score::parse(score(&[full, empty]).as_bytes()).unwrap();
    assert_eq!(parsed.version, 7);
    assert_eq!(parsed.fields, [100, 5, 1, 4, 1, 2, 120]);
    assert_eq!((parsed.rows, parsed.columns), (1, 2));
    let cell = parsed.cells[0].as_ref().unwrap();
    assert_eq!(cell.file, "bird.xa");
    assert_eq!(cell.fields, (1..=18).collect::<Vec<i64>>());
    assert_eq!(parsed.cells[1], None);

    assert!(
        Score::parse(score(&[full]).as_bytes()).is_err(),
        "a cell short"
    );
    assert!(Score::parse(b"vers 7\n").is_err());
    assert!(Score::parse(score(&[full, "\t1\t2\tbird.xa"]).as_bytes()).is_err());
}
