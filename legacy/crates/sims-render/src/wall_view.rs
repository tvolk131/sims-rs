//! Read-only consumer of recovered native wall-flag calculation. The renderer
//! derives pending presentation flags on a small private graph, never clears a
//! retained dirty byte or acknowledges the original caller's tile callbacks.
use anyhow::{Context, Result, ensure};
use sims_runtime::original_session::Session;
use sims_sim::lot_presenter::{
    lifecycle,
    walls::{self, construction::Graph, cutaway},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub style: u8,
    pub flags: u8,
}
type Key = (i32, [[i32; 2]; 2]);
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    segments: BTreeMap<Key, Segment>,
    objects: Option<crate::wall_objects::View>,
}
fn key(level: i32, a: [i32; 2], b: [i32; 2]) -> Key {
    (level, if a < b { [a, b] } else { [b, a] })
}
const DIRECTIONS: [[i32; 2]; 8] = [
    [0, -1],
    [0, 1],
    [-1, 0],
    [1, 0],
    [-1, -1],
    [1, 1],
    [1, -1],
    [-1, 1],
];
fn unrotate([x, y, z]: [i8; 3], rotation: u8) -> [i32; 3] {
    let (x, y) = (i32::from(x), i32::from(y));
    let [x, y] = match rotation {
        1 => [y, 64 - x],
        2 => [64 - x, 64 - y],
        3 => [64 - y, x],
        _ => [x, y],
    };
    [x, y, i32::from(z)]
}
impl Plan {
    pub fn from_session(session: &Session) -> Result<Option<Self>> {
        let Some(p) = session.host.presentation.presenter.as_ref() else {
            return Ok(None);
        };
        let Some(camera) = p.camera.as_ref() else {
            return Ok(None);
        };
        let Some(roof) = camera.settings.roof else {
            // Initialization has not supplied the native roof-mode input yet.
            return Ok(None);
        };
        Self::prepare(&camera.graph, &p.view.floor, roof, p.view.floor.level).map(Some)
    }
    ///4a8895's wall passes on renderer-owned records. The supplied mask/modes
    ///remain explicit inputs; this does not reconstruct them from camera angles.
    pub fn prepare(source: &Graph, view: &lifecycle::State, roof: u8, level: i32) -> Result<Self> {
        ensure!((1..=2).contains(&level), "wall view floor unavailable");
        view.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            view.level == level,
            "wall view floor disagrees with draw floor"
        );
        let mut graph = source.clone();
        graph.validate().map_err(anyhow::Error::msg)?;
        if view.visibility.dirty != 0 {
            let history = cutaway::History::capture(&graph).map_err(anyhow::Error::msg)?;
            for floor in 1..=level {
                if floor == level
                    && roof == 0
                    && view.all_walls == 0
                    && view.visibility.enabled != 0
                {
                    // begin performs all flag/cap calculations before issuing
                    // invalidation callbacks. No callback is serviced here.
                    let _ =
                        cutaway::Cursor::begin(&mut graph, &history, &view.visibility.mask, floor)
                            .map_err(anyhow::Error::msg)?;
                } else {
                    let enabled = u8::from(floor == level && roof == 0 && view.all_walls != 0);
                    graph
                        .state
                        .update(enabled, floor, walls::BASIS, |_| {})
                        .map_err(anyhow::Error::msg)?;
                }
            }
        }
        let mut plan = Self::from_graph(&graph)?;
        plan.objects = Some(crate::wall_objects::View::new(view, graph.rotation));
        Ok(plan)
    }
    pub fn from_graph(graph: &Graph) -> Result<Self> {
        graph.validate().map_err(anyhow::Error::msg)?;
        let mut segments = BTreeMap::new();
        for e in &graph.state.edges {
            let mut start = graph.geometry.positions[e.ends[0]].map(i32::from);
            for s in &e.segments {
                let d = DIRECTIONS[e.kind as usize];
                let end = [start[0] + d[0], start[1] + d[1], start[2]];
                let a = unrotate(start.map(|n| n as i8), graph.rotation);
                let b = unrotate(end.map(|n| n as i8), graph.rotation);
                ensure!(
                    segments
                        .insert(
                            key(a[2], [a[0], a[1]], [b[0], b[1]]),
                            Segment {
                                style: s.style,
                                flags: s.flags
                            }
                        )
                        .is_none(),
                    "duplicate wall presentation segment"
                );
                start = end;
            }
        }
        Ok(Self {
            segments,
            objects: None,
        })
    }
    pub(crate) fn hides_object(&self, sim: &sims_sim::Simulation, id: i16) -> Result<bool> {
        let Some(view) = &self.objects else {
            return Ok(false);
        };
        let o = &sim.vm.world.objects[&id];
        let flags = o.data.get(&8).copied().unwrap_or(0) as u16;
        if flags & 0x400 == 0 {
            return Ok(false);
        }
        let position = sim
            .vm
            .world
            .positions
            .get(&id)
            .context("wall object physical position absent")?;
        let tile = crate::wall_objects::rotate(
            sims_vm::loading::coarse_position(*position),
            view.rotation,
        );
        // This client has no wall-editing tool yet (native4cea33).
        view.hidden(
            tile,
            flags,
            o.data.get(&13).copied().unwrap_or(0) as u16,
            o.data.get(&1).copied().unwrap_or(0),
            false,
        )
    }
    pub fn segment(&self, level: i32, a: [i32; 2], b: [i32; 2]) -> Result<Segment> {
        self.segments
            .get(&key(level, a, b))
            .copied()
            .context("drawn wall absent from native graph")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn view(mask: cutaway::Mask, level: i32) -> lifecycle::State {
        lifecycle::State {
            level,
            all_walls: 0,
            history: sims_sim::lot_presenter::hover::History::house_view(),
            visibility: sims_sim::lot_presenter::visibility::State {
                enabled: 1,
                dirty: 1,
                include_visible: 0,
                mask,
            },
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn pending_cutaway_matches_native_fixture_without_changing_retained_records() {
        let report: serde_json::Value = serde_json::from_str(include_str!(
            "../../sims-sim/tests/fixtures/wall-cutaway.json"
        ))
        .unwrap();
        let c = &report["cases"][1]; // no callback-time fixture mutations
        let graph: Graph = serde_json::from_value(c["input"]["graph"].clone()).unwrap();
        let expected: Graph = serde_json::from_value(c["original"]["graph"].clone()).unwrap();
        // Oracle storage includes guard rows; actual view+208 owns rows0..64.
        let mut mask: cutaway::Mask = serde_json::from_value(c["input"]["mask"].clone()).unwrap();
        mask.rows = mask.rows[(-mask.first_row) as usize..(-mask.first_row) as usize + 64].to_vec();
        mask.first_row = 0;
        let v = view(mask, 1);
        let saved = serde_json::to_vec(&(&graph, &v)).unwrap();
        assert_eq!(
            Plan::prepare(&graph, &v, 0, 1).unwrap().segments,
            Plan::from_graph(&expected).unwrap().segments
        );
        assert_eq!(saved, serde_json::to_vec(&(&graph, &v)).unwrap());
        let mut clean = v.clone();
        clean.visibility.dirty = 0;
        assert_eq!(
            Plan::prepare(&graph, &clean, 0, 1).unwrap().segments,
            Plan::from_graph(&graph).unwrap().segments
        );
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn mode_and_roof_passes_match_native_flags_and_rotation_preserves_cell_identity() {
        let report: serde_json::Value = serde_json::from_str(include_str!(
            "../../sims-sim/tests/fixtures/wall-cutaway.json"
        ))
        .unwrap();
        let source: Graph =
            serde_json::from_value(report["cases"][0]["input"]["graph"].clone()).unwrap();
        let original = Plan::from_graph(&source).unwrap();
        for rotation in 0..4 {
            let mut graph = source.clone();
            graph.rotate(rotation).unwrap();
            assert_eq!(original, Plan::from_graph(&graph).unwrap());
            for (all_walls, roof, enabled) in [(0, 0, 0), (1, 0, 1), (1, 1, 0)] {
                let mut v = view(
                    cutaway::Mask {
                        first_row: 0,
                        rows: vec![0; 64],
                    },
                    1,
                );
                v.visibility.enabled = 0;
                v.all_walls = all_walls;
                let mut expected = graph.clone();
                expected
                    .state
                    .update(enabled, 1, walls::BASIS, |_| {})
                    .unwrap();
                assert_eq!(
                    Plan::prepare(&graph, &v, roof, 1).unwrap().segments,
                    Plan::from_graph(&expected).unwrap().segments
                );
            }
        }
    }
}
