#[test]
#[ignore = "requires original zoning resources"]
fn original_zoning_sources_agree_on_the_initial_residential_homes() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
    let index = sims_content::Index::scan(&root).unwrap();
    for source in [
        "GameData_ALL_Recursive/MasterLotZoning.iff",
        "TemplateUserData/LotZoning.iff",
        "TemplateCommunity/LotZoning.iff",
        "TemplateStudiotown/LotZoning.iff",
        "TemplateMagicTown/LotZoning.iff",
    ] {
        let zoning = sims_content::zoning::Zoning::load(&index, source, 0xff).unwrap();
        let queries = zoning.query_table();
        for id in [5, 7, 9] {
            assert_eq!(queries.get(&id), Some(&1), "{source} lot {id}");
        }
        assert_eq!(queries.get(&90), Some(&1));
        assert_eq!(queries.get(&98), Some(&2));
    }
}
