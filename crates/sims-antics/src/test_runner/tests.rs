//! Tests for the spec parser and, under `vm-replay`, the harness.

use super::*;

const HEAD: &str = "[subject]\nantics = \"obj.antics\"\n";

fn parse(text: &str) -> Result<TestFile> {
    TestFile::parse(text, Path::new("/specs"))
}

fn parse_ok(text: &str) -> TestFile {
    parse(text).unwrap_or_else(|e| panic!("{e}\n---\n{text}"))
}

/// The error a spec gives, which must contain every fragment.
fn parse_err(text: &str, fragments: &[&str]) {
    let e = match parse(text) {
        Ok(f) => panic!("parsed:\n{text}\n{f:#?}"),
        Err(e) => e.to_string(),
    };
    for f in fragments {
        assert!(e.contains(f), "`{e}` lacks `{f}`\n---\n{text}");
    }
}

/// A one-case file around `case` (the `[[test]]` body).
fn one(case: &str) -> String {
    format!("{HEAD}[sim]\n\n[[test]]\nname = \"t\"\n{case}\n")
}

fn one_case(case: &str) -> TestCase {
    parse_ok(&one(case)).cases.remove(0)
}

// ---------------------------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------------------------

#[test]
fn parses_the_module_doc_example() {
    let text = r#"
[subject]
antics = "../objects/open-tv.antics"
pack = ".."

[sim]
motives = { fun = -20 }

[[test]]
name = "turning the set on clicks"
action = "Turn On"
given = { attrs = { power = 0 } }
expect = { result = true, attrs = { power = 1 }, sounds = ["open_tv_click"] }
"#;
    let f = TestFile::parse(text, Path::new("/pack/tests")).unwrap();
    assert_eq!(
        f.subject,
        Source::Antics {
            path: PathBuf::from("/pack/tests/../objects/open-tv.antics"),
            pack: Some(PathBuf::from("/pack/tests/..")),
        }
    );
    assert_eq!(f.subject.slug(), "open-tv");
    let sim = f.sim.as_ref().unwrap();
    assert_eq!(sim.motives.get(&15), Some(&-20.0));
    let c = &f.cases[0];
    assert_eq!(c.name, "turning the set on clicks");
    assert_eq!(c.target, Target::Action("Turn On".into()));
    assert_eq!(c.budget, DEFAULT_BUDGET);
    assert_eq!(c.turns, None);
    assert_eq!(c.given.attrs.get(&AttrKey::Name("power".into())), Some(&0));
    assert_eq!(c.expect.result, Some(Outcome::True));
    assert_eq!(c.expect.attrs.get(&AttrKey::Name("power".into())), Some(&1));
    assert_eq!(c.expect.sounds, Some(vec!["open_tv_click".to_owned()]));
}

#[test]
fn iff_subjects_and_also_sources() {
    let f = parse_ok(
        "[subject]\niff = \"a.iff\"\n\n[lot]\nalso = [\"b.iff\", \"c.antics\", { antics = \"d.antics\", pack = \"p\" }]\n\n[[test]]\nname = \"t\"\ntree = 4096\n",
    );
    assert_eq!(
        f.subject,
        Source::Iff {
            path: "/specs/a.iff".into()
        }
    );
    assert_eq!(
        f.also,
        vec![
            Source::Iff {
                path: "/specs/b.iff".into()
            },
            // A bare `.antics` path shares the subject's pack (an `.iff` subject has none).
            Source::Antics {
                path: "/specs/c.antics".into(),
                pack: None
            },
            Source::Antics {
                path: "/specs/d.antics".into(),
                pack: Some("/specs/p".into())
            },
        ]
    );
}

#[test]
fn also_antics_inherits_the_subject_pack() {
    let f = parse_ok(
        "[subject]\nantics = \"a.antics\"\npack = \"..\"\n\n[lot]\nalso = [\"b.antics\"]\n\n[[test]]\nname = \"t\"\ntree = 4096\n",
    );
    assert_eq!(
        f.also,
        vec![Source::Antics {
            path: "/specs/b.antics".into(),
            pack: Some("/specs/..".into())
        }]
    );
}

#[test]
fn absolute_paths_stay_absolute() {
    let f = parse_ok("[subject]\nantics = \"/x/obj.antics\"\n\n[[test]]\nname = \"t\"\ntree = 1\n");
    assert_eq!(f.subject.path(), Path::new("/x/obj.antics"));
}

#[test]
fn file_level_errors() {
    parse_err("[[test]]\nname = \"t\"\ntree = 1\n", &["[subject]"]);
    parse_err(HEAD, &["no [[test]]"]);
    parse_err(
        "[subject]\nantics = \"a.antics\"\niff = \"b.iff\"\n\n[[test]]\nname = \"t\"\ntree = 1\n",
        &["[subject]", "not both"],
    );
    parse_err(
        "[subject]\nfile = \"a.antics\"\n\n[[test]]\nname = \"t\"\ntree = 1\n",
        &["unknown key 'file'"],
    );
    parse_err(
        "[subject]\niff = \"a.iff\"\npack = \"..\"\n\n[[test]]\nname = \"t\"\ntree = 1\n",
        &["pack"],
    );
    parse_err(
        &format!("{HEAD}[extra]\nx = 1\n\n[[test]]\nname = \"t\"\ntree = 1\n"),
        &["unknown key 'extra'"],
    );
    parse_err(
        &format!("{HEAD}[[test]]\nname = \"t\"\ntree = 1\n\n[[test]]\nname = \"t\"\ntree = 2\n"),
        &["two tests are named 't'"],
    );
    parse_err(
        &format!("{HEAD}[lot]\nalso = [\"../x/obj.iff\"]\n\n[[test]]\nname = \"t\"\ntree = 1\n"),
        &["share the name 'obj'"],
    );
    parse_err(
        &format!("{HEAD}[lot]\nalso = [3]\n\n[[test]]\nname = \"t\"\ntree = 1\n"),
        &["path or a table"],
    );
    parse_err("[subject\nantics = \"a\"\n", &[]);
}

#[test]
fn idle_main_keys() {
    let f = parse_ok(&format!(
        "{HEAD}idle_main = true

[lot]
also = [\"a.antics\", {{ antics = \"b.antics\", idle_main = true }}, {{ antics = \"c.antics\", idle_main = false }}]

[[test]]
name = \"t\"
tree = 1
"
    ));
    assert!(f.idle_main);
    assert_eq!(f.also.len(), 3);
    assert_eq!(f.also_idle_main, BTreeSet::from(["b".to_owned()]));
    let f = parse_ok(&format!("{HEAD}\n[[test]]\nname = \"t\"\ntree = 1\n"));
    assert!(!f.idle_main);
    assert!(f.also_idle_main.is_empty());
    parse_err(
        &format!("{HEAD}idle_main = 1\n\n[[test]]\nname = \"t\"\ntree = 1\n"),
        &["[subject].idle_main must be a boolean"],
    );
    parse_err(
        &format!(
            "{HEAD}\n[lot]\nalso = [{{ antics = \"b.antics\", idle_main = \"yes\" }}]\n\n[[test]]\nname = \"t\"\ntree = 1\n"
        ),
        &["[lot].also[0].idle_main must be a boolean"],
    );
}

// ---------------------------------------------------------------------------------------
// Stubs
// ---------------------------------------------------------------------------------------

#[test]
fn stubs() {
    let f = parse_ok(&format!(
        "{HEAD}[[stub]]\nid = 300\nreturns = true\n\n[[stub]]\nid = 8200\nlabel = \"Semi\"\nargs = 2\nreturns = false\n\n[[stub]]\nsource = \"tree 301 \\\"R\\\" {{ return true; }}\"\n\n[[test]]\nname = \"t\"\ntree = 1\n"
    ));
    assert_eq!(
        f.stubs[0],
        Stub::Returns {
            id: 300,
            label: "Stub300".into(),
            args: 0,
            result: true
        }
    );
    assert_eq!(
        f.stubs[1],
        Stub::Returns {
            id: 8200,
            label: "Semi".into(),
            args: 2,
            result: false
        }
    );
    match &f.stubs[2] {
        Stub::Source { text, .. } => assert!(text.contains("tree 301")),
        other => panic!("{other:?}"),
    }
    assert_eq!(f.stubs.len(), 3);
    // A stub file is read with the spec.
    parse_err(
        &format!("{HEAD}[[stub]]\nfile = \"stubs.air\"\n\n[[test]]\nname = \"t\"\ntree = 1\n"),
        &["stubs.air"],
    );
}

#[test]
fn stub_errors() {
    let with = |stub: &str| format!("{HEAD}[[stub]]\n{stub}\n\n[[test]]\nname = \"t\"\ntree = 1\n");
    parse_err(&with("returns = true"), &["needs an 'id'"]);
    parse_err(&with("id = 5000\nreturns = true"), &["5000", "global"]);
    parse_err(&with("id = 100\nreturns = true"), &["100", "global"]);
    parse_err(
        &with("id = 300\nreturns = 1"),
        &["returns must be a boolean"],
    );
    parse_err(&with("id = 300\nargs = 5\nreturns = true"), &["args = 5"]);
    parse_err(
        &with("id = 300\nlabel = \"a\\\"b\"\nreturns = true"),
        &["quotes"],
    );
    parse_err(&with("id = 300"), &["returns"]);
    parse_err(
        &with("source = \"tree 300 \\\"x\\\" { return true; }\"\nid = 300"),
        &["'id' applies to a 'returns' stub"],
    );
    parse_err(&with("bogus = 1"), &["unknown key 'bogus'"]);
}

#[test]
fn stub_range_is_global_or_semi_global() {
    assert!(!stub_range(255));
    assert!(stub_range(256));
    assert!(stub_range(4095));
    assert!(!stub_range(4096));
    assert!(!stub_range(8191));
    assert!(stub_range(8192));
    assert!(stub_range(u16::MAX));
}

// ---------------------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------------------

#[test]
fn targets() {
    assert_eq!(
        one_case("tree = 4096").target,
        Target::Tree(TreeRef::Id(4096))
    );
    assert_eq!(
        one_case("tree = \"Main\"").target,
        Target::Tree(TreeRef::Label("Main".into()))
    );
    assert_eq!(
        one_case("guard = \"Watch TV\"").target,
        Target::Guard("Watch TV".into())
    );
    assert_eq!(
        one_case("action = \"Watch TV\"").target,
        Target::Action("Watch TV".into())
    );
    let c = one_case("interaction = \"Watch TV\"\nturns = 9\nanswers = [1, 3]");
    assert_eq!(c.target, Target::Interaction("Watch TV".into()));
    assert_eq!(c.turns, Some(9));
    assert_eq!(c.answers, vec![1, 3]);
}

#[test]
fn target_errors() {
    parse_err(&one(""), &["one of"]);
    parse_err(&one("tree = 1\nguard = \"x\""), &["one of"]);
    parse_err(&one("tree = -1"), &["tree"]);
    parse_err(&one("tree = true"), &["tree"]);
    parse_err(&one("guard = 1"), &["string"]);
    // A case without a name is numbered.
    assert_eq!(
        parse_ok(&format!("{HEAD}[[test]]\ntree = 1\n")).cases[0].name,
        "test #1"
    );
    parse_err(&one("tree = 1\nbogus = 2"), &["unknown key 'bogus'"]);
}

#[test]
fn run_settings() {
    let c = one_case(
        "tree = 1\nme = \"sim\"\nstack_obj = \"none\"\nparams = [1, -2]\nturns = 3\nbudget = 500",
    );
    assert_eq!(c.me, Some(Actor::Sim));
    assert_eq!(c.stack_obj, Some(ObjRef::None));
    assert_eq!(c.params, vec![1, -2]);
    assert_eq!(c.turns, Some(3));
    assert_eq!(c.budget, 500);
    for (text, want) in [
        ("\"object\"", ObjRef::Object),
        ("\"self\"", ObjRef::Object),
        ("\"me\"", ObjRef::Object),
        ("\"sim\"", ObjRef::Sim),
        ("7", ObjRef::Id(7)),
    ] {
        assert_eq!(
            one_case(&format!("tree = 1\nstack_obj = {text}")).stack_obj,
            Some(want),
            "{text}"
        );
    }
}

#[test]
fn run_setting_errors() {
    parse_err(&one("tree = 1\nme = \"stove\""), &["me"]);
    parse_err(&one("tree = 1\nstack_obj = \"stove\""), &["stack_obj"]);
    parse_err(&one("tree = 1\nparams = [70000]"), &["16-bit"]);
    parse_err(&one("tree = 1\nturns = 0"), &["turns"]);
    parse_err(
        &one(&format!("tree = 1\nturns = {}", MAX_TURNS + 1)),
        &["turns"],
    );
    parse_err(&one("tree = 1\nbudget = 0"), &["budget"]);
    parse_err(&one("tree = 1\nanswers = [4]"), &["answers"]);
    parse_err(&one("tree = 1\nanswers = [0]"), &["answers"]);
}

#[test]
fn interaction_case_restrictions() {
    let i = "interaction = \"X\"\n";
    parse_err(&one(&format!("{i}me = \"sim\"")), &["me"]);
    parse_err(&one(&format!("{i}stack_obj = \"sim\"")), &["stack_obj"]);
    parse_err(&one(&format!("{i}params = [1]")), &["params"]);
    parse_err(
        &one(&format!("{i}expect = {{ result = true }}")),
        &["result"],
    );
    parse_err(
        &one(&format!("{i}expect = {{ stack_obj = \"sim\" }}")),
        &["stack_obj"],
    );
    // An interaction ends ok, unavailable, yield or error.
    for r in ["ok", "unavailable", "yield", "error"] {
        let c = one_case(&format!("{i}expect = {{ result = \"{r}\" }}"));
        assert_eq!(c.expect.result.map(|o| o.to_string()), Some(r.to_owned()));
    }
    // `unavailable` is for interactions only.
    parse_err(
        &one("tree = 1\nexpect = { result = \"unavailable\" }"),
        &["unavailable"],
    );
}

#[test]
fn sim_requirements() {
    let no_sim = |case: &str| format!("{HEAD}[[test]]\nname = \"t\"\n{case}\n");
    parse_err(&no_sim("interaction = \"X\""), &["Sim"]);
    parse_err(&no_sim("tree = 1\nme = \"sim\""), &["Sim"]);
    parse_err(&no_sim("tree = 1\nstack_obj = \"sim\""), &["Sim"]);
    parse_err(
        &no_sim("tree = 1\nexpect = { sim = { motives = { fun = 1 } } }"),
        &["Sim"],
    );
    parse_err(&no_sim("tree = 1\nexpect = { queue = 0 }"), &["Sim"]);
    // A case may bring its own Sim.
    let f = parse_ok(&no_sim(
        "interaction = \"X\"\ngiven = { sim = { motives = { fun = 1 } } }",
    ));
    assert!(f.cases[0].has_sim(&f));
    assert!(!parse_ok(&no_sim("tree = 1")).cases[0].has_sim(&parse_ok(&no_sim("tree = 1"))));
}

#[test]
fn sim_state_keys_and_merging() {
    let f = parse_ok(&format!(
        "{HEAD}[sim]\nmotives = {{ hunger = -10, fun = 20.5, 2 = 3 }}\nskills = {{ cooking = 300 }}\npersonality = {{ nice = 700 }}\nage = 20\npriority = 50\nperson = {{ 100 = 1 }}\nattrs = {{ 0 = 4 }}\ntemps = {{ 1 = 2 }}\ndata = {{ 3 = 9 }}\n\n[[test]]\nname = \"t\"\ntree = 1\ngiven = {{ sim = {{ motives = {{ fun = 1 }}, person = {{ 100 = 2 }} }} }}\n"
    ));
    let base = f.sim.as_ref().unwrap();
    assert_eq!(base.motives.get(&7), Some(&-10.0));
    assert_eq!(base.motives.get(&15), Some(&20.5));
    assert_eq!(base.motives.get(&2), Some(&3.0));
    assert_eq!(base.person.get(&10), Some(&300), "cooking");
    assert_eq!(base.person.get(&2), Some(&700), "nice");
    assert_eq!(base.person.get(&58), Some(&20), "age");
    assert_eq!(base.person.get(&33), Some(&50), "priority");
    assert_eq!(base.attrs.get(&0), Some(&4));
    assert_eq!(base.temps.get(&1), Some(&2));
    assert_eq!(base.data.get(&3), Some(&9));
    let merged = f.cases[0].sim_state(&f).unwrap();
    assert_eq!(merged.motives.get(&15), Some(&1.0), "the case's value wins");
    assert_eq!(merged.motives.get(&7), Some(&-10.0), "the file's is kept");
    assert_eq!(merged.person.get(&100), Some(&2));
    assert_eq!(merged.person.get(&10), Some(&300));
}

#[test]
fn sim_state_errors() {
    let sim = |body: &str| format!("{HEAD}[sim]\n{body}\n\n[[test]]\nname = \"t\"\ntree = 1\n");
    parse_err(&sim("motives = { thirst = 1 }"), &["thirst"]);
    parse_err(&sim("motives = { 16 = 1 }"), &["16"]);
    parse_err(&sim("motives = { fun = \"x\" }"), &["number"]);
    parse_err(&sim("skills = { juggling = 1 }"), &["juggling"]);
    parse_err(&sim("personality = { grumpy = 1 }"), &["grumpy"]);
    parse_err(&sim("mood = 3"), &["unknown key 'mood'"]);
    parse_err(&sim("person = { x = 1 }"), &["index"]);
    parse_err(&sim("motives = 4"), &["table"]);
}

#[test]
fn one_key_two_spellings_is_refused() {
    // A table keeps one value per canonical key: the later spelling would silently drop
    // the earlier one's assertion (`15` and `fun` are both motive 15).
    parse_err(
        &one("tree = 1\nexpect = { sim = { motives = { 15 = 10, fun = 0 } } }"),
        &["'15' and 'fun' both set motive 15"],
    );
    parse_err(
        &one("tree = 1\ngiven = { sim = { motives = { 13 = 1, room = 2 } } }"),
        &["motive 13"],
    );
    parse_err(
        &one("tree = 1\nexpect = { data = { 0 = 1, 00 = 2 } }"),
        &["both set word 0"],
    );
    parse_err(
        &one("tree = 1\ngiven = { globals = { 7 = 1, 007 = 2 } }"),
        &["both set word 7"],
    );
    parse_err(
        &one("tree = 1\nexpect = { attrs = { 1 = 1, 01 = 2 } }"),
        &["both set attribute 1"],
    );
    parse_err(
        &one("tree = 1\nexpect = { sim = { temps = { 2 = 1, 02 = 1 } } }"),
        &["both set word 2"],
    );
    // Named and raw person fields.
    parse_err(
        &one("tree = 1\nexpect = { sim = { age = 20, person = { 58 = 30 } } }"),
        &["'person.58' and 'age' both set person word 58"],
    );
    parse_err(
        &one("tree = 1\ngiven = { sim = { skills = { cooking = 1 }, person = { cooking = 2 } } }"),
        &["person word 10"],
    );
    parse_err(
        &one("tree = 1\ngiven = { sim = { priority = 1, person = { 33 = 2 } } }"),
        &["person word 33"],
    );
    parse_err(
        &one("tree = 1\ngiven = { sim = { personality = { nice = 1 }, person = { 2 = 2 } } }"),
        &["person word 2"],
    );
    parse_err(
        &format!(
            "{HEAD}[sim]\nmotives = {{ 7 = 1, hunger = 2 }}\n\n[[test]]\nname = \"t\"\ntree = 1\n"
        ),
        &["[sim].motives", "motive 7"],
    );
    // A case may still override the file's Sim, key by key.
    let f = parse_ok(&format!(
        "{HEAD}[sim]\nmotives = {{ fun = 1 }}\nage = 20\n\n[[test]]\nname = \"t\"\ntree = 1\ngiven = {{ sim = {{ motives = {{ 15 = 2 }}, person = {{ 58 = 30 }} }} }}\n"
    ));
    let merged = f.cases[0].sim_state(&f).unwrap();
    assert_eq!(merged.motives.get(&15), Some(&2.0));
    assert_eq!(merged.person.get(&58), Some(&30));
}

#[test]
fn room_may_be_asserted_after_any_run() {
    // The lot updates between turns run the Sim's person tick, which sets Room from the
    // room's score: what a run leaves is the game's, on any thread.
    one_case("interaction = \"X\"\nexpect = { sim = { motives = { room = 20 } } }");
    one_case("action = \"X\"\nexpect = { sim = { motives = { 13 = 20 } } }");
    one_case("tree = 1\nme = \"sim\"\nexpect = { sim = { motives = { room = 20 } } }");
}

#[cfg(unix)]
#[test]
fn spec_inputs_must_be_regular_files() {
    let dir = std::env::temp_dir().join(format!(
        "sims-antics-test-runner-{}-fifo",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let fifo = dir.join("stubs.air");
    if !fifo.exists() {
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo");
        assert!(made.success());
    }
    // Opening the FIFO would block; the parser refuses it first, naming it.
    let e = TestFile::parse(
        "[subject]\nantics = \"obj.antics\"\n\n[[stub]]\nfile = \"stubs.air\"\n\n[[test]]\nname = \"t\"\ntree = 1\n",
        &dir,
    )
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("stubs.air") && e.contains("not a regular file"),
        "{e}"
    );
    let e = TestFile::read(&fifo).unwrap_err().to_string();
    assert!(e.contains("not a regular file"), "{e}");
    // A directory is not one either.
    let e = TestFile::read(&dir).unwrap_err().to_string();
    assert!(e.contains("not a regular file"), "{e}");
    // A FIFO named on the command line is neither a spec nor a directory of them.
    let e = collect_test_files(&fifo).unwrap_err().to_string();
    assert!(e.contains("not a regular file"), "{e}");
    // The open itself refuses a FIFO (here with no writer) without blocking, whatever the
    // path check before it saw: one swapped in after the check cannot hang the run.
    let (tx, rx) = std::sync::mpsc::channel();
    let path = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(open_checked(&path).map(|_| ()));
    });
    let e = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("opening a FIFO blocked")
        .unwrap_err()
        .to_string();
    assert!(e.contains("not a regular file"), "{e}");
}

#[test]
fn givens() {
    let c = one_case(
        "tree = 1\ngiven = { attrs = { 0 = 1, power = -1 }, temps = { 2 = 3 }, data = { 4 = 5 }, globals = { 6 = 7 }, funds = 1234, create = [\"open-fridge\", 0x7F000020] }",
    );
    let g = &c.given;
    assert_eq!(g.attrs.get(&AttrKey::Index(0)), Some(&1));
    assert_eq!(g.attrs.get(&AttrKey::Name("power".into())), Some(&-1));
    assert_eq!(g.temps.get(&2), Some(&3));
    assert_eq!(g.data.get(&4), Some(&5));
    assert_eq!(g.globals.get(&6), Some(&7));
    assert_eq!(g.funds, Some(1234));
    assert_eq!(
        g.create,
        vec![
            CreateRef::Slug("open-fridge".into()),
            CreateRef::Guid(0x7F00_0020)
        ]
    );
}

#[test]
fn given_errors() {
    parse_err(&one("tree = 1\ngiven = { funds = -1 }"), &["funds"]);
    parse_err(&one("tree = 1\ngiven = { funds = 100000000 }"), &["funds"]);
    parse_err(
        &one("tree = 1\ngiven = { attrs = { 0 = 40000 } }"),
        &["16-bit"],
    );
    parse_err(&one("tree = 1\ngiven = { temps = { a = 1 } }"), &["index"]);
    parse_err(&one("tree = 1\ngiven = { create = [true] }"), &["create"]);
    parse_err(&one("tree = 1\ngiven = { create = [-1] }"), &["create"]);
    parse_err(
        &one("tree = 1\ngiven = { bogus = 1 }"),
        &["unknown key 'bogus'"],
    );
    parse_err(&one("tree = 1\ngiven = 3"), &["table"]);
}

#[test]
fn expectations() {
    let c = one_case(
        "tree = 1\nme = \"sim\"\nturns = 4\nexpect = { result = false, attrs = { 1 = 2 }, temps = { 0 = 1 }, data = { 3 = 4 }, globals = { 5 = 6 }, funds = 7, stack_obj = \"sim\", sounds = [], dialogs = [\"ok\", \"yes_no\", \"yes_no_cancel\", \"text\", 9], queue = [{ interaction = \"Eat\", target = \"object\", priority = 50 }, { menu = 2 }, {}], turns = 3, sim = { motives = { hunger = 50 } } }",
    );
    let e = &c.expect;
    assert_eq!(e.result, Some(Outcome::False));
    assert_eq!(e.attrs.get(&AttrKey::Index(1)), Some(&2));
    assert_eq!(e.temps.get(&0), Some(&1));
    assert_eq!(e.data.get(&3), Some(&4));
    assert_eq!(e.globals.get(&5), Some(&6));
    assert_eq!(e.funds, Some(7));
    assert_eq!(e.stack_obj, Some(ObjRef::Sim));
    assert_eq!(e.sounds, Some(vec![]));
    assert_eq!(
        e.dialogs
            .as_ref()
            .unwrap()
            .iter()
            .map(|d| d.kind)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 9]
    );
    let q = e.queue.as_ref().unwrap();
    assert_eq!(
        q[0],
        QueueExpect {
            interaction: Some("Eat".into()),
            menu: None,
            target: Some(ObjRef::Object),
            priority: Some(50)
        }
    );
    assert_eq!(q[1].menu, Some(2));
    assert_eq!(
        q[2],
        QueueExpect {
            interaction: None,
            menu: None,
            target: None,
            priority: None
        }
    );
    assert_eq!(e.turns, Some(3));
    assert_eq!(e.sim.as_ref().unwrap().motives.get(&7), Some(&50.0));
}

#[test]
fn expectation_shorthands() {
    // `error` implies result = "error"; a count stands for that many entries of any kind.
    let c = one_case("tree = 1\nexpect = { error = \"trap\", dialogs = 2, queue = 1 }");
    assert_eq!(c.expect.result, Some(Outcome::Error));
    assert_eq!(c.expect.error.as_deref(), Some("trap"));
    assert_eq!(c.expect.dialogs.as_ref().unwrap().len(), 2);
    assert!(
        c.expect
            .dialogs
            .as_ref()
            .unwrap()
            .iter()
            .all(|d| d.kind == 0xFF)
    );
    assert_eq!(c.expect.queue.as_ref().unwrap().len(), 1);
    for (text, want) in [
        ("true", Outcome::True),
        ("false", Outcome::False),
        ("\"true\"", Outcome::True),
        ("\"false\"", Outcome::False),
        ("\"error\"", Outcome::Error),
        ("\"ok\"", Outcome::Ok),
        ("\"yield\"", Outcome::Yield),
    ] {
        assert_eq!(
            one_case(&format!(
                "tree = 1\nturns = 2\nexpect = {{ result = {text} }}"
            ))
            .expect
            .result,
            Some(want),
            "{text}"
        );
    }
}

#[test]
fn expectation_errors() {
    let e = |body: &str| one(&format!("tree = 1\nexpect = {{ {body} }}"));
    parse_err(&e("result = \"maybe\""), &["maybe"]);
    parse_err(&e("result = 1"), &["result"]);
    parse_err(&e("error = \"x\", result = true"), &["error"]);
    parse_err(&e("dialogs = [\"popup\"]"), &["popup"]);
    parse_err(&e("dialogs = -1"), &["dialogs"]);
    parse_err(&e("queue = [{ bogus = 1 }]"), &["unknown key 'bogus'"]);
    parse_err(&e("queue = [3]"), &["queue"]);
    parse_err(
        &e("queue = [{ interaction = \"A\", menu = 1 }]"),
        &["not both"],
    );
    parse_err(&e("sounds = [1]"), &["sounds"]);
    parse_err(&e("sounds = \"x\""), &["sounds"]);
    parse_err(&e("stack_obj = \"stove\""), &["stack_obj"]);
    parse_err(&e("bogus = 1"), &["unknown key 'bogus'"]);
}

#[test]
fn dialog_kind_names() {
    assert_eq!(dialog_kind_name(0), Some("ok"));
    assert_eq!(dialog_kind_name(1), Some("yes_no"));
    assert_eq!(dialog_kind_name(2), Some("yes_no_cancel"));
    assert_eq!(dialog_kind_name(3), Some("text"));
    assert_eq!(dialog_kind_name(4), None);
}

#[test]
fn reports_count_cases() {
    let r = FileReport {
        path: "x".into(),
        cases: vec![
            CaseReport {
                name: "a".into(),
                assumptions: vec![],
                failures: vec![],
            },
            CaseReport {
                name: "b".into(),
                assumptions: vec![],
                failures: vec!["attrs.0: expected 1, got 2".into()],
            },
        ],
    };
    assert_eq!(r.passed(), 1);
    assert_eq!(r.failed(), 1);
    assert!(r.cases[0].passed());
    assert!(!r.cases[1].passed());
}

// ---------------------------------------------------------------------------------------
// Collecting files
// ---------------------------------------------------------------------------------------

fn open_pack() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../open-pack")
}

#[test]
fn collects_the_open_pack_specs_sorted() {
    let files = collect_test_files(&open_pack().join("tests")).unwrap();
    assert!(files.len() >= 4, "{files:?}");
    assert!(files.windows(2).all(|w| w[0] < w[1]));
    assert!(
        files
            .iter()
            .all(|f| f.to_string_lossy().ends_with(".test.toml"))
    );
    // A file is taken as it is.
    let one = collect_test_files(&files[0]).unwrap();
    assert_eq!(one, vec![files[0].clone()]);
    assert!(collect_test_files(&open_pack().join("no-such-dir")).is_err());
}

#[test]
fn every_open_pack_spec_parses() {
    for f in collect_test_files(&open_pack().join("tests")).unwrap() {
        let spec = TestFile::read(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        assert!(!spec.cases.is_empty());
    }
}

#[cfg(not(feature = "vm-replay"))]
#[test]
fn running_needs_vm_replay() {
    assert!(!can_run());
    let f = parse_ok(&one("tree = 1"));
    let e = run_test_file(&f, Path::new("x.test.toml")).unwrap_err();
    assert!(e.to_string().contains("vm-replay"), "{e}");
}

// ---------------------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------------------

#[cfg(feature = "vm-replay")]
mod harness_tests {
    use super::*;

    /// A scratch directory of this test process (left in place: tests never delete).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sims-antics-test-runner-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const PROBE: &str = r#"object "Runner Probe" {
    guid: 0x7E00_7E02;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);

    extern mod g {
        #[id(300)] fn probe() -> bool;
    }

    state count: i16 = 3;
    state seen: i16 = 0;

    // A main that does not return: the lot update between turns runs it.
    main {
        loop {
            sleep(1);
        }
    }

    sound beep = preset("chime", wav: "runner_beep");

    #[bhav(id = 4400, label = "Bump")]
    fn bump() {
        self.count += 1;
        play_sound(beep);
        return true;
    }

    #[bhav(id = 4401, label = "Nap")]
    fn nap() {
        sleep(3);
        self.count += 10;
    }

    #[bhav(id = 4402, label = "Loop")]
    fn loop_forever() {
        while 0 == 0 {
            self.count += 1;
        }
    }

    #[bhav(id = 4403, label = "Ask")]
    fn ask() {
        if g::probe() {
            return true;
        }
        return false;
    }

    #[bhav(id = 4404, label = "Look", args = 1)]
    fn look() {
        self.seen = param[0];
    }

    #[bhav(id = 4405, label = "Pay")]
    fn pay() {
        if debit_funds(25) {
            return true;
        }
        return false;
    }

    interaction "Greet" {
        guard(sim) => self.count < 5,
        run(sim) {
            sim.motive.social += 10;
            dialog(kind: yes_no, title: "Probe", message: "Hello?", buttons: ("Yes", "No"));
            self.seen = 1;
        }
    }
}
"#;

    /// Writes the probe object and a spec around `cases` into a fresh scratch directory and
    /// runs it.
    fn run_probe(name: &str, head: &str, cases: &str) -> FileReport {
        let dir = scratch(name);
        std::fs::write(dir.join("probe.antics"), PROBE).unwrap();
        let spec = format!(
            "[subject]\nantics = \"probe.antics\"\n\n[[stub]]\nid = 300\nreturns = true\n\n{head}\n{cases}"
        );
        let path = dir.join("probe.test.toml");
        std::fs::write(&path, &spec).unwrap();
        let file = TestFile::read(&path).unwrap_or_else(|e| panic!("{e}\n{spec}"));
        run_test_file(&file, &path).unwrap_or_else(|e| panic!("{e}\n{spec}"))
    }

    fn failures(r: &FileReport, case: &str) -> Vec<String> {
        r.cases
            .iter()
            .find(|c| c.name == case)
            .unwrap_or_else(|| panic!("no case {case}"))
            .failures
            .clone()
    }

    fn assert_all_pass(r: &FileReport) {
        let bad: Vec<_> = r.cases.iter().filter(|c| !c.passed()).collect();
        assert!(bad.is_empty(), "{bad:#?}");
    }

    #[test]
    fn attribute_names_come_from_the_air_header() {
        let names = harness::attribute_names(
            "// Generated\n// Attributes: attr[0] = occupied, attr[1] = power, attr[2] = watch_cycles\n\ntree 1 {}\n",
        );
        assert_eq!(names.get("occupied"), Some(&0));
        assert_eq!(names.get("watch_cycles"), Some(&2));
        assert_eq!(names.len(), 3);
        assert!(harness::attribute_names("tree 1 {}\n").is_empty());
    }

    /// The runner's own test Sim and idle-loop definitions cannot be created by a spec.
    #[test]
    fn given_create_refuses_the_runners_reserved_guids() {
        let r = run_probe(
            "reserved-guids",
            "",
            &format!(
                r#"
[[test]]
name = "sim"
tree = "Init"
given = {{ create = [{sim:#010X}] }}
expect = {{ result = true }}

[[test]]
name = "idle"
tree = "Init"
given = {{ create = [{idle:#010X}] }}
expect = {{ result = true }}
"#,
                sim = harness::SIM_GUID,
                idle = harness::IDLE_GUID,
            ),
        );
        for (case, what) in [("sim", "test Sim"), ("idle", "idle loop")] {
            let f = failures(&r, case);
            assert!(
                f.len() == 1
                    && f[0].contains("is reserved for the runner's")
                    && f[0].contains(what),
                "{case}: {f:?}"
            );
        }
    }

    #[test]
    fn synchronous_calls_and_state() {
        let r = run_probe(
            "sync",
            "",
            r#"
[[test]]
name = "init"
tree = "Init"
expect = { result = true, attrs = { count = 3, seen = 0 } }

[[test]]
name = "bump"
tree = "Bump"
given = { attrs = { count = 7 } }
expect = { result = true, attrs = { count = 8 }, sounds = ["runner_beep"], turns = 0 }

[[test]]
name = "bump by id and index"
tree = 4400
expect = { result = "ok", attrs = { 0 = 4 } }

[[test]]
name = "stub"
tree = "Ask"
expect = { result = true }

[[test]]
name = "params"
tree = "Look"
params = [9]
expect = { attrs = { seen = 9 } }

[[test]]
name = "funds"
tree = "Pay"
given = { funds = 30 }
expect = { result = true, funds = 5 }

[[test]]
name = "no funds"
tree = "Pay"
given = { funds = 20 }
expect = { result = false, funds = 20 }

[[test]]
name = "globals"
tree = "Init"
given = { globals = { 30 = 4 } }
expect = { globals = { 30 = 4, 8 = 2000 } }
"#,
        );
        assert_all_pass(&r);
    }

    #[test]
    fn failures_name_field_expected_and_actual() {
        let r = run_probe(
            "fail",
            "",
            r#"
[[test]]
name = "wrong"
tree = "Bump"
expect = { result = false, attrs = { count = 99, 1 = 5 }, sounds = [], funds = 1 }
"#,
        );
        let f = failures(&r, "wrong");
        assert_eq!(
            f,
            vec![
                "result: expected false, got true".to_owned(),
                // Index keys sort before names.
                "attrs.1 (seen): expected 5, got 0".to_owned(),
                "attrs.count (attr[0]): expected 99, got 4".to_owned(),
                "funds: expected 1, got 0".to_owned(),
                "sounds: expected [], got [\"runner_beep\"]".to_owned(),
            ]
        );
        assert_eq!(r.failed(), 1);
    }

    #[test]
    fn faults_and_budgets() {
        let r = run_probe(
            "fault",
            "",
            r#"
[[test]]
name = "budget fault"
tree = "Loop"
budget = 1000
expect = { error = "budget" }

[[test]]
name = "unexpected fault"
tree = "Loop"
budget = 1000

[[test]]
name = "missing tree"
tree = 4500

[[test]]
name = "a missing tree is not a fault of the code"
tree = 4500
expect = { result = "error" }

[[test]]
name = "nor as a thread"
tree = 4500
turns = 1
expect = { result = "error" }

[[test]]
name = "missing label"
tree = "Nope"
"#,
        );
        assert!(failures(&r, "budget fault").is_empty(), "{r:#?}");
        let f = failures(&r, "unexpected fault");
        assert!(
            f.len() == 1 && f[0].starts_with("result: the run did not finish: error"),
            "{f:?}"
        );
        for missing in [
            "missing tree",
            "a missing tree is not a fault of the code",
            "nor as a thread",
        ] {
            let f = failures(&r, missing);
            assert!(
                f.len() == 1 && f[0].contains("setup: the subject's code has no tree 4500"),
                "{missing}: {f:?}"
            );
        }
        let f = failures(&r, "missing label");
        assert!(f[0].contains("no tree labelled \"Nope\""), "{f:?}");
        assert!(f[0].contains("Bump"), "lists the labels: {f:?}");
    }

    #[test]
    fn threads_sleep_and_yield() {
        let r = run_probe(
            "thread",
            "",
            r#"
[[test]]
name = "a sleeping tree needs turns"
tree = "Nap"
turns = 1
expect = { result = "yield", attrs = { count = 3 } }

[[test]]
name = "it wakes after its sleep"
tree = "Nap"
turns = 10
expect = { result = true, attrs = { count = 13 }, turns = 4 }

[[test]]
name = "the stack object of the last frame"
tree = "Look"
turns = 1
stack_obj = "none"
expect = { stack_obj = "none", attrs = { seen = 0 } }

[[test]]
name = "stack_obj needs a thread"
tree = "Look"
expect = { stack_obj = "object" }
"#,
        );
        assert!(
            failures(&r, "a sleeping tree needs turns").is_empty(),
            "{r:#?}"
        );
        assert!(
            failures(&r, "it wakes after its sleep").is_empty(),
            "{r:#?}"
        );
        assert!(
            failures(&r, "the stack object of the last frame").is_empty(),
            "{r:#?}"
        );
        let f = failures(&r, "stack_obj needs a thread");
        assert!(f[0].contains("needs the tree to run as a thread"), "{f:?}");
    }

    #[test]
    fn interactions_dialogs_and_motives() {
        let r = run_probe(
            "interaction",
            "[sim]\nmotives = { social = 5 }\n",
            r#"
[[test]]
name = "guard"
guard = "Greet"
expect = { result = true }

[[test]]
name = "guard refuses"
guard = "Greet"
given = { attrs = { count = 5 } }
expect = { result = false }

[[test]]
name = "greet"
interaction = "Greet"
answers = [1]
expect = { result = "ok", dialogs = ["yes_no"], attrs = { seen = 1 }, sim = { motives = { social = 15 } }, queue = 0 }

[[test]]
name = "unanswered"
interaction = "Greet"
turns = 4
expect = { result = "yield", dialogs = 1, attrs = { seen = 0 } }

[[test]]
name = "a button the dialog does not offer"
interaction = "Greet"
answers = [3]
expect = { result = "ok", dialogs = ["yes_no"] }

[[test]]
name = "unavailable"
interaction = "Greet"
given = { attrs = { count = 9 } }
expect = { result = "unavailable" }

[[test]]
name = "wrong dialog"
interaction = "Greet"
answers = [2]
expect = { dialogs = ["ok"], sim = { motives = { social = 1 } } }

[[test]]
name = "unknown interaction"
guard = "Wave"
"#,
        );
        for ok in ["guard", "guard refuses", "greet", "unavailable"] {
            assert!(failures(&r, ok).is_empty(), "{ok}: {:?}", failures(&r, ok));
        }
        assert_eq!(
            failures(&r, "wrong dialog"),
            vec![
                "sim.motives.social: expected 1, got 15".to_owned(),
                "dialogs: expected [\"ok\"], got [\"yes_no\"]".to_owned(),
            ]
        );
        // The game cannot go on without an answer the dialog offers (its
        // `TickError::Unanswered`): whatever the case expects, it fails.
        assert_eq!(
            failures(&r, "unanswered"),
            vec![
                "turn 1: dialogs: a yes_no dialog of object 1 waits for an answer (button(s) 1, 2), and \
                 `answers` has none left"
                    .to_owned()
            ]
        );
        assert_eq!(
            failures(&r, "a button the dialog does not offer"),
            vec![
                "turn 1: dialogs: a yes_no dialog of object 1 offers button(s) 1, 2; the game refuses \
                 answer 3"
                    .to_owned()
            ]
        );
        let f = failures(&r, "unknown interaction");
        assert!(
            f[0].contains("no interaction \"Wave\"") && f[0].contains("\"Greet\""),
            "{f:?}"
        );
    }

    #[test]
    fn unknown_state_names_are_reported() {
        let r = run_probe(
            "names",
            "",
            "[[test]]\nname = \"n\"\ntree = \"Bump\"\ngiven = { attrs = { colour = 1 } }\n",
        );
        let f = failures(&r, "n");
        assert!(
            f[0].contains("no state 'colour'") && f[0].contains("count"),
            "{f:?}"
        );
    }

    #[test]
    fn iff_subjects_load_through_the_same_path() {
        let dir = scratch("iff");
        let src = dir.join("probe.antics");
        std::fs::write(&src, PROBE).unwrap();
        let compiled = crate::v2::compile_v2_object_file(&src, None).unwrap();
        std::fs::write(dir.join("probe.iff"), &compiled.iff_bytes).unwrap();
        let path = dir.join("iff.test.toml");
        std::fs::write(
            &path,
            "[subject]\niff = \"probe.iff\"\n\n[[stub]]\nid = 300\nreturns = false\n\n[[test]]\nname = \"bump\"\ntree = 4400\nexpect = { result = true, attrs = { 0 = 4 } }\n\n[[test]]\nname = \"stub false\"\ntree = \"Ask\"\nexpect = { result = false }\n\n[[test]]\nname = \"no names\"\ntree = 4400\nexpect = { attrs = { count = 4 } }\n",
        )
        .unwrap();
        let r = run_test_file(&TestFile::read(&path).unwrap(), &path).unwrap();
        assert!(failures(&r, "bump").is_empty(), "{r:#?}");
        assert!(failures(&r, "stub false").is_empty(), "{r:#?}");
        assert!(
            failures(&r, "no names")[0].contains("no named states"),
            "{r:#?}"
        );
    }

    #[test]
    fn file_level_errors_are_errors() {
        let dir = scratch("errors");
        std::fs::write(dir.join("probe.antics"), PROBE).unwrap();
        let run = |spec: &str| {
            let path = dir.join("e.test.toml");
            std::fs::write(&path, spec).unwrap();
            let file = TestFile::read(&path).unwrap();
            run_test_file(&file, &path)
                .map(|_| ())
                .unwrap_err()
                .to_string()
        };
        // The probe calls tree 300, but a missing stub only faults when it runs; a stub in
        // the object's own range is refused before anything runs.
        let e = run("[subject]\nantics = \"missing.antics\"\n\n[[test]]\nname = \"t\"\ntree = 1\n");
        assert!(e.contains("missing.antics"), "{e}");
        let e = run(
            "[subject]\nantics = \"probe.antics\"\n\n[[stub]]\nsource = \"tree 4400 \\\"X\\\" { return true; }\"\n\n[[test]]\nname = \"t\"\ntree = 1\n",
        );
        assert!(e.contains("4400"), "{e}");
        let e = run(
            "[subject]\nantics = \"probe.antics\"\n\n[[stub]]\nid = 300\nreturns = true\n\n[[stub]]\nid = 300\nreturns = false\n\n[[test]]\nname = \"t\"\ntree = 1\n",
        );
        assert!(e.contains("twice"), "{e}");
    }

    #[test]
    fn a_missing_global_faults_the_case() {
        let dir = scratch("nostub");
        std::fs::write(dir.join("probe.antics"), PROBE).unwrap();
        let path = dir.join("n.test.toml");
        std::fs::write(
            &path,
            "[subject]\nantics = \"probe.antics\"\n\n[[test]]\nname = \"t\"\ntree = \"Ask\"\nexpect = { result = \"error\" }\n",
        )
        .unwrap();
        let r = run_test_file(&TestFile::read(&path).unwrap(), &path).unwrap();
        assert!(failures(&r, "t").is_empty(), "{r:#?}");
    }

    /// Writes `files` and a spec into a fresh scratch directory and runs the spec.
    fn run_spec(name: &str, files: &[(&str, &str)], spec: &str) -> FileReport {
        let dir = scratch(name);
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
        let path = dir.join("spec.test.toml");
        std::fs::write(&path, spec).unwrap();
        let file = TestFile::read(&path).unwrap_or_else(|e| panic!("{e}\n{spec}"));
        run_test_file(&file, &path).unwrap_or_else(|e| panic!("{e}\n{spec}"))
    }

    /// Interactions that finish in ways the runner must still see: at once, inside the
    /// pick; with a successor queued at the front that finishes in the same turn; with a
    /// text prompt; and with motive changes over time.
    const FLOW: &str = r#"object "Runner Flow" {
    guid: 0x7E00_7E03;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);

    state count: i16 = 0;
    state seen: i16 = 0;

    main {
        loop {
            sleep(1);
        }
    }

    interaction "Zap" {
        flags: 4,
        run(sim) {
            self.count += 100;
        }
    }

    interaction "Chain" {
        run(sim) {
            self.seen += 1;
            let target = self;
            stack_obj = sim;
            push_interaction(menu: 2, object: target, priority: 3, flags: 4);
        }
    }

    interaction "Tail" {
        run(sim) {
            self.seen += 10;
        }
    }

    interaction "Ask" {
        run(sim) {
            dialog(kind: text_entry, title: "Flow", message: "Name?", buttons: ("Save",));
            self.seen = 7;
        }
    }

    interaction "Relax" {
        run(sim) {
            set_motive_change(fun, 10, 100);
            sleep(2);
            clear_motive_changes();
            self.seen = 2;
        }
    }
}
"#;

    #[test]
    fn interactions_that_finish_within_a_turn_or_the_pick_are_done() {
        let r = run_spec(
            "flow",
            &[("flow.antics", FLOW)],
            r#"
[subject]
antics = "flow.antics"

[sim]

[[test]]
name = "an entry that runs at once runs inside the pick"
interaction = "Zap"
expect = { result = "ok", attrs = { count = 100 }, turns = 0, queue = 0 }

[[test]]
name = "a successor queued at the front finishes in the same turn"
interaction = "Chain"
expect = { result = "ok", attrs = { seen = 11 }, turns = 1, queue = 0 }
"#,
        );
        assert_all_pass(&r);
    }

    #[test]
    fn the_lockout_word_wraps_as_the_vms_does() {
        // The object tick's tail counts data word 25 down with a wrapping subtraction;
        // -32768 must not overflow (a panic in a debug build).
        let r = run_spec(
            "wrap",
            &[("flow.antics", FLOW)],
            r#"
[subject]
antics = "flow.antics"

[sim]

[[test]]
name = "wraps"
interaction = "Tail"
given = { sim = { data = { 25 = -32768 } } }
expect = { result = "ok", sim = { data = { 25 = 32767 } } }
"#,
        );
        assert_all_pass(&r);
    }

    #[test]
    fn text_prompts_need_an_explicit_answer() {
        let r = run_spec(
            "text",
            &[("flow.antics", FLOW)],
            r#"
[subject]
antics = "flow.antics"

[sim]

[[test]]
name = "answered"
interaction = "Ask"
answers = [1]
expect = { result = "ok", attrs = { seen = 7 }, dialogs = ["text"] }

[[test]]
name = "unanswered"
interaction = "Ask"
expect = { attrs = { seen = 7 } }

[[test]]
name = "a second button"
interaction = "Ask"
answers = [2]
"#,
        );
        assert!(failures(&r, "answered").is_empty(), "{r:#?}");
        assert_eq!(
            failures(&r, "unanswered"),
            vec![
                "turn 1: dialogs: a text dialog of object 1 waits for an answer (button(s) 1), and \
                 `answers` has none left"
                    .to_owned()
            ]
        );
        assert_eq!(
            failures(&r, "a second button"),
            vec![
                "turn 1: dialogs: a text dialog of object 1 offers button(s) 1; the game refuses answer 2"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn the_sims_person_tick_applies_motive_changes_after_each_turn() {
        // The game's person tick applies the motive changes a script set after every turn
        // of the Sim (`motives::apply_changes`), so a script that reads the motive back
        // later sees them applied (once here: the action clears the change on its next
        // turn), and person word 27 counts the ticks.
        let r = run_spec(
            "deltas",
            &[("review.antics", REVIEW)],
            r#"
[subject]
antics = "review.antics"

[sim]
motives = { fun = 0 }

[[test]]
name = "the script reads the applied change"
interaction = "Relax"
expect = { attrs = { seen = 3 }, sim = { motives = { fun = 33.333336 } } }

[[test]]
name = "a stale read is not what the game does"
interaction = "Relax"
expect = { attrs = { seen = 2 } }

[[test]]
name = "person word 27 counts the Sim's ticks"
interaction = "Tail"
expect = { result = "ok", turns = 1, sim = { person = { 27 = 1 } } }

[[test]]
name = "not counting them is not what the game does"
interaction = "Tail"
expect = { result = "ok", turns = 1, sim = { person = { 27 = 0 } } }
"#,
        );
        for ok in [
            "the script reads the applied change",
            "person word 27 counts the Sim's ticks",
        ] {
            assert!(failures(&r, ok).is_empty(), "{ok}: {r:#?}");
        }
        assert_eq!(
            failures(&r, "a stale read is not what the game does"),
            vec!["attrs.seen (attr[1]): expected 2, got 3".to_owned()]
        );
        assert_eq!(
            failures(&r, "not counting them is not what the game does"),
            vec!["sim.person.27: expected 0, got 1".to_owned()]
        );
    }

    #[test]
    fn the_sims_tick_runs_while_an_objects_thread_runs() {
        // Each turn is a lot update, which ticks the Sim (ID 1) before the subject: person
        // word 27, which the person tick counts, is the game's after a run of an object's
        // thread too.
        let r = run_probe(
            "untick",
            "[sim]\n",
            r#"
[[test]]
name = "counted"
tree = "Nap"
me = "object"
turns = 5
expect = { attrs = { count = 13 }, turns = 4, sim = { person = { 27 = 4 } } }

[[test]]
name = "not counting them is not what the game does"
tree = "Nap"
me = "object"
turns = 5
expect = { attrs = { count = 13 }, sim = { person = { 27 = 0 } } }
"#,
        );
        assert!(failures(&r, "counted").is_empty(), "{r:#?}");
        assert_eq!(
            failures(&r, "not counting them is not what the game does"),
            vec!["sim.person.27: expected 0, got 4".to_owned()]
        );
    }

    /// The third review's repros: a Room read after a turn, a debug entry's shown name, an
    /// entry its selection's check refuses, and an object that removes itself.
    const GAPS: &str = r#"object "Runner Gaps" {
    guid: 0x7E00_7E06;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);

    state seen: i16 = 0;

    main {
        loop {
            sleep(1);
        }
    }

    interaction "Watch Room" {
        run(sim) {
            sleep(2);
            if sim.motive.room == 73 { self.seen = 1; } else { self.seen = 2; }
        }
    }

    interaction "Debug Label" {
        flags: 128,
        run(sim) {
            self.seen = 3;
        }
    }

    interaction "Check Twice" {
        guard(sim) {
            self.seen += 1;
            return self.seen < 3;
        }
        run(sim) {
            if self.seen == 2 { self.seen = 4; } else { self.seen = 5; }
        }
    }

    #[bhav(id = 4410, label = "Remove Self")]
    fn remove_self() {
        destroy_self(no_yield: true);
        sleep(1);
        return true;
    }
}
"#;

    fn run_gaps(name: &str, cases: &str) -> FileReport {
        run_spec(
            name,
            &[("gaps.antics", GAPS)],
            &format!(
                "[subject]\nantics = \"gaps.antics\"\n\n[sim]\nmotives = {{ room = 73 }}\n{cases}"
            ),
        )
    }

    #[test]
    fn the_sims_person_tick_sets_room() {
        // The person tick sets Room from the score of the room the Sim is in after each
        // turn (0x45cef4): a script reading it later does not see the given value.
        let r = run_gaps(
            "room",
            r#"
[[test]]
name = "the room's score"
interaction = "Watch Room"
expect = { result = "ok", attrs = { seen = 2 } }

[[test]]
name = "the given Room is stale"
interaction = "Watch Room"
expect = { attrs = { seen = 1 } }
"#,
        );
        assert!(failures(&r, "the room's score").is_empty(), "{r:#?}");
        assert_eq!(
            failures(&r, "the given Room is stale"),
            vec!["attrs.seen (attr[0]): expected 1, got 2".to_owned()]
        );
    }

    #[test]
    fn an_object_removed_at_the_end_of_an_update_does_not_run_on() {
        // The end of each lot update removes the objects queued for removal; a thread whose
        // object queued itself does not get another turn.
        let r = run_gaps(
            "removed",
            r#"
[[test]]
name = "removed"
tree = "Remove Self"
turns = 4
expect = { error = "left the lot at the end of turn 1" }

[[test]]
name = "returning after the removal is not what the game does"
tree = "Remove Self"
turns = 4
expect = { result = true }
"#,
        );
        assert!(failures(&r, "removed").is_empty(), "{r:#?}");
        let f = failures(&r, "returning after the removal is not what the game does");
        assert!(
            f.len() == 1
                && f[0].starts_with("result: expected true, got error")
                && f[0].contains("the subject (object 2) left the lot at the end of turn 1"),
            "{f:?}"
        );
    }

    #[test]
    fn the_selection_runs_the_check_again() {
        // The game checks an entry when it lists the menu, again when the player selects it
        // (`highlight`, 0x48df28), and when the action starts: the third check refuses here.
        let r = run_gaps(
            "recheck",
            r#"
[[test]]
name = "three checks"
interaction = "Check Twice"
expect = { result = "unavailable", attrs = { seen = 3 } }

[[test]]
name = "a click that skips the selection's check"
interaction = "Check Twice"
expect = { attrs = { seen = 4 } }
"#,
        );
        assert!(failures(&r, "three checks").is_empty(), "{r:#?}");
        assert_eq!(
            failures(&r, "a click that skips the selection's check"),
            vec![
                "result: the run did not finish: unavailable (the Sim dropped \"Check Twice\" \
                 from its queue without starting it)"
                    .to_owned(),
                "attrs.seen (attr[0]): expected 4, got 3".to_owned(),
            ]
        );
    }

    #[test]
    fn a_debug_entry_is_shown_with_a_star() {
        // The menu shows a debug entry (TTAB flag 0x80) with "*" before its label (sims-game
        // `menu::entry_label`); guard/action/queue keep the table's name.
        let r = run_gaps(
            "debug",
            r#"
[[test]]
name = "shown"
interaction = "*Debug Label"
expect = { result = "ok", attrs = { seen = 3 } }

[[test]]
name = "the table's name is not shown"
interaction = "Debug Label"
expect = { attrs = { seen = 3 } }

[[test]]
name = "the action by the table's name"
action = "Debug Label"
expect = { result = true, attrs = { seen = 3 } }
"#,
        );
        for ok in ["shown", "the action by the table's name"] {
            assert!(failures(&r, ok).is_empty(), "{ok}: {r:#?}");
        }
        assert_eq!(
            failures(&r, "the table's name is not shown"),
            vec![
                "result: the run did not finish: unavailable (the pie menu does not offer \
                 \"Debug Label\"; it shows that entry as \"*Debug Label\")"
                    .to_owned(),
                "attrs.seen (attr[0]): expected 3, got 1".to_owned(),
            ]
        );
    }

    #[test]
    fn a_name_the_file_lacks_is_a_setup_error_when_the_menu_faults() {
        // A pet's pie menu is not ported, so listing it faults; a name the file does not
        // have is still the spec's mistake, not an error of the code under test.
        let r = run_gaps(
            "listing",
            r#"
[[test]]
name = "misspelled"
interaction = "No Such Interaction"
given = { sim = { person = { 65 = 2 } } }
expect = { result = "error" }

[[test]]
name = "spelled"
interaction = "Watch Room"
given = { sim = { person = { 65 = 2 } } }
expect = { error = "a pet's pie menu" }
"#,
        );
        let f = failures(&r, "misspelled");
        assert!(
            f.len() == 1
                && f[0]
                    .starts_with("setup: the subject has no interaction \"No Such Interaction\"")
                && f[0].contains("listing the pie menu faulted"),
            "{f:?}"
        );
        assert!(failures(&r, "spelled").is_empty(), "{r:#?}");
    }

    #[test]
    fn a_threaded_run_takes_no_budget() {
        let r = run_gaps(
            "budget",
            "[[test]]\nname = \"b\"\ntree = \"Remove Self\"\nturns = 2\nbudget = 500\nexpect = { error = \"left the lot\" }\n",
        );
        let f = failures(&r, "b");
        assert!(
            f.len() == 1 && f[0].contains("budget is for synchronous calls"),
            "{f:?}"
        );
    }

    /// The second review's repros: an at-once action that queues a successor, a check that names its entry, and an action that
    /// reads a motive back after setting a change over time.
    const REVIEW: &str = r#"object "Runner Review" {
    guid: 0x7E00_7E04;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);

    strings 301 "Runner names" { renamed = "Displayed Only", }
    state count: i16 = 0;
    state seen: i16 = 0;

    main {
        loop {
            sleep(1);
        }
    }

    interaction "Zap" {
        flags: 4,
        run(sim) {
            self.count += 100;
            let target = self;
            stack_obj = sim;
            push_interaction(menu: 1, object: target, priority: 3, flags: 0);
        }
    }

    interaction "Tail" {
        run(sim) {
            self.seen += 10;
        }
    }

    interaction "Labeled" {
        guard(sim) {
            action_label(table: 301, index: 1);
            return true;
        }
        run(sim) {
            self.seen += 20;
        }
    }

    interaction "Relax" {
        run(sim) {
            set_motive_change(fun, 30000, 100);
            sleep(2);
            clear_motive_changes();
            if sim.motive.fun == 0 { self.seen = 2; } else { self.seen = 3; }
        }
    }
}
"#;

    /// A check that makes a dialog and refuses (listing the menu runs it).
    const REFUSING: &str = r#"object "Runner Refusing" {
    guid: 0x7E00_7E05;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);

    state count: i16 = 0;

    main {
        loop {
            sleep(1);
        }
    }

    interaction "No Entry" {
        guard(sim) {
            dialog(kind: yes_no, title: "Review", message: "Continue?", buttons: ("Yes", "No"), inform: true);
            return false;
        }
        run(sim) {
            self.count += 1;
        }
    }
}
"#;

    #[test]
    fn dialogs_from_a_refusing_check_are_settled() {
        let r = run_spec(
            "refused",
            &[("refusing.antics", REFUSING)],
            r#"
[subject]
antics = "refusing.antics"

[sim]

[[test]]
name = "unanswered"
interaction = "No Entry"
expect = { result = "unavailable", dialogs = 0 }

[[test]]
name = "answered"
interaction = "No Entry"
answers = [1]
expect = { result = "unavailable", dialogs = ["yes_no"], attrs = { count = 0 } }
"#,
        );
        assert_eq!(
            failures(&r, "unanswered"),
            vec![
                "dialogs: a yes_no dialog of object 1 waits for an answer (button(s) 1, 2), \
                 and `answers` has none left"
                    .to_owned()
            ]
        );
        assert!(failures(&r, "answered").is_empty(), "{r:#?}");
    }

    #[test]
    fn an_at_once_action_is_done_after_the_pick() {
        // `pick` returns the number of the successor the at-once action queued; the runner
        // must not run that successor as if it were the selection.
        let r = run_spec(
            "at-once",
            &[("review.antics", REVIEW)],
            r#"
[subject]
antics = "review.antics"

[sim]

[[test]]
name = "the successor stays queued"
interaction = "Zap"
expect = { result = "ok", attrs = { count = 100, seen = 0 }, turns = 0, queue = [{ interaction = "Tail" }] }

[[test]]
name = "the successor is not run as the selection"
interaction = "Zap"
expect = { result = "ok", attrs = { count = 100, seen = 10 }, turns = 1, queue = 0 }
"#,
        );
        assert!(
            failures(&r, "the successor stays queued").is_empty(),
            "{r:#?}"
        );
        assert_eq!(
            failures(&r, "the successor is not run as the selection"),
            vec![
                "turns: expected 1, got 0".to_owned(),
                "attrs.seen (attr[1]): expected 10, got 0".to_owned(),
                "queue: expected [], got [\"Tail\" on object (priority 50)]".to_owned(),
            ]
        );
    }

    #[test]
    fn entries_are_selected_by_the_name_the_menu_shows() {
        let r = run_spec(
            "label",
            &[("review.antics", REVIEW)],
            r#"
[subject]
antics = "review.antics"

[sim]

[[test]]
name = "the shown name"
interaction = "Displayed Only"
expect = { result = "ok", attrs = { seen = 20 } }

[[test]]
name = "the table's name is not shown"
interaction = "Labeled"
expect = { result = "ok", attrs = { seen = 20 } }

[[test]]
name = "neither"
interaction = "Nowhere"
"#,
        );
        assert!(failures(&r, "the shown name").is_empty(), "{r:#?}");
        assert_eq!(
            failures(&r, "the table's name is not shown"),
            vec![
                "result: expected ok (no fault), got unavailable (the pie menu does not offer \
                 \"Labeled\"; it shows that entry as \"Displayed Only\")"
                    .to_owned(),
                "attrs.seen (attr[1]): expected 20, got 0".to_owned(),
            ]
        );
        let f = failures(&r, "neither");
        assert!(
            f.len() == 1
                && f[0].contains("no interaction \"Nowhere\"")
                && f[0].contains("its pie menu shows: ")
                && f[0].contains("\"Displayed Only\""),
            "{f:?}"
        );
    }

    #[test]
    fn entry_arguments_are_limited_in_both_modes() {
        let r = run_probe(
            "params",
            "[[stub]]\nsource = 'tree 303 \"Probe\" { return true; }'\n",
            &format!(
                r#"
[[test]]
name = "255 sync"
tree = 303
params = [{p255}]
expect = {{ result = true }}

[[test]]
name = "255 thread"
tree = 303
turns = 1
params = [{p255}]
expect = {{ result = true }}

[[test]]
name = "256 sync"
tree = 303
params = [{p256}]
expect = {{ result = "error", error = "entry arguments exceed the native limit of 255" }}

[[test]]
name = "256 thread"
tree = 303
turns = 1
params = [{p256}]
expect = {{ result = "error", error = "entry arguments exceed the native limit of 255" }}
"#,
                p255 = vec!["0"; 255].join(", "),
                p256 = vec!["0"; 256].join(", "),
            ),
        );
        assert_all_pass(&r);
    }

    #[test]
    fn stack_obj_is_the_one_the_frame_returned_with() {
        // The tree's only instruction changes the stack object and returns.
        let r = run_probe(
            "stack",
            "[[stub]]\nsource = \"tree 302 \\\"Drop\\\" { stack_obj := 0; }\"\n",
            r#"
[[test]]
name = "stale"
tree = 302
turns = 1
expect = { stack_obj = "object" }

[[test]]
name = "final"
tree = 302
turns = 1
expect = { stack_obj = "none", result = true }
"#,
        );
        assert_eq!(
            failures(&r, "stale"),
            vec!["stack_obj: expected object, got none".to_owned()]
        );
        assert!(failures(&r, "final").is_empty(), "{r:#?}");
    }

    #[test]
    fn a_threaded_tree_gets_the_frame_a_call_gets() {
        // Format 0x8000 always gives a frame four arguments, whatever the header's count
        // (`Behavior::frame_dimensions`), so the tree reads its fourth as a call would.
        let r = run_probe(
            "frame",
            "[[stub]]\nsource = \"tree 303 \\\"Wide\\\" (format: 0x8000) { stack_obj := param[3]; }\"\n",
            r#"
[[test]]
name = "thread"
tree = 303
turns = 1
params = [1, 2, 3, 9]
expect = { result = "ok", stack_obj = 9 }

[[test]]
name = "call"
tree = 303
params = [1, 2, 3, 9]
expect = { result = "ok" }
"#,
        );
        assert_all_pass(&r);
    }

    #[test]
    fn a_state_name_and_its_index_are_one_key() {
        let r = run_probe(
            "alias",
            "",
            r#"
[[test]]
name = "given"
tree = "Bump"
given = { attrs = { count = 1, 0 = 2 } }

[[test]]
name = "expect"
tree = "Bump"
expect = { attrs = { count = 4, 0 = 9 } }
"#,
        );
        for (case, what) in [("given", "given.attrs"), ("expect", "expect.attrs")] {
            let f = failures(&r, case);
            assert!(
                f.len() == 1
                    && f[0].contains(what)
                    && f[0].contains("'0' and 'count' both set attribute 0"),
                "{case}: {f:?}"
            );
        }
    }

    /// Two definitions in one file, each with its own interaction table: `a` names menu
    /// number 0 "Alpha", `b` names it "Beta". Built from two compiled objects, `b`'s
    /// definition, table and names renumbered to 129.
    fn two_definitions(dir: &Path) -> (PathBuf, u32) {
        two_definitions_with(
            dir,
            "main {\n        loop {\n            sleep(1);\n        }\n    }",
        )
    }

    /// [`two_definitions`] with `main` as both definitions' Main.
    fn two_definitions_with(dir: &Path, main: &str) -> (PathBuf, u32) {
        let a = format!(
            r#"object "Runner Alpha" {{
    guid: 0x7E00_7E10;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);
    state n: i16 = 0;
    {main}
    interaction "Alpha" {{
        run(sim) {{
            self.n += 1;
        }}
    }}
}}
"#
        );
        let b = a
            .replace("Runner Alpha", "Runner Beta")
            .replace("0x7E00_7E10", "0x7E00_7E11")
            .replace("\"Alpha\"", "\"Beta\"");
        let compile = |name: &str, text: &str| {
            let path = dir.join(name);
            std::fs::write(&path, text).unwrap();
            let bytes = crate::v2::compile_v2_object_file(&path, None)
                .unwrap()
                .iff_bytes;
            sims_formats::iff::Iff::parse(bytes).unwrap()
        };
        let (a, b) = (compile("a.antics", &a), compile("b.antics", &b));
        let chunk = |iff: &sims_formats::iff::Iff, c: &sims_formats::iff::Chunk| {
            sims_formats::iff::ChunkData {
                tag: c.tag,
                id: c.id,
                flags: c.flags,
                label: c.label.clone(),
                data: iff.data(c).to_vec(),
            }
        };
        let mut chunks: Vec<_> = a
            .chunks
            .iter()
            .filter(|c| &c.tag != b"rsmp")
            .map(|c| chunk(&a, c))
            .collect();
        let objd = b.all(b"OBJD").next().unwrap();
        let table = u16::from_le_bytes(b.data(objd)[14..16].try_into().unwrap());
        let mut def = chunk(&b, objd);
        def.id = 129;
        def.data[14..16].copy_from_slice(&129u16.to_le_bytes());
        chunks.push(def);
        for tag in [b"TTAB", b"TTAs"] {
            let mut c = chunk(&b, b.get(tag, table).unwrap());
            c.id = 129;
            chunks.push(c);
        }
        let path = dir.join("two.iff");
        std::fs::write(&path, crate::encode::encode_iff_with_rsmp(&chunks).unwrap()).unwrap();
        (path, 0x7E00_7E11)
    }

    #[test]
    fn interaction_names_come_from_the_targets_own_definition() {
        let dir = scratch("definitions");
        let (_, beta) = two_definitions(&dir);
        let spec = format!(
            r#"[subject]
iff = "two.iff"

[[stub]]
source = "tree 302 \"PushOn\" (args: 1, locals: 1) {{ local[0] := param[0]; push_interaction(menu: 0, object: 0, priority: 3, flags: 2); return true; }}"

[sim]

[[test]]
name = "the second definition's entry"
tree = 302
stack_obj = "sim"
params = [3]
given = {{ create = [{beta}] }}
expect = {{ result = true, queue = [{{ interaction = "Beta", target = 3 }}] }}

[[test]]
name = "not the first definition's name for its number"
tree = 302
stack_obj = "sim"
params = [3]
given = {{ create = [{beta}] }}
expect = {{ queue = [{{ interaction = "Alpha" }}] }}

[[test]]
name = "the subject's own"
interaction = "Alpha"
expect = {{ attrs = {{ 0 = 1 }} }}

[[test]]
name = "another definition's name is not the subject's"
interaction = "Beta"
expect = {{ result = "unavailable" }}

[[test]]
name = "nor when listing faults"
interaction = "Beta"
given = {{ sim = {{ person = {{ 65 = 2 }} }} }}
expect = {{ error = "a pet's pie menu" }}

[[test]]
name = "the subject's own name when listing faults"
interaction = "Alpha"
given = {{ sim = {{ person = {{ 65 = 2 }} }} }}
expect = {{ error = "a pet's pie menu" }}
"#
        );
        let path = dir.join("two.test.toml");
        std::fs::write(&path, &spec).unwrap();
        let r = run_test_file(&TestFile::read(&path).unwrap(), &path).unwrap();
        for ok in [
            "the second definition's entry",
            "the subject's own",
            "the subject's own name when listing faults",
        ] {
            assert!(failures(&r, ok).is_empty(), "{ok}: {r:#?}");
        }
        // A name only another definition of the file has is not on the subject's menu: the
        // spec's mistake, whether or not listing the menu faults.
        for (case, then) in [
            (
                "another definition's name is not the subject's",
                "its pie menu shows: \"Alpha\"",
            ),
            ("nor when listing faults", "listing the pie menu faulted"),
        ] {
            let f = failures(&r, case);
            assert!(
                f.len() == 1
                    && f[0].starts_with(
                        "setup: the subject has no interaction \"Beta\" (it has: \"Alpha\")"
                    )
                    && f[0].contains(then),
                "{case}: {f:?}"
            );
        }
        assert_eq!(
            failures(&r, "not the first definition's name for its number"),
            vec!["queue: expected [\"Alpha\"], got [\"Beta\" on 3 (priority 50)]".to_owned()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_subject_that_is_not_a_regular_file_is_refused() {
        // Compiling a FIFO named as the subject would block on the open.
        let dir = scratch("fifo-subject");
        let fifo = dir.join("subject.antics");
        if !fifo.exists() {
            let made = std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .expect("mkfifo");
            assert!(made.success());
        }
        let path = dir.join("spec.test.toml");
        std::fs::write(
            &path,
            "[subject]\nantics = \"subject.antics\"\n\n[[test]]\nname = \"t\"\ntree = 1\n",
        )
        .unwrap();
        let e = run_test_file(&TestFile::read(&path).unwrap(), &path)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("subject.antics") && e.contains("not a regular file"),
            "{e}"
        );
    }

    /// The bed's cases: the Sim starts sleeping within three turns (the settle sound proves
    /// the action ran), and a busy Sim never starts it.
    const BED_CASES: &str = r#"
[sim]
motives = { energy = 0 }

[[test]]
name = "left"
interaction = "Sleep (Left Side)"
turns = 3
expect = { result = "yield", sounds = ["open_double_bed_settle"] }

[[test]]
name = "busy"
interaction = "Sleep (Left Side)"
turns = 3
given = { sim = { priority = 100 } }
expect = { result = "yield" }
"#;

    /// A copy of `open-pack`'s sources in `dir` (the compiler wants the subject inside its
    /// pack), its double bed given a Main that does not return if the pack's has none (an
    /// object without one gets `return true;`, which the lot update faults on). Returns the
    /// pack and the bed.
    fn looping_bed(dir: &Path) -> (PathBuf, PathBuf) {
        let pack = dir.join("pack");
        for sub in ["", "objects", "templates", "common"] {
            std::fs::create_dir_all(pack.join(sub)).unwrap();
            // Copied in name order (the listing's own order depends on the filesystem).
            #[allow(clippy::disallowed_methods)]
            let mut entries: Vec<_> = std::fs::read_dir(open_pack().join(sub))
                .unwrap()
                .map(|e| e.unwrap())
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let name = e.file_name();
                let keep = Path::new(&name)
                    .extension()
                    .is_some_and(|x| x == "antics" || x == "toml");
                if keep && e.file_type().unwrap().is_file() {
                    std::fs::copy(e.path(), pack.join(sub).join(&name)).unwrap();
                }
            }
        }
        let bed = pack.join("objects/open-double-bed.antics");
        let src = std::fs::read_to_string(&bed).unwrap();
        if !src.contains("\n    main {") {
            let at = src.find("\nobject \"").expect("the bed's object");
            let open = at + src[at..].find('{').unwrap() + 1;
            let text = format!(
                "{}\n    main {{\n        loop {{\n            sleep(1);\n        }}\n    }}\n{}",
                &src[..open],
                &src[open..]
            );
            std::fs::write(&bed, text).unwrap();
        }
        (pack, bed)
    }

    fn assert_bed(r: &FileReport) {
        let f = failures(r, "left");
        assert!(f.is_empty(), "{f:?}");
        assert_eq!(
            failures(r, "busy"),
            vec![
                "result: expected yield, got unavailable (the Sim never started \"Sleep (Left \
                 Side)\" in 3 turns (it is still queued; a Sim whose person.priority is not 0 \
                 is busy))"
                    .to_owned()
            ]
        );
    }

    /// A multi-tile object's pie menu targets its lead tile; the runner runs the entry on
    /// the target the menu gives it.
    #[test]
    fn a_multi_tile_objects_interaction_runs() {
        let (pack, bed) = looping_bed(&scratch("tiles-bed"));
        let r = run_spec(
            "tiles",
            &[],
            &format!(
                "[subject]\nantics = \"{}\"\npack = \"{}\"\n{BED_CASES}",
                bed.display(),
                pack.display()
            ),
        );
        assert_bed(&r);
    }

    /// A tile whose OBJD word 7 is -1 uses its group master's interaction table (sims-vm's
    /// `Program::interactions`); its names are the master table's TTAs. The fixture is the
    /// compiled `open-double-bed` with every tile but the master set to inherit.
    #[test]
    fn a_tile_that_inherits_its_masters_table_keeps_its_names() {
        use sims_formats::iff::{ChunkData, Iff};
        let dir = scratch("inherited");
        let (pack, bed) = looping_bed(&dir);
        let bytes = crate::v2::compile_v2_object_file(&bed, Some(&pack))
            .unwrap()
            .iff_bytes;
        let iff = Iff::parse(bytes).unwrap();
        let word = |data: &[u8], i: usize| i16::from_le_bytes([data[2 * i], data[2 * i + 1]]);
        let mut patched = 0;
        let chunks: Vec<ChunkData> = iff
            .chunks
            .iter()
            .filter(|c| &c.tag != b"rsmp")
            .map(|c| {
                let mut data = iff.data(c).to_vec();
                if &c.tag == b"OBJD" && word(&data, 10) != 0 && word(&data, 11) != -1 {
                    assert!(word(&data, 7) > 0, "the tile names a table of its own");
                    data[14..16].copy_from_slice(&(-1i16).to_le_bytes());
                    patched += 1;
                }
                ChunkData {
                    tag: c.tag,
                    id: c.id,
                    flags: c.flags,
                    label: c.label.clone(),
                    data,
                }
            })
            .collect();
        assert!(patched >= 2, "the bed's tiles: {patched}");
        std::fs::write(
            dir.join("inherited.iff"),
            crate::encode::encode_iff_with_rsmp(&chunks).unwrap(),
        )
        .unwrap();
        let path = dir.join("inherited.test.toml");
        std::fs::write(
            &path,
            format!("[subject]\niff = \"inherited.iff\"\n{BED_CASES}"),
        )
        .unwrap();
        let r = run_test_file(&TestFile::read(&path).unwrap(), &path).unwrap();
        assert_bed(&r);
    }

    /// An object whose Main returns at once (v2's default Main idles, so it says so).
    const RETURNING: &str = r#"object "Runner Returning" {
    guid: 0x7E00_7E21;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);

    state count: i16 = 0;

    main {
        return true;
    }

    interaction "Poke" {
        run(sim) {
            self.count += 1;
        }
    }
}
"#;

    /// Another, for `[lot].also`.
    const RETURNING_TOO: &str = r#"object "Runner Returning Too" {
    guid: 0x7E00_7E22;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);

    state count: i16 = 0;

    main {
        return true;
    }
}
"#;

    const POKE_CASES: &str = r#"
[sim]

[[test]]
name = "poke"
interaction = "Poke"
expect = { result = "ok", attrs = { count = 1 } }

[[test]]
name = "main"
tree = "Main"
expect = { result = true, turns = 0 }
"#;

    #[test]
    fn idle_main_lets_an_interaction_run_on_an_object_whose_main_returns() {
        let r = run_spec(
            "idle-main-off",
            &[("returning.antics", RETURNING)],
            &format!("[subject]\nantics = \"returning.antics\"\n{POKE_CASES}"),
        );
        assert!(r.cases.iter().all(|c| c.assumptions.is_empty()), "{r:#?}");
        let f = failures(&r, "poke");
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(
            f[0].contains("the subject (object 2), which has no thread to run: its Main returned")
                && f[0].contains("idle_main = true"),
            "{f:?}"
        );
        // The real Main, which returns at once, runs as it is.
        assert!(failures(&r, "main").is_empty(), "{r:#?}");

        let r = run_spec(
            "idle-main-on",
            &[("returning.antics", RETURNING)],
            &format!("[subject]\nantics = \"returning.antics\"\nidle_main = true\n{POKE_CASES}"),
        );
        assert_all_pass(&r);
        for case in &r.cases {
            assert_eq!(
                case.assumptions,
                vec![
                    "subject Main (OBJD #128, GUID 0x7e007e21) replaced by an idle loop \
                     (idle_main)"
                        .to_owned()
                ],
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn idle_main_on_a_main_that_loops_is_a_setup_error() {
        let dir = scratch("idle-main-loops");
        std::fs::write(dir.join("probe.antics"), PROBE).unwrap();
        let path = dir.join("p.test.toml");
        std::fs::write(
            &path,
            "[subject]\nantics = \"probe.antics\"\nidle_main = true\n\n[[test]]\nname = \"t\"\ntree = \"Bump\"\nexpect = { result = true }\n",
        )
        .unwrap();
        let r = run_test_file(&TestFile::read(&path).unwrap(), &path).unwrap();
        let f = failures(&r, "t");
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(
            f[0].starts_with("setup: ")
                && f[0].contains("idle_main is only for a Main that returns")
                && f[0].contains("subject (OBJD #128, GUID 0x7e007e02)'s Main is still running"),
            "{f:?}"
        );
    }

    #[test]
    fn idle_main_on_an_also_object_applies_to_it_only() {
        let files = [
            ("returning.antics", RETURNING),
            ("returning-too.antics", RETURNING_TOO),
        ];
        let spec = |also: &str| {
            format!(
                "[subject]\nantics = \"returning.antics\"\nidle_main = true\n\n[lot]\nalso = [{also}]\n\n[sim]\n\n[[test]]\nname = \"poke\"\ninteraction = \"Poke\"\ngiven = {{ create = [\"returning-too\"] }}\nexpect = {{ result = \"ok\", attrs = {{ count = 1 }} }}\n"
            )
        };
        // The subject's key does not reach the other object: its returning Main still fails
        // the case, without the subject's hint.
        let r = run_spec(
            "idle-main-also-off",
            &files,
            &spec("\"returning-too.antics\""),
        );
        let f = failures(&r, "poke");
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(
            f[0].contains("object 3, which has no thread to run") && !f[0].contains("idle_main"),
            "{f:?}"
        );
        let r = run_spec(
            "idle-main-also-on",
            &files,
            &spec("{ antics = \"returning-too.antics\", idle_main = true }"),
        );
        assert_all_pass(&r);
        assert_eq!(
            r.cases[0].assumptions,
            vec![
                "subject Main (OBJD #128, GUID 0x7e007e21) replaced by an idle loop (idle_main)"
                    .to_owned(),
                "returning-too Main (OBJD #128, GUID 0x7e007e22) replaced by an idle loop \
                 (idle_main)"
                    .to_owned(),
            ]
        );
    }

    /// An object whose Main returns (it declares none) and whose "Probe" calls private tree
    /// 8191, which its file does not have: the ID the idle loop took when it was added to the
    /// object's own file.
    const CALLS_MISSING: &str = r#"object "Runner Calls Missing" {
    guid: 0x7E00_7E23;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);
    main { return true; }
    #[bhav(label = "Probe")]
    fn probe() {
        raw { call 8191(); }
    }
}
"#;

    #[test]
    fn idle_main_adds_no_tree_the_lot_can_call() {
        let cases = r#"
[[test]]
name = "a call"
tree = "Probe"
turns = 2
expect = { error = "tree 0x1fff is missing" }

[[test]]
name = "the spec's ID"
tree = 8191
turns = 2
expect = { result = "yield" }

[[test]]
name = "the spec's label"
tree = "TestRunnerIdleMain"
turns = 2
expect = { result = "yield" }
"#;
        for idle in [false, true] {
            let r = run_spec(
                &format!("idle-unreachable-{idle}"),
                &[("calls.antics", CALLS_MISSING)],
                &format!("[subject]\nantics = \"calls.antics\"\nidle_main = {idle}\n{cases}"),
            );
            assert!(failures(&r, "a call").is_empty(), "{idle}: {r:#?}");
            assert_eq!(
                failures(&r, "the spec's ID"),
                vec!["setup: the subject's code has no tree 8191".to_owned()],
                "{idle}"
            );
            let f = failures(&r, "the spec's label");
            assert!(
                f.len() == 1 && f[0].starts_with("setup: no tree labelled \"TestRunnerIdleMain\""),
                "{idle}: {f:?}"
            );
            // The threaded runs replaced the subject's Main when it was made.
            let replaced = r
                .cases
                .iter()
                .all(|c| c.assumptions.len() == usize::from(idle));
            assert!(replaced, "{idle}: {r:#?}");
        }
    }

    /// Switches the Sim's decay on (person word 29 to 0) after 60 ticks: in the tick whose
    /// person tick reaches the decay point, after the Sim's own turn.
    const DECAY: &str = r#"object "Runner Decay" {
    guid: 0x7E00_7E24;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);
    main { loop { sleep(1); } }
    interaction "Enable Decay" {
        run(sim) {
            sleep(60);
            raw { my.person[29] := 0; }
        }
    }
    interaction "Wait" {
        run(sim) {
            sleep(60);
        }
    }
    interaction "Short" {
        run(sim) {
            sleep(10);
        }
    }
}
"#;

    #[test]
    fn the_decay_point_stops_a_run_whatever_switches_decay_off() {
        let r = run_spec(
            "decay-point",
            &[("decay.antics", DECAY)],
            r#"[subject]
antics = "decay.antics"

[sim]
person = { 29 = 1 }
motives = { fun = 50 }

[[test]]
name = "switched on in the decay tick"
interaction = "Enable Decay"
turns = 62
expect = { result = "ok", turns = 61 }

[[test]]
name = "off throughout"
interaction = "Wait"
turns = 62
expect = { result = "ok", turns = 61, sim = { motives = { fun = 50 } } }

[[test]]
name = "hidden throughout"
interaction = "Wait"
turns = 62
given = { sim = { person = { 29 = 0 }, data = { 34 = 1 } } }
expect = { result = "ok", turns = 61 }

[[test]]
name = "short of the decay point"
interaction = "Short"
expect = { result = "ok", sim = { motives = { fun = 50 } } }
"#,
        );
        for case in [
            "switched on in the decay tick",
            "off throughout",
            "hidden throughout",
        ] {
            let f = failures(&r, case);
            assert!(
                f.len() == 1
                    && f[0].starts_with(
                        "turn 61: the Sim's person tick reached its motive decay (every 60 ticks)"
                    ),
                "{case}: {f:?}"
            );
        }
        assert!(
            failures(&r, "short of the decay point").is_empty(),
            "{r:#?}"
        );
    }

    /// A check that faults the second time it runs (the selection's), after changing the lot.
    const SELECTION_FAULT: &str = r#"object "Runner Selection Fault" {
    guid: 0x7E00_7E25;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);
    main { loop { sleep(1); } }
    state checked: i16 = 0;
    interaction "Probe" {
        guard(sim) {
            self.checked += 1;
            if self.checked == 2 { raw { call 300(); } }
            return true;
        }
        run(sim) { self.checked += 10; }
    }
}
"#;

    #[test]
    fn a_fault_in_the_selections_check_fails_the_case() {
        let r = run_spec(
            "selection-fault",
            &[("fault.antics", SELECTION_FAULT)],
            r#"[subject]
antics = "fault.antics"

[sim]

[[test]]
name = "probe"
interaction = "Probe"
expect = { error = "tree 0x012c is missing", attrs = { checked = 2 } }
"#,
        );
        let f = failures(&r, "probe");
        assert!(
            f.len() == 1
                && f[0].starts_with(
                    "\"Probe\": the check run when the entry is selected (0x48df28) faulted ("
                )
                && f[0].contains("tree 0x012c is missing")
                && f[0].ends_with("the runner does not follow"),
            "{f:?}"
        );
    }

    #[test]
    fn a_main_that_makes_its_own_kind_fails_without_overflowing_the_stack() {
        let r = run_spec(
            "recursive-creation",
            &[(
                "recursive.antics",
                r#"object "Runner Recursive" {
    guid: 0x7E00_7E26;
    price: 1;
    sprite: Sprite::procedural("floor_lamp", primary: #101010, accent: #202020, trim: #303030);
    main { create_object(0x7E007E26, mode: 6); }
}
"#,
            )],
            "[subject]\nantics = \"recursive.antics\"\n\n[[test]]\nname = \"main\"\ntree = \"Main\"\n",
        );
        // The factory refuses the creation nested `MAX_CREATION_DEPTH` deep, so the
        // chain stops there, with one short line per level and nothing elided.
        let f = failures(&r, "main");
        let limit = sims_vm::MAX_CREATION_DEPTH;
        assert!(
            f.len() == 1
                && f[0].starts_with("setup: creating the subject: faulted: object 1,")
                && f[0].contains(&format!(
                    "object {limit}, tree 0x1001 node 1: unsupported: creating an object of type \
                     0x7E007E26: object 0: unsupported: creating an object of type 0x7E007E26 \
                     inside {limit} nested creations"
                ))
                && f[0].ends_with("the original's limit is not modeled")
                && !f[0].contains(" characters elided ")
                && f[0].len() < 5000,
            "{f:?}"
        );
    }

    #[test]
    fn idle_main_replaces_only_the_definitions_given_it() {
        // A second definition of the subject's file whose Main returns too: the subject's
        // key is not its, so it still stops the case, naming it.
        let dir = scratch("idle-main-sibling");
        let (_, beta) = two_definitions_with(&dir, "main {\n        return true;\n    }");
        let path = dir.join("sibling.test.toml");
        std::fs::write(
            &path,
            format!(
                "[subject]\niff = \"two.iff\"\nidle_main = true\n\n[sim]\n\n[[test]]\nname = \"alpha\"\ninteraction = \"Alpha\"\nexpect = {{ result = \"ok\", attrs = {{ 0 = 1 }} }}\n\n[[test]]\nname = \"alpha beside beta\"\ninteraction = \"Alpha\"\ngiven = {{ create = [{beta}] }}\nexpect = {{ result = \"ok\", attrs = {{ 0 = 1 }} }}\n"
            ),
        )
        .unwrap();
        let r = run_test_file(&TestFile::read(&path).unwrap(), &path).unwrap();
        let alpha =
            "subject Main (OBJD #128, GUID 0x7e007e10) replaced by an idle loop (idle_main)";
        assert!(failures(&r, "alpha").is_empty(), "{r:#?}");
        for c in &r.cases {
            assert_eq!(c.assumptions, vec![alpha.to_owned()], "{}", c.name);
        }
        let f = failures(&r, "alpha beside beta");
        assert!(
            f.len() == 1 && f[0].contains("object 3, which has no thread to run"),
            "{f:?}"
        );

        // An `also` object given the key that no case makes is not reported, and its Main
        // (which loops) is not an error.
        let r = run_spec(
            "idle-main-unused-also",
            &[("returning.antics", RETURNING), ("probe.antics", PROBE)],
            "[subject]\nantics = \"returning.antics\"\nidle_main = true\n\n[lot]\nalso = [{ antics = \"probe.antics\", idle_main = true }]\n\n[sim]\n\n[[test]]\nname = \"poke\"\ninteraction = \"Poke\"\nexpect = { result = \"ok\", attrs = { count = 1 } }\n",
        );
        assert_all_pass(&r);
        assert_eq!(
            r.cases[0].assumptions,
            vec![
                "subject Main (OBJD #128, GUID 0x7e007e21) replaced by an idle loop (idle_main)"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn the_open_pack_and_fixture_specs_pass() {
        let mut files = collect_test_files(&open_pack().join("tests")).unwrap();
        files.extend(
            collect_test_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test_runner"))
                .unwrap(),
        );
        assert!(files.len() >= 5, "{files:?}");
        for f in files {
            let spec = TestFile::read(&f).unwrap();
            let r = run_test_file(&spec, &f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            let bad: Vec<_> = r.cases.iter().filter(|c| !c.passed()).collect();
            assert!(bad.is_empty(), "{}: {bad:#?}", f.display());
        }
    }
}
