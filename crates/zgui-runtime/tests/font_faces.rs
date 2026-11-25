//! The `@font-face` rules of a window's sheets reach the window's font loader, with their sources
//! resolved against the directory each sheet was read from.
//!
//! The loader is a recording closure: what is asserted is what the window hands it, and loading a
//! face is the application crate's, which has tests of its own.

mod support;

use std::cell::RefCell;
use std::rc::Rc;

use zgui_platform_headless::Harness;
use zgui_runtime::resources::{FontFace, FontSource, ResourceBase};
use zgui_runtime::{App, FontFaceLoader};
use zgui_view::{Anchor, BuildCx, IntoView, View};

#[test]
fn the_faces_of_an_opening_sheet_and_of_a_sheet_a_view_installs_reach_the_loader() {
    let root = std::env::temp_dir().join(format!("zgui-runtime-faces-{}", std::process::id()));
    std::fs::create_dir_all(root.join("styles")).unwrap();
    let base = ResourceBase::directory(root.join("styles"))
        .unwrap()
        .with_root(&root)
        .unwrap();
    let loaded: Rc<RefCell<Vec<FontFace>>> = Rc::default();
    let into = Rc::clone(&loaded);
    let styles = root.join("styles");
    let view_root = root.clone();
    let handler = App::new()
        .with_title("faces")
        .with_size(200.0, 100.0)
        .with_stylesheet_at(
            "@font-face { font-family: Early; src: url(../fonts/early.ttf) }",
            base.clone(),
        )
        .with_font_faces(FontFaceLoader::new(move |faces| {
            into.borrow_mut().extend_from_slice(faces);
        }))
        .with_renderer(Box::new(support::capture))
        .into_handler(move |cx: &mut BuildCx<'_>| -> Box<dyn Anchor> {
            zgui_view::install_stylesheet_at(
                "late",
                "@font-face { font-family: Late; src: url(late.ttf) format('truetype') }",
                &styles,
                &view_root,
            );
            Box::new(zgui_elements::column().into_view().build(cx))
        })
        .expect("the reactive runtime installs");
    let mut harness = Harness::new(handler);
    harness.settle(8);

    let loaded = loaded.borrow();
    let families: Vec<&str> = loaded.iter().map(|face| face.family.as_str()).collect();
    assert_eq!(families, ["Early", "Late"]);
    let root_url = ResourceBase::directory(&root).unwrap();
    assert_eq!(
        loaded[0].sources,
        [FontSource::Url {
            url: format!("{}fonts/early.ttf", root_url.url()),
            format: None,
        }],
        "a source resolves against the sheet's directory, above it, inside the root"
    );
    assert_eq!(
        loaded[1].sources,
        [FontSource::Url {
            url: format!("{}styles/late.ttf", root_url.url()),
            format: Some("truetype".to_owned()),
        }]
    );
}
