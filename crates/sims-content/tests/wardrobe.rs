use sims_content::{Assets, Wardrobe};

/// Needs the game: set `SIMS_INSTALL` to the install folder.
///
/// The Complete Collection's body and head counts for each sex, age and
/// skin tone (PersonGlobals' STR# 8192 is "unknown", "lgt", "drk",
/// "med"), counted from the original's patterns over every suit and
/// texture name (track 4's suit-filter research). None of the three real
/// tones gives an empty list; "unknown" gives no body or head.
#[test]
fn the_complete_collections_counts() {
    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL to read the install's suits");
        return;
    };
    let assets = Assets::scan(std::path::Path::new(&root)).unwrap();
    let wardrobe = Wardrobe::load(&assets);
    assert_eq!(wardrobe.errors, Default::default());
    // (head, female, child): lgt, med, drk.
    let table = [
        ((false, false, false), [143, 94, 97]),
        ((false, true, false), [121, 105, 106]),
        ((false, false, true), [17, 17, 18]),
        ((false, true, true), [17, 17, 17]),
        ((true, false, false), [137, 84, 78]),
        ((true, true, false), [107, 84, 80]),
        ((true, false, true), [22, 20, 19]),
        ((true, true, true), [26, 24, 23]),
    ];
    for ((head, female, child), counts) in table {
        let found = ["lgt", "med", "drk"].map(|tone| wardrobe.count(head, female, child, tone));
        assert_eq!(found, counts, "head {head}, female {female}, child {child}");
        assert_eq!(wardrobe.count(head, female, child, "unknown"), 0);
    }
}
