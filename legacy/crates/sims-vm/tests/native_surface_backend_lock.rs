use sims_vm::scheduler::clock::draw::targets::backend::{Fields, Lock, Request};
fn hex(v: &serde_json::Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_backend_nesting_descriptor_writes_retry_limits_and_cold_cuts() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-surface-backend-lock.json")).unwrap();
    assert_eq!(v["cases"].as_array().unwrap().len(), 512);
    for (i, c) in v["cases"].as_array().unwrap().iter().enumerate() {
        let mut fields = Fields {
            raw: hex(&c["input"]["raw"]),
        };
        let mut frame = Lock::new();
        let mut calls = 0;
        for _ in 0..64 {
            let mut cold: Lock =
                serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            let mut cf = fields.clone();
            cold.validate(&cf).unwrap();
            if let Some(req) = frame.pending().cloned() {
                match req {
                    Request::DirectDrawLock {
                        interface,
                        descriptor_offset,
                        descriptor,
                        flags,
                        event,
                    } => {
                        let e = &c["events"][calls];
                        assert_eq!(e["kind"], "DirectDrawLock");
                        assert_eq!(e["interface"], interface);
                        assert_eq!(e["descriptor_offset"], descriptor_offset);
                        assert_eq!(flags, 1);
                        assert_eq!(event, 0);
                        assert_eq!(descriptor, hex(&e["before"]));
                        let result = e["hresult"].as_u64().unwrap() as u32;
                        let after = hex(&e["after"]);
                        frame
                            .directdraw_return(&mut fields, result, &after)
                            .unwrap();
                        cold.directdraw_return(&mut cf, result, &after).unwrap();
                        calls += 1;
                    }
                    Request::Restore => {
                        let e = &c["events"][calls];
                        assert_eq!(e["kind"], "Restore");
                        let r = e["result"].as_u64().unwrap() as u32;
                        frame.restore_return(&fields, r).unwrap();
                        cold.restore_return(&cf, r).unwrap();
                        calls += 1;
                    }
                    Request::Error { hresult } => {
                        assert_eq!(c["expected"]["error"], hresult, "case{i} error child");
                        break;
                    }
                    Request::DirectDrawUnlock { .. } => panic!("lock cursor requested an unlock"),
                }
            } else {
                assert_eq!(
                    frame.advance(&mut fields).unwrap(),
                    cold.advance(&mut cf).unwrap()
                );
            }
            assert_eq!(frame, cold, "case{i} saved cursor");
            assert_eq!(fields, cf, "case{i} saved fields");
            if frame.complete() {
                break;
            }
        }
        assert!(
            frame.complete() || matches!(frame.pending(), Some(Request::Error { .. })),
            "case{i} stalled"
        );
        assert_eq!(calls, c["events"].as_array().unwrap().len());
        assert_eq!(fields.raw, hex(&c["expected"]["raw"]), "case{i} fields");
        assert_eq!(
            serde_json::json!(frame.result()),
            c["expected"]["result"],
            "case{i} result"
        );
    }
}
