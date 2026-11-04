use sims_vm::dialog::{GlobalBitmap, NamedIcon, named_icon};

// Expectations recovered from 436552..436933 and 46f891. These are static
// instruction-derived checks, not newly executed original-engine cases.
#[test]
fn named_global_icons_preserve_identity_crop_and_crt_number_rules() {
    for (text, id, button_cell) in [
        ("gz 4500", 4500, true),
        ("GZ 4 5 0 0", 4500, true),
        ("gzi 4500", 4500, false),
        ("GzI \t+4500tail", 4500, false),
        ("gz -1", -1, true),
        ("gz 4294967297", 1, true),
    ] {
        assert_eq!(
            named_icon(text).unwrap(),
            Some(NamedIcon::Global(GlobalBitmap { id, button_cell })),
            "{text}"
        );
    }
    for text in ["", "gz", "gzi zero", "gz 0", "gz 4294967296"] {
        assert_eq!(named_icon(text).unwrap(), None, "{text}");
    }
    for (text, job) in [("JOB -1", -1), ("job", 0), ("Job 2 1", 21)] {
        assert_eq!(named_icon(text).unwrap(), Some(NamedIcon::Job(job)));
    }
    assert_eq!(named_icon("job 22").unwrap(), None);
    for text in ["gz\t4500", "my 5000", "guid 123"] {
        assert!(
            named_icon(text).is_err(),
            "unsupported categories must stay explicit: {text}"
        );
    }
}
