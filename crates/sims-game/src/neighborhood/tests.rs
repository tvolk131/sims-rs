use super::*;

#[test]
fn neighborhood_id_maps_slots_one_through_eight() {
    assert_eq!(neighborhood_id_for_slot(0), "UserData");
    assert_eq!(neighborhood_id_for_slot(1), "UserData");
    assert_eq!(neighborhood_id_for_slot(2), "UserData2");
    assert_eq!(neighborhood_id_for_slot(8), "UserData8");
    assert_eq!(neighborhood_id_for_slot(99), "UserData8");
}

#[test]
fn street_names_formats_assigned_lot_addresses() {
    let streets = StreetNames {
        streets: vec!["%s Sim Lane".into(), "%s Ridge Road".into()],
        assignments: vec![Some(0), Some(1), None],
    };
    assert_eq!(streets.address_for(1).as_deref(), Some("1 Sim Lane"));
    assert_eq!(streets.address_for(2).as_deref(), Some("2 Ridge Road"));
    assert_eq!(streets.address_for(3), None);
    assert_eq!(streets.address_for(0), None);
}

#[test]
fn vacant_lot_thumbnail_draws_non_key_grass_diamond() {
    let thumb = vacant_lot_thumbnail(64, 48);
    assert_eq!((thumb.width, thumb.height), (64, 48));
    assert!(thumb.pixels.iter().any(|&p| p != COLOR_KEY_565));
    assert_eq!(thumb.pixels[0], COLOR_KEY_565);
}
