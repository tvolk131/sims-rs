use anyhow::Result;
use serde::Deserialize;
use sims_render::sprite_depth::{Rows, Surface, Table, base_depth, blit};
use sims_sim::pose::portrait::view::Input;

#[derive(Deserialize)]
struct Scale {
    input: Input,
    bits: u32,
}
#[derive(Deserialize)]
struct CaseInput {
    width: usize,
    height: usize,
    zoom: u32,
    flip: bool,
    clip: [usize; 4],
    anchor: i32,
    screen_y: i32,
    lookup: Vec<i16>,
    color_pitch: usize,
    depth_pitch: usize,
    colors: Vec<u8>,
    depths: Vec<u8>,
    palette: Vec<u16>,
    destination: Vec<u16>,
    zbuffer: Vec<u16>,
    row_results: Vec<u8>,
}
#[derive(Deserialize)]
struct Original {
    destination: Vec<u16>,
    zbuffer: Vec<u16>,
    consumed: Vec<u8>,
}
#[derive(Deserialize)]
struct Case {
    input: CaseInput,
    original: Original,
}
#[derive(Deserialize)]
struct Report {
    scales: Vec<Scale>,
    table: Vec<u16>,
    cases: Vec<Case>,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_sprite_depth_clipping_flip_and_consumption_match() -> Result<()> {
    let report: Report = serde_json::from_str(include_str!("fixtures/sprite-depth.json"))?;
    assert_eq!(report.scales.len(), 3);
    let table = Table::new(report.scales[0].bits);
    assert_eq!(table.0.as_slice(), report.table);
    for s in report.scales {
        // Original49b0de writes graphics+(8-zoom)*4 from the same far value
        // passed to its projection. These are not synthetic1/2/4 scales.
        assert_eq!(
            s.input.build().map_err(anyhow::Error::msg)?.far.to_bits(),
            s.bits
        );
        assert_eq!(Table::new(s.bits), table);
    }
    assert_eq!(report.cases.len(), 120);
    for (n, c) in report.cases.into_iter().enumerate() {
        let mut i = c.input;
        let base = base_depth(&i.lookup, i.zoom, i.screen_y, i.screen_y, i.anchor)?;
        blit(
            &table,
            base,
            Rows {
                width: i.width,
                height: i.height,
                colors: &i.colors,
                depths: &mut i.depths,
                depth_results: &i.row_results,
            },
            &i.palette.try_into().unwrap(),
            Surface {
                colors: &mut i.destination,
                depths: &mut i.zbuffer,
                color_pitch: i.color_pitch,
                depth_pitch: i.depth_pitch,
            },
            i.clip,
            i.flip,
        )?;
        assert_eq!(
            i.destination, c.original.destination,
            "case{n}: color/padding"
        );
        assert_eq!(i.zbuffer, c.original.zbuffer, "case{n}: depth/padding");
        assert_eq!(i.depths, c.original.consumed, "case{n}: row consumption");
    }
    Ok(())
}

#[test]
fn transparent_depth_ties_and_signed_wrapping_are_preserved() {
    let mut table = Table([0; 255]);
    table.0[0] = (-1i16) as u16;
    assert_eq!(table.sample(100, 1, 100), Some(100));
    assert_eq!(table.sample(101, 1, 100), None);
    assert_eq!(table.sample(0, 0, 0), Some(65535));
    assert_eq!(table.sample(0, 255, 65535), None);
    assert_eq!(table.sample(i32::MIN, 0, 65535), None);
    assert!(base_depth(&[], 3, 0, 0, 0).is_err());
}
