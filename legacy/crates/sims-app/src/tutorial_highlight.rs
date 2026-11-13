//! Original5de13f positions the shared50px tutorial indicator above a control.
//! These helpers paint only; they never complete the UI-effect owner or input.
use super::*;

pub(super) fn origin(position: [i32; 2], target: [i32; 4], marker: [i32; 4]) -> [i32; 2] {
    let difference = target[2]
        .wrapping_sub(marker[2])
        .wrapping_sub(target[0])
        .wrapping_add(marker[0]);
    [
        position[0].wrapping_add(difference / 2),
        position[1]
            .wrapping_sub(3)
            .wrapping_add(marker[1].wrapping_sub(marker[3])),
    ]
}

pub(crate) fn layer(
    icons: &dialog_view::StaticArt,
    position: [i32; 2],
    target_size: [u32; 2],
) -> Element<'static, Message> {
    let [x, y] = origin(
        position,
        [0, 0, target_size[0] as i32, target_size[1] as i32],
        [0, 0, 50, 50],
    );
    at(
        widget::image(icons.tutorial()).width(50).height(50).into(),
        x as f32,
        y as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn geometry_matches_unchanged_native_highlight_corpus() -> Result<()> {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/tutorial-highlight.json"))?;
        let mut moved = 0;
        for row in data["cases"].as_array().unwrap() {
            let c = &row["input"];
            let position: [i32; 2] = serde_json::from_value(c["position"].clone())?;
            let target: [i32; 4] = serde_json::from_value(c["target_rect"].clone())?;
            let marker: [i32; 4] = serde_json::from_value(c["highlight_rect"].clone())?;
            for e in row["original"]["events"].as_array().unwrap() {
                if e[0] == "move" {
                    ensure!(
                        origin(position, target, marker)
                            == [e[1].as_i64().unwrap() as i32, e[2].as_i64().unwrap() as i32]
                    );
                    moved += 1;
                }
            }
        }
        ensure!(moved > 10);
        // Native800x600 ZoomIn control plus the shipped marker's red mask.
        let marker = origin([26, 522], [0, 0, 27, 27], [0, 0, 50, 50]);
        ensure!(marker == [15, 469]);
        ensure!(
            [
                marker[0] + 11,
                marker[1] + 5,
                marker[0] + 38,
                marker[1] + 35
            ] == [26, 474, 53, 504]
        );
        Ok(())
    }
}
