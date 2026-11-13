use sims_formats::avatar::{MeshBinding, read_bcf, read_bmf, read_skn};
fn s(b: &mut Vec<u8>, s: &str) {
    b.push(s.len() as u8);
    b.extend(s.bytes());
}
fn u(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes());
}
fn f(b: &mut Vec<u8>, v: f32) {
    b.extend(v.to_le_bytes());
}
fn skeleton(parent: &str) -> Vec<u8> {
    let mut b = vec![];
    u(&mut b, 1);
    s(&mut b, "test");
    u(&mut b, 1);
    s(&mut b, "ROOT");
    s(&mut b, parent);
    u(&mut b, 0);
    for v in [0., 0., 0., 0., 0., 0., -1.] {
        f(&mut b, v);
    }
    for v in [1, 1, 0] {
        u(&mut b, v);
    }
    f(&mut b, 0.);
    f(&mut b, 0.);
    u(&mut b, 0);
    u(&mut b, 0);
    b
}
fn mesh() -> Vec<u8> {
    let mut b = vec![];
    s(&mut b, "skin");
    s(&mut b, "tex");
    u(&mut b, 1);
    s(&mut b, "ROOT");
    u(&mut b, 1);
    for i in [0, 1, 2] {
        u(&mut b, i);
    }
    u(&mut b, 1);
    for i in [0, 0, 3, 0, 1] {
        u(&mut b, i);
    }
    u(&mut b, 3);
    for _ in 0..6 {
        f(&mut b, 0.);
    }
    u(&mut b, 1);
    u(&mut b, 16384);
    u(&mut b, 1);
    u(&mut b, 4);
    for _ in 0..4 {
        for v in [0., 1., 2., 0., 1., 0.] {
            f(&mut b, v);
        }
    }
    b
}
#[test]
fn skeleton_hierarchy_and_truncations() {
    let data = skeleton("NULL");
    let c = read_bcf(&data).unwrap();
    assert_eq!(c.skeletons[0].bones[0].parent, None);
    assert_eq!(c.skeletons[0].bones[0].rotation, [0., 0., 0., -1.]);
    for scale in [1, 400_000, 700_000, -1, i32::MIN] {
        let mut noise = data.clone();
        let at = noise.len() - 16;
        noise[at..at + 4].copy_from_slice(&scale.to_le_bytes());
        noise[at + 4..at + 8].copy_from_slice(&0.05_f32.to_le_bytes());
        let b = read_bcf(&noise)
            .unwrap()
            .skeletons
            .remove(0)
            .bones
            .remove(0);
        assert_eq!(b.wiggle.gradient_scale, scale);
        assert_eq!(b.wiggle.amplitude, 0.05);
        noise[at + 4..at + 8].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(
            read_bcf(&noise).is_err(),
            "amplitude still requires a finite float"
        );
    }
    for end in 0..data.len() {
        assert!(read_bcf(&data[..end]).is_err(), "{end}");
    }
    assert!(read_bcf(&skeleton("missing")).is_err());
    assert!(read_bcf(&skeleton("ROOT")).is_err());
    let mut tail = data.clone();
    tail.push(0);
    assert!(read_bcf(&tail).is_err());
    let mut oversized = data.clone();
    oversized[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read_bcf(&oversized).is_err());
}
#[test]
fn mesh_keeps_local_coordinates_blends_and_checks_every_truncation() {
    let data = mesh();
    let m = read_bmf(&data).unwrap();
    assert_eq!(m.vertices.len(), 3);
    assert_eq!(m.blends[0].vertex, 1);
    assert_eq!(m.blends[0].weight, 0.5);
    assert_eq!(m.vertices[2].position, [0., 1., 2.]);
    assert_eq!(m.vertices[2].bone, 0);
    assert_eq!(
        m.bindings,
        vec![MeshBinding {
            bone: 0,
            first_vertex: 0,
            vertex_count: 3,
            first_blend: 0,
            blend_count: 1,
        }]
    );
    for end in 0..data.len() {
        assert!(read_bmf(&data[..end]).is_err(), "{end}");
    }
    let mut bad = data.clone();
    let len = bad.len();
    bad[len - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(read_bmf(&bad).is_err());
    let mut bad = data.clone();
    bad.push(0);
    assert!(read_bmf(&bad).is_err());
    // The triangle's last index lies outside the three real vertices.
    let mut bad = data;
    let at = 1 + 4 + 1 + 3 + 4 + 1 + 4 + 4 + 8;
    bad[at..at + 4].copy_from_slice(&3u32.to_le_bytes());
    assert!(read_bmf(&bad).is_err());
}

#[test]
fn mesh_preserves_empty_binding_rows_and_sentinel_offsets() {
    let mut data = mesh();
    let count_at = 1 + 4 + 1 + 3 + 4 + 1 + 4 + 4 + 12;
    data[count_at..count_at + 4].copy_from_slice(&2_u32.to_le_bytes());
    let mut empty = Vec::new();
    for value in [0, u32::MAX, 0, u32::MAX, 0] {
        u(&mut empty, value);
    }
    data.splice(count_at + 4..count_at + 4, empty);
    let parsed = read_bmf(&data).unwrap();
    assert_eq!(parsed.bindings.len(), 2);
    assert_eq!(
        parsed.bindings[0],
        MeshBinding {
            bone: 0,
            first_vertex: u32::MAX as usize,
            vertex_count: 0,
            first_blend: u32::MAX as usize,
            blend_count: 0,
        }
    );
    assert_eq!(parsed.bindings[1].vertex_count, 3);
    assert_eq!(parsed.vertices.len(), 3);
}

fn skn(texture: &str) -> String {
    format!(
        "cart\r\n{texture}\r\n1\r\nROOT\r\n1\r\n0 1 2\r\n1\r\n0 0 3 0 0\r\n3\r\n0 0\r\n1 0\r\n0 1\r\n0\r\n3\r\n\
         0 0 0 0 0 1\r\n1 0 0 0 0 1\r\n0 1 0 0 0 1\r\n"
    )
}

#[test]
fn skn_texture_line_may_be_empty() {
    let mesh = read_skn(skn("cart-tex").as_bytes()).unwrap();
    assert_eq!(mesh.texture, "cart-tex");
    let mesh = read_skn(skn("").as_bytes()).unwrap();
    assert_eq!(mesh.texture, "");
    assert_eq!(mesh.bones, ["ROOT"]);
    assert_eq!(mesh.triangles, [[0, 1, 2]]);
    assert_eq!(mesh.vertices.len(), 3);
    assert_eq!(mesh.vertices[1].position, [1.0, 0.0, 0.0]);
}
