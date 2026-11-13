use sims_vm::scheduler::clock::draw::targets::{Begin, Buffers, Fields, Plane, Request};
fn hex(v: &serde_json::Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn set(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_target_begin_preserves_call_order_partial_failure_and_cold_cuts() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-target-begin.json")).unwrap();
    assert_eq!(f["begin"].as_array().unwrap().len(), 1024);
    for (i, c) in f["begin"].as_array().unwrap().iter().enumerate() {
        let input = &c["input"];
        let mut world = hex(&input["world"]);
        let g: Vec<u32> = serde_json::from_value(input["globals"].clone()).unwrap();
        let mut fields = Fields {
            active: world[0x78],
            phase: word(&world, 0x7c) as i32,
            surface: word(&world, 0x80),
            buffers: Buffers {
                color: g[0],
                color_pitch: g[1],
                depth: g[2],
                depth_pitch: g[3],
            },
        };
        let mut frame = Begin::new(0x10001000).unwrap();
        let mut depth = input["depth"].as_u64().unwrap() as u32;
        let mut calls = 0;
        for _ in 0..128 {
            let mut cold: Begin =
                serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            cold.validate().unwrap();
            let mut cf = fields;
            if let Some(request) = frame.pending() {
                let e = &c["events"][calls];
                let (name, surface, flags) = match request {
                    Request::Lock { surface, flags } => ("Lock", surface, Some(flags)),
                    Request::Unlock { surface, flags } => ("Unlock", surface, Some(flags)),
                    Request::Buffer { surface } => ("Buffer", surface, None),
                    Request::Pitch { surface } => ("Pitch", surface, None),
                };
                assert_eq!(e["name"], name, "case{i} call{calls}");
                assert_eq!(e["surface"], surface);
                assert_eq!(e["flags"], serde_json::json!(flags));
                let result = e["result"].as_u64().unwrap() as u32;
                frame.acknowledge(request, result).unwrap();
                cold.acknowledge(request, result).unwrap();
                let mutation = input["mutation"].as_u64().unwrap();
                if name == "Buffer" && surface == 0x10001000 && mutation & 1 != 0 {
                    depth = 0x10002400;
                }
                if name == "Buffer" && surface != 0x10001000 && mutation & 2 != 0 {
                    depth = 0;
                }
                calls += 1;
            } else {
                let flags = input["flags"].as_u64().unwrap() as u32;
                let d = (depth != 0).then_some(depth);
                assert_eq!(
                    frame.advance(&mut fields, flags, d).unwrap(),
                    cold.advance(&mut cf, flags, d).unwrap()
                );
            }
            assert_eq!(frame, cold, "case{i} cursor cold");
            assert_eq!(fields, cf, "case{i} fields cold");
            if frame.complete() {
                break;
            }
        }
        assert!(frame.complete(), "case{i} stalled");
        assert_eq!(calls, c["events"].as_array().unwrap().len());
        world[0x78] = fields.active;
        set(&mut world, 0x7c, fields.phase as u32);
        set(&mut world, 0x80, fields.surface);
        assert_eq!(world, hex(&c["expected"]["world"]), "case{i} world");
        assert_eq!(
            [
                fields.buffers.color,
                fields.buffers.color_pitch,
                fields.buffers.depth,
                fields.buffers.depth_pitch
            ],
            serde_json::from_value::<[u32; 4]>(c["expected"]["globals"].clone()).unwrap(),
            "case{i} globals"
        );
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_plane_flags_counts_low_byte_failures_and_wrapping_match() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-target-begin.json")).unwrap();
    assert_eq!(f["planes"].as_array().unwrap().len(), 1024);
    for (i, c) in f["planes"].as_array().unwrap().iter().enumerate() {
        let v = &c["input"];
        let mut raw = hex(&v["raw"]);
        let mut p = Plane {
            buffer: word(&raw, 0x34),
            half_pitch: word(&raw, 0x38),
            locks: word(&raw, 0x3c),
            flags: word(&raw, 0x50),
        };
        p = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        let flags = v["flags"].as_u64().unwrap() as u32;
        let reply = v["backend_result"].as_u64().unwrap() as u32;
        let result = if v["unlock"].as_bool().unwrap() {
            p.unlock(flags, reply)
        } else {
            p.lock(
                flags,
                reply,
                v["buffer"].as_u64().unwrap() as u32,
                v["pitch"].as_u64().unwrap() as u32,
            )
        };
        for (o, value) in [
            (0x34, p.buffer),
            (0x38, p.half_pitch),
            (0x3c, p.locks),
            (0x50, p.flags),
        ] {
            set(&mut raw, o, value);
        }
        assert_eq!(
            result as u64,
            c["expected"]["result"].as_u64().unwrap(),
            "case{i} result"
        );
        assert_eq!(raw, hex(&c["expected"]["raw"]), "case{i} plane bytes");
        assert_eq!(
            usize::from(Plane::needs_backend(flags)),
            c["backend_calls"].as_array().unwrap().len()
        );
    }
}
