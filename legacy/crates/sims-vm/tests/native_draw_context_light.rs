use sims_vm::scheduler::clock::draw::context::{Frame, Getter, LightUpdate, Reply};
fn hex(v: &serde_json::Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|x| u8::from_str_radix(std::str::from_utf8(x).unwrap(), 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_light_lower_and_upper_stores_match_every_word_and_cold_cut() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-draw-context-light.json")).unwrap();
    assert_eq!(
        v["bounds"],
        serde_json::json!([0x3f000000u32, 0x3f800000u32])
    );
    assert_eq!(v["cases"].as_array().unwrap().len(), 1024);
    for c in v["cases"].as_array().unwrap() {
        let mut words: [u32; 29] = hex(&c["input"]["raw"])
            .chunks_exact(4)
            .map(|x| u32::from_le_bytes(x.try_into().unwrap()))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();
        let mut f = LightUpdate::new(c["input"]["value"].as_u64().unwrap() as u32);
        for row in c["cuts"].as_array().unwrap() {
            let (mut cold, mut cold_words): (LightUpdate, [u32; 29]) =
                serde_json::from_slice(&serde_json::to_vec(&(&f, &words)).unwrap()).unwrap();
            assert!(f.advance_words(&mut words).unwrap());
            assert!(cold.advance_words(&mut cold_words).unwrap());
            assert_eq!(f, cold);
            assert_eq!(words, cold_words);
            assert!(f.at_native_cut(row["pc"].as_u64().unwrap() as u32));
            assert_eq!(
                words
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
                hex(&row["raw"])
            );
        }
        assert!(f.complete());
        assert!(!f.advance_words(&mut words).unwrap());
    }
}
#[test]
fn live_light_setter_retains_constructor_guards_and_binary_restore() {
    let mut frame = Frame::new(0x12345678);
    let mut light = LightUpdate::new(0x7f800000);
    assert!(frame.advance_light(&mut light).is_err());
    while !frame.complete() {
        let request = frame.advance().unwrap().unwrap();
        let reply = if request.getter == Getter::Origin {
            Reply::Origin([1, 2])
        } else {
            Reply::Word(7)
        };
        frame.acknowledge(request, reply).unwrap();
    }
    assert_eq!(frame.state().words()[18], 0x3f000000);
    for expected in [0x7f800000, 0x3f800000] {
        let (mut cold, mut child): (Frame, LightUpdate) =
            serde_json::from_slice(&serde_json::to_vec(&(&frame, &light)).unwrap()).unwrap();
        assert!(frame.advance_light(&mut light).unwrap());
        assert!(cold.advance_light(&mut child).unwrap());
        assert_eq!(frame, cold);
        assert_eq!(light, child);
        assert_eq!(frame.state().words()[18], expected);
        frame.validate().unwrap();
    }
    let mut bad = serde_json::to_value(&frame).unwrap();
    bad["state"]["words"][0] = serde_json::json!(1);
    let bad: Frame = serde_json::from_value(bad).unwrap();
    assert!(bad.validate().is_err());
    let mut bad = serde_json::to_value(&frame).unwrap();
    bad["live_writes"] = serde_json::json!(1<<7);
    let bad: Frame = serde_json::from_value(bad).unwrap();
    assert!(bad.validate().is_err());
}
#[test]
fn corrupted_suspended_light_store_is_rejected_before_mutation() {
    let mut words = [0; 29];
    let mut light = LightUpdate::new(0x3f800000);
    light.advance_words(&mut words).unwrap();
    words[18] = 0x3f400000;
    let previous = (words, light.clone());
    assert!(light.advance_words(&mut words).is_err());
    assert_eq!(previous, (words, light));
    let mut bad: LightUpdate =
        serde_json::from_value(serde_json::json!({"value":0x3f800000,"next":3})).unwrap();
    let previous = (words, bad.clone());
    assert!(bad.advance_words(&mut words).is_err());
    assert_eq!(previous, (words, bad));
}
