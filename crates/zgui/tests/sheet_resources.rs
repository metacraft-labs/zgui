//! A style sheet's `url()` resources, loaded by the application the way a browser loads them.
//!
//! The sheet sits in a directory of its own and names a face and an icon above it, with `../`, as
//! a built application's sheets do. The application is the real one built by [`zgui::app`]: the
//! loader that registers the face is the one every application gets, and the frame is emitted by
//! the real paint stage. Only the renderer is a capturing stand-in, because what is asserted is the
//! display list the frame produced, which is what that renderer keeps.

use std::path::PathBuf;
use std::sync::Arc;

use zgui::app::Fonts;
use zgui::platform::{Surface, SurfaceEvent};
use zgui::prelude::*;
use zgui::render::{RenderTarget, Renderer};
use zgui::resources::ResourceBase;
use zgui::runtime::{AppError, Runtime};
use zgui::view::Ident;
use zgui_platform_headless::Harness;
use zgui_text_parley::LineRequest;

/// A ten unit square icon, drawn in black.
const ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="black"/></svg>"#;

/// The sheet, read from `<root>/styles`.
const SHEET: &str = r#"
    @font-face { font-family: "Sheet Face"; src: url("../fonts/missing.woff2") format("woff2"), url("../fonts/face.ttf") format("truetype") }
    root { display: block; width: 100px; height: 100px }
    .icon {
        display: block;
        width: 40px;
        height: 40px;
        background-color: #ff0000;
        mask-image: url("../icons/square.svg");
        mask-repeat: no-repeat;
        mask-position: center;
        mask-size: 10px 10px;
    }
    .missing {
        display: block;
        width: 40px;
        height: 40px;
        background-color: #00ff00;
        -webkit-mask-image: url("../icons/no-such-icon.svg");
    }
"#;

/// A directory holding `styles/`, `fonts/face.ttf` and `icons/square.svg`.
fn project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("zgui-sheet-{name}-{}", std::process::id()));
    for dir in ["styles", "fonts", "icons"] {
        std::fs::create_dir_all(root.join(dir)).expect("the project directories can be made");
    }
    std::fs::copy(
        format!(
            "{}/../zgui-text-parley/tests/fonts/NotoSans-Regular.ttf",
            env!("CARGO_MANIFEST_DIR")
        ),
        root.join("fonts/face.ttf"),
    )
    .expect("the face is copied");
    std::fs::write(root.join("icons/square.svg"), ICON).expect("the icon is written");
    root
}

/// The application, built over `fonts` with the sheet read from `<root>/styles`, after its first
/// frames.
fn mounted(fonts: Fonts, root: &std::path::Path, sheet_root: &std::path::Path) -> Harness<Runtime> {
    let base = ResourceBase::directory(root.join("styles"))
        .and_then(|base| base.with_root(sheet_root))
        .expect("the directories exist");
    let handler = app()
        .with_title("sheet resources")
        .with_size(100.0, 100.0)
        .with_fonts(fonts)
        .with_stylesheet_at(SHEET, base)
        .with_renderer(Box::new(
            |_surface: &Arc<dyn Surface>, target: RenderTarget| {
                let mut renderer = zgui_testkit_scene::CaptureRenderer::new();
                Renderer::configure(&mut renderer, target);
                Ok::<_, AppError>(Box::new(renderer) as Box<dyn Renderer>)
            },
        ))
        .into_handler(|| {
            view! {
                box(class = "root") {
                    box(class = "icon") {}
                    box(class = "missing") {}
                }
            }
        })
        .expect("the reactive runtime installs")
        .into_runtime();
    let mut harness = Harness::new(handler);
    harness.deliver_to_first(SurfaceEvent::Resized(zgui::geom::Size::new(
        zgui::geom::DevicePx(100.0),
        zgui::geom::DevicePx(100.0),
    )));
    harness.settle(32);
    harness
}

/// Whether `fonts` shapes text in the family the sheet declares.
fn shapes_sheet_face(fonts: &Fonts) -> bool {
    let families = [Ident::new("Sheet Face")];
    !fonts
        .shaper()
        .shape_line(
            "hi",
            &LineRequest {
                families: &families,
                weight: 400,
                italic: false,
                size_device_px: 16.0,
                letter_spacing: 0.0,
                ligatures: true,
            },
        )
        .is_empty()
}

/// Every rectangle something was drawn into, as `[x, y, width, height]`, with its colour when the
/// primitive carries one.
fn drawn(harness: &mut Harness<Runtime>) -> Vec<([f32; 4], Option<[f32; 4]>)> {
    let window = &mut harness.app_mut().windows_mut()[0];
    let scene = window.scene();
    let mut out = Vec::new();
    for quad in &scene.primitives.quads {
        out.push((quad.bounds, None));
    }
    for sprite in &scene.primitives.mono_sprites {
        out.push((sprite.bounds, Some(sprite.color)));
    }
    for vector in &scene.primitives.vectors {
        let ink = vector.local_ink;
        out.push((
            [
                ink.origin.x.0,
                ink.origin.y.0,
                ink.size.width.0,
                ink.size.height.0,
            ],
            None,
        ));
    }
    out
}

#[test]
fn a_sheet_face_named_above_the_sheet_reaches_the_applications_fonts() {
    let root = project("face");
    let fonts = Fonts::shipped_only();
    assert!(
        !shapes_sheet_face(&fonts),
        "no face is registered before the sheet loads"
    );
    let _harness = mounted(fonts.clone(), &root, &root);
    assert!(
        shapes_sheet_face(&fonts),
        "the sheet's @font-face rule registered its TrueType source under the rule's family"
    );
}

#[test]
fn a_face_above_the_root_the_sheet_may_read_is_not_loaded() {
    let root = project("confined");
    let fonts = Fonts::shipped_only();
    let _harness = mounted(fonts.clone(), &root, &root.join("styles"));
    assert!(
        !shapes_sheet_face(&fonts),
        "`../fonts/face.ttf` from a sheet whose root is its own directory stays inside it"
    );
}

#[test]
fn a_masked_box_draws_its_background_colour_through_the_mask_picture() {
    let root = project("mask");
    let mut harness = mounted(Fonts::shipped_only(), &root, &root);
    let drawn = drawn(&mut harness);
    // The icon box is 40 by 40 at the origin; its 10 by 10 mask is centred, so it covers 15..25.
    let inside = |rect: [f32; 4]| {
        rect[0] >= 14.0 && rect[1] >= 14.0 && rect[0] + rect[2] <= 26.0 && rect[1] + rect[3] <= 26.0
    };
    assert!(
        drawn.iter().any(|(rect, _)| inside(*rect)),
        "the mask's square is drawn in the middle of the box: {drawn:?}"
    );
    assert!(
        !drawn
            .iter()
            .any(|(rect, _)| rect[2] >= 39.0 && rect[3] >= 39.0 && rect[1] < 40.0),
        "the box's background is not drawn as a whole 40 by 40 square: {drawn:?}"
    );
    // A mask whose picture cannot be read hides the box: the green box below draws nothing.
    assert!(
        !drawn
            .iter()
            .any(|(rect, _)| rect[1] >= 39.0 && rect[1] < 80.0),
        "a box whose mask does not load is transparent: {drawn:?}"
    );
}
