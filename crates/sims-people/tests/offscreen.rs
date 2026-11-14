//! Offscreen renders of a made-up body: no game data, no window.

use sims_formats::avatar::{Bone, Mesh, MeshBinding, Skeleton, Vertex, Wiggle};
use sims_people::math::{Quat, Transform, Vec3};
use sims_people::raster::{Camera, Canvas};
use sims_people::{Body, Part, Texture, rig};

/// A one-bone skeleton with a square of two triangles, one unit wide,
/// standing on the bone's origin and facing -Z, textured solid red.
fn body() -> Body {
    let skeleton = Skeleton {
        name: "test".into(),
        bones: vec![Bone {
            name: "ROOT".into(),
            parent: None,
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            properties: Vec::new(),
            flags: [0; 3],
            wiggle: Wiggle::default(),
        }],
    };
    let vertex = |x: f32, y: f32| Vertex {
        position: [x, y, 0.0],
        normal: [0.0, 0.0, -1.0],
        uv: [0.5, 0.5],
        bone: 0,
    };
    let mesh = Mesh {
        name: "square".into(),
        texture: "red".into(),
        bones: vec!["ROOT".into()],
        bindings: vec![MeshBinding {
            bone: 0,
            first_vertex: 0,
            vertex_count: 4,
            first_blend: 0,
            blend_count: 0,
        }],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        vertices: vec![
            vertex(-0.5, 0.0),
            vertex(0.5, 0.0),
            vertex(0.5, 1.0),
            vertex(-0.5, 1.0),
        ],
        blends: Vec::new(),
    };
    Body {
        skeleton,
        parts: vec![Part {
            appearance: "test".into(),
            bone: 0,
            bones: vec![0],
            mesh,
            texture_name: Some("red".into()),
            texture: Texture::solid([255, 0, 0, 255]),
            censor: 0,
        }],
    }
}

fn front() -> Camera {
    Camera {
        yaw: [0.0, 1.0],
        pitch: [0.0, 1.0],
        scale: 20.0,
        center: Vec3::new(0.0, 0.5, 0.0),
    }
}

#[test]
fn a_square_draws_where_the_camera_looks() {
    let body = body();
    let world = rig::Pose::rest(&body.skeleton).world(&body.skeleton, Transform::IDENTITY);
    let mut canvas = Canvas::new(40, 40, [0, 0, 0, 0]);
    canvas.draw_body(&front(), &body, &world);
    // A square of 20 by 20 pixels in the middle.
    assert_eq!(canvas.covered(), 400);
    let at = |x: u32, y: u32| {
        let i = ((y * 40 + x) * 4) as usize;
        canvas.rgba[i..i + 4].to_vec()
    };
    assert_eq!(at(0, 0), [0, 0, 0, 0]);
    assert_eq!(at(20, 20)[3], 255);
    assert!(at(20, 20)[0] > 100 && at(20, 20)[1] == 0);
}

#[test]
fn turning_the_bone_turns_the_mesh() {
    let body = body();
    // A quarter turn about Y shows the square edge on: nothing is drawn.
    let root = Transform {
        rotation: Quat::about_y(1.0, 0.0),
        translation: Vec3::ZERO,
    };
    let world = rig::Pose::rest(&body.skeleton).world(&body.skeleton, root);
    let mut canvas = Canvas::new(40, 40, [0, 0, 0, 0]);
    canvas.draw_body(&front(), &body, &world);
    assert_eq!(canvas.covered(), 0);
    // Moving the root moves the square across the image.
    let root = Transform {
        rotation: Quat::IDENTITY,
        translation: Vec3::new(0.5, 0.0, 0.0),
    };
    let world = rig::Pose::rest(&body.skeleton).world(&body.skeleton, root);
    let mut moved = Canvas::new(40, 40, [0, 0, 0, 0]);
    moved.draw_body(&front(), &body, &world);
    assert_eq!(moved.covered(), 400);
    assert_eq!(moved.rgba[((20 * 40 + 39) * 4) as usize + 3], 255);
    assert_eq!(moved.rgba[((20 * 40 + 5) * 4) as usize + 3], 0);
}
