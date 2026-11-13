use sims_vm::Object;
use std::collections::BTreeMap;

#[test]
fn relationship_insertion_order_survives_replacement_growth_and_serialization() {
    let mut o = Object {
        relationship_order: Some(vec![]),
        ..Default::default()
    };
    for id in [9, 2, 7] {
        assert!(o.insert_relationship(id, vec![1]).is_none());
    }
    assert_eq!(o.insert_relationship(2, vec![3, 4]), Some(vec![1]));
    assert_eq!(o.relationship_keys().unwrap().as_ref(), [9, 2, 7]);
    let restored: Object = serde_json::from_slice(&serde_json::to_vec(&o).unwrap()).unwrap();
    assert_eq!(restored, o);
    for order in [vec![2, 7], vec![9, 2, 2], vec![9, 2, 8]] {
        o.relationship_order = Some(order);
        assert!(o.relationship_keys().is_err());
    }
}

#[test]
fn legacy_order_is_only_recovered_when_unambiguous() {
    let mut o = Object::default();
    assert!(o.relationship_keys().unwrap().is_empty());
    o.relationships.insert(8, vec![1]);
    assert_eq!(o.relationship_keys().unwrap().as_ref(), [8]);
    o.insert_relationship(3, vec![]);
    assert_eq!(o.relationship_keys().unwrap().as_ref(), [8, 3]);
    let mut unknown = Object {
        relationships: BTreeMap::from([(8, vec![1]), (3, vec![1])]),
        ..Default::default()
    };
    assert!(unknown.relationship_keys().is_err());
    unknown.insert_relationship(10, vec![1]);
    assert!(unknown.relationship_order.is_none());
    assert!(unknown.relationship_keys().is_err());
}
