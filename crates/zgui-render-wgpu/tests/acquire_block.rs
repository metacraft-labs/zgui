//! A frame presented into a texture reports no acquisition wait.
//!
//! The runtime holds the next frame back by the wait the last frame reported. A texture never
//! makes a frame wait, so a non-zero report holds frames for no reason. On the headless
//! platform's virtual clock, a held frame runs only when the clock moves.

mod support;

use std::time::Duration;

use zgui_bits::DamageSet;
use zgui_render::Renderer;
use zgui_scene::{Quad, Scene};

use support::{SIDE, opaque, plain_renderer, present, rect};

#[test]
fn a_frame_presented_into_a_texture_reports_no_wait() {
    let Some(mut renderer) = plain_renderer() else {
        return;
    };
    for shade in 0..8u8 {
        let mut scene = Scene::new();
        scene.begin_frame(zgui_geom::Size::new(SIDE, SIDE));
        let paint = scene
            .paints
            .add(zgui_scene::Paint::Solid(opaque(shade * 30, 0, 0)));
        scene.push_quad(Quad::filled(
            rect(0.0, 0.0, SIDE as f32, SIDE as f32),
            paint,
        ));
        scene.finish(&DamageSet::full());
        present(&mut renderer, &scene);
        assert_eq!(
            renderer.acquire_block(),
            Duration::ZERO,
            "frame {shade} waited for a texture"
        );
    }
}
