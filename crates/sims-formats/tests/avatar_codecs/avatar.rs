use sims_formats::avatar::{MeshBinding, read_bcf, read_bmf, read_cmx, read_skn};
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

/// The SKN reader (0x59a629) skips empty lines before the name, the
/// texture and each bone name (0x59a676, 0x59a6db, 0x59a769). An empty
/// texture line is skipped, so the bone count becomes the texture and the
/// count is then read from the bone name, which fails.
#[test]
fn skn_skips_empty_name_lines_so_an_empty_texture_line_fails() {
    let mesh = read_skn(skn("cart-tex").as_bytes()).unwrap();
    assert_eq!(mesh.name, "cart");
    assert_eq!(mesh.texture, "cart-tex");
    assert_eq!(mesh.bones, ["ROOT"]);
    assert_eq!(mesh.triangles, [[0, 1, 2]]);
    assert_eq!(mesh.vertices.len(), 3);
    assert_eq!(mesh.vertices[1].position, [1.0, 0.0, 0.0]);

    let error = read_skn(skn("").as_bytes()).unwrap_err();
    assert_eq!(error.message, "invalid avatar integer");
    // At the end of the bone name the count was read from.
    let text = skn("");
    assert_eq!(error.offset, text.find("ROOT").unwrap() + 4);

    // Empty lines before the name, the texture and a bone are skipped.
    let padded = skn("cart-tex")
        .replacen("cart\r\n", "\r\n\ncart\r\n", 1)
        .replacen("cart-tex\r\n", "\r\ncart-tex\r\n", 1)
        .replacen("1\r\nROOT", "1\r\n\r\nROOT", 1);
    let mesh = read_skn(padded.as_bytes()).unwrap();
    assert_eq!(mesh.name, "cart");
    assert_eq!(mesh.texture, "cart-tex");
    assert_eq!(mesh.bones, ["ROOT"]);

    // A file that ends in empty lines has no texture.
    assert!(read_skn(b"skin\n\n").is_err());
    assert!(read_skn(b"skin\r\n\r\n\r\n").is_err());
}

/// What the text readers reject: an unknown CMX version, appearance or
/// binding properties, NUL, trailing data; SKN indices out of range, a
/// non-finite float, a binding past the vertices, a blend weight over
/// 32768, trailing data, a cut-off file and a count too large to read.
#[test]
fn text_avatars_reject_invalid_or_unrepresented_data() {
    let cmx = "// fixture\nversion 300\n0\n1\nbody\n0\n0\n1\nPELVIS\nskin\n0\n0\n0\n";
    assert_eq!(read_cmx(cmx.as_bytes()).unwrap().appearances.len(), 1);
    for bad in [
        cmx.replace("version 300", "version 301"),
        cmx.replace("body\n0\n0", "body\n0\n1"),
        cmx.replace("skin\n0\n0\n0", "skin\n0\n1\n0"),
        cmx.replace("PELVIS", "PE\0LVIS"),
        format!("{cmx}unexpected"),
    ] {
        assert!(read_cmx(bad.as_bytes()).is_err(), "{bad:?}");
    }

    let skn = "skin\r\ntex\r\n1\r\nROOT\r\n1\r\n0 1 2\r\n1\r\n0 0 3 0 1\r\n3\r\n\
               -0.25 0\r\n1 0\r\n0 1\r\n1\r\n0 32768\r\n4\r\n\
               0 0 0 0 0 1\r\n1 0 0 0 0 1\r\n0 1 0 0 0 1\r\n0 0 1 0 0 1\r\n";
    let mesh = read_skn(skn.as_bytes()).unwrap();
    assert_eq!(mesh.vertices[0].uv, [-0.25, 0.0]);
    assert_eq!(mesh.blends.len(), 1);
    for bad in [
        skn.replace("0 1 2", "0 1 999"),
        skn.replace("-0.25", "NaN"),
        skn.replace("-0.25", "inf"),
        skn.replace("0 0 3 0 1", "0 0 300 0 1"),
        skn.replace("0 32768", "0 32769"),
        skn.replace("0 32768", "999 32768"),
        format!("{skn}unexpected"),
        skn[..skn.len() / 2].to_owned(),
        skn.replace("skin", "sk\0in"),
    ] {
        assert!(read_skn(bad.as_bytes()).is_err(), "{bad:?}");
    }
    // Counts are checked before anything is allocated for them, and a line
    // read that runs out of file ends the read.
    assert!(read_skn(b"skin\ntex\n2147483647\n").is_err());
    assert!(read_skn(b"skin\ntex\n-1\n").is_err());
    assert!(read_skn(b"skin\n\n").is_err());
}

/// Every SKN of an install, loose or in a top-level FAR, either reads or
/// has an empty texture line and fails at its bone count, as the original
/// reader does (0x59a6db). Set `SIMS_INSTALL` to run it.
#[test]
fn installed_skns_fail_only_for_an_empty_texture_line() {
    use std::io::{Read, Seek, SeekFrom};
    use std::path::{Path, PathBuf};

    #[expect(clippy::disallowed_methods, reason = "the paths are sorted")]
    fn files(directory: &Path, out: &mut Vec<PathBuf>) {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                files(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    fn has_extension(name: &str, extension: &str) -> bool {
        name.rsplit('.')
            .next()
            .is_some_and(|x| x.eq_ignore_ascii_case(extension))
    }
    /// Whether the second non-header line, the texture, is empty.
    fn empty_texture(bytes: &[u8]) -> bool {
        let mut lines = bytes
            .split(|&b| b == b'\n')
            .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
            .skip_while(|line| line.is_empty());
        lines.next();
        lines.next().is_some_and(<[u8]>::is_empty)
    }

    let Some(root) = std::env::var_os("SIMS_INSTALL") else {
        eprintln!("skipping: set SIMS_INSTALL");
        return;
    };
    let mut paths = Vec::new();
    files(Path::new(&root), &mut paths);
    let mut meshes: Vec<(String, Vec<u8>)> = Vec::new();
    for path in &paths {
        let name = path.to_string_lossy().into_owned();
        if has_extension(&name, "skn") {
            meshes.push((name, std::fs::read(path).unwrap()));
        } else if has_extension(&name, "far") {
            let mut file = std::fs::File::open(path).unwrap();
            let length = file.metadata().unwrap().len();
            let mut header = [0; 16];
            file.read_exact(&mut header).unwrap();
            let offset = sims_formats::far::manifest_offset(&header, length).unwrap();
            file.seek(SeekFrom::Start(offset.into())).unwrap();
            let mut directory = vec![0; sims_formats::far::directory_len(offset, length).unwrap()];
            file.read_exact(&mut directory).unwrap();
            for entry in sims_formats::far::directory(&directory, length).unwrap() {
                if !has_extension(&entry.name, "skn") || entry.stored_size != entry.unpacked_size {
                    continue;
                }
                file.seek(SeekFrom::Start(entry.offset.into())).unwrap();
                let mut bytes = vec![0; entry.stored_size as usize];
                file.read_exact(&mut bytes).unwrap();
                meshes.push((format!("{name}:{}", entry.name), bytes));
            }
        }
    }
    let mut failed = 0;
    for (name, bytes) in &meshes {
        match read_skn(bytes) {
            Ok(_) => assert!(!empty_texture(bytes), "{name} reads with an empty texture"),
            Err(error) => {
                assert!(empty_texture(bytes), "{name}: {error:?}");
                assert_eq!(error.message, "invalid avatar integer", "{name}");
                failed += 1;
            }
        }
    }
    eprintln!("{} SKNs, {failed} with an empty texture line", meshes.len());
    assert!(meshes.len() > 100);
}
