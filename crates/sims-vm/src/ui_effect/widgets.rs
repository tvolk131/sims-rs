//! Borrowed native widget searches (47649e/47692b/4769ca) and highlight geometry.
use serde::{Deserialize, Serialize};
pub mod button;
pub mod layout;
pub mod messages;
pub mod platform;
pub mod surface_geometry;
pub mod surface_resource;
pub mod timers;
pub mod transition;
pub mod visibility;
pub mod window;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    Window,
    Button,
    Relationship,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bitmap {
    pub handle: u32,
    pub resource: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Node {
    pub kind: Kind,
    pub bitmap: Option<Bitmap>,
    pub children: Vec<usize>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Query {
    Control(i32),
    Portrait(u32),
    Relationship,
}

/// The root itself is excluded. Each child is tested before its descendants;
/// the first match wins. cWinRelationship also derives from cTSWinBtn.
pub fn find(nodes: &[Node], root: Option<usize>, query: Query) -> Result<Option<usize>, String> {
    fn search(
        nodes: &[Node],
        root: usize,
        query: Query,
        path: &mut Vec<usize>,
        remaining: &mut usize,
    ) -> Result<Option<usize>, String> {
        if path.contains(&root) || path.len() >= 128 || *remaining == 0 {
            return Err("invalid UI widget graph".into());
        }
        *remaining -= 1;
        path.push(root);
        let root = nodes.get(root).ok_or("UI widget root absent")?;
        for &id in &root.children {
            let node = nodes.get(id).ok_or("UI widget child absent")?;
            let matched = match query {
                Query::Control(resource) => {
                    node.kind != Kind::Window && node.bitmap.is_some_and(|b| b.resource == resource)
                }
                Query::Portrait(handle) => {
                    node.kind != Kind::Window && node.bitmap.map_or(0, |b| b.handle) == handle
                }
                Query::Relationship => node.kind == Kind::Relationship,
            };
            if matched {
                path.pop();
                return Ok(Some(id));
            }
            if let Some(found) = search(nodes, id, query, path, remaining)? {
                path.pop();
                return Ok(Some(found));
            }
        }
        path.pop();
        Ok(None)
    }
    root.map_or(Ok(None), |root| {
        search(nodes, root, query, &mut vec![], &mut 65536)
    })
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Geometry {
    pub position: [i32; 2],
    pub rectangle: [i32; 4],
    pub flags: u32,
}

/// 5de13f/4cb0bb. None requests Hide. Some requests Move, Show, Raise,
/// palette entry0, then5ddf61 color assignment/invalidation, in that order.
pub fn highlight_rectangle(
    target: Geometry,
    highlight: [i32; 4],
    enabled: bool,
) -> Option<[i32; 4]> {
    if !enabled || target.flags & 1 == 0 {
        return None;
    }
    let [left, top, right, bottom] = highlight;
    let dx = target.rectangle[2]
        .wrapping_sub(right)
        .wrapping_sub(target.rectangle[0])
        .wrapping_add(left)
        / 2;
    let x = target.position[0].wrapping_add(dx);
    let y = target.position[1]
        .wrapping_add(top.wrapping_sub(bottom))
        .wrapping_sub(3);
    Some([
        x,
        y,
        x.wrapping_add(right.wrapping_sub(left)),
        y.wrapping_add(bottom.wrapping_sub(top)),
    ])
}
