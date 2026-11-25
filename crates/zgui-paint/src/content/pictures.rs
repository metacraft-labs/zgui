//! The pictures a style sheet names with `background-image` and `mask-image`, read once per URL.
//!
//! A picture is read the first time a frame draws it, through the rule in
//! [`zgui_css::resources`]: a `data:` URI, or a file inside a root a sheet was granted. The read is
//! synchronous and on the frame's thread, which is acceptable for what style sheets name — icons
//! of a few hundred bytes, each read once for the life of the process — and keeps the frame
//! deterministic: the first frame that draws a picture draws all of it.
//!
//! Only SVG documents are pictures here. A raster file named by a style sheet is reported once and
//! draws nothing.
//!
//! Every URL is read once, and the same allocation is handed to every box that draws it: a path
//! rasteriser keeps its encoding of a shape under the identity of the shape's allocation, so a new
//! allocation per frame would encode every icon again on every frame.

use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;
use zgui_svg::{Ink, Paint, Shape};

/// One picture, read from its URL.
#[derive(Debug)]
pub struct Picture {
    /// The outlines, in the picture's own space, with their own paints.
    pub shapes: Arc<[Shape]>,
    /// The same outlines, each painted with the inherited colour at its own paint's opacity.
    ///
    /// What a mask draws: the shapes are its coverage, and the colour is the box's background.
    pub silhouette: Arc<[Shape]>,
    /// The picture's own size, in CSS pixels, which is also the space its outlines are in.
    pub size: (f32, f32),
}

/// What each URL read as: a picture, or nothing when it could not be read.
type Read = FxHashMap<Arc<str>, Option<Arc<Picture>>>;

/// Every picture read so far, by URL, and the URLs that could not be read.
static PICTURES: Mutex<Option<Read>> = Mutex::new(None);

/// The picture `url` names, or nothing when it cannot be read as one.
///
/// A URL that cannot be read is reported once and not tried again.
pub fn picture(url: &str) -> Option<Arc<Picture>> {
    let mut held = PICTURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let held = held.get_or_insert_with(FxHashMap::default);
    if let Some(known) = held.get(url) {
        return known.clone();
    }
    let read = read(url)
        .map_err(|reason| {
            tracing::warn!(target: "zgui::css", url = %short(url), "a picture a style sheet names is not drawn: {reason}");
        })
        .ok()
        .map(Arc::new);
    held.insert(Arc::from(url), read.clone());
    read
}

/// At most the first hundred characters of `url`, which is enough to name a `data:` URI.
fn short(url: &str) -> &str {
    url.char_indices()
        .nth(100)
        .map_or(url, |(index, _)| &url[..index])
}

/// Reads and parses one picture.
fn read(url: &str) -> Result<Picture, String> {
    let bytes = zgui_css::resources::fetch(url).map_err(|error| error.to_string())?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "it is not an SVG document; only SVG pictures are drawn".to_owned())?;
    let document = zgui_svg::parse(text).map_err(|error| error.to_string())?;
    let [_, _, width, height] = document.view_box();
    let shapes = document.shapes_shared();
    let silhouette = shapes.iter().map(silhouette).collect::<Vec<_>>();
    Ok(Picture {
        shapes,
        silhouette: Arc::from(silhouette),
        size: (width, height),
    })
}

/// `shape` with each of its paints replaced by the inherited colour at that paint's opacity.
fn silhouette(shape: &Shape) -> Shape {
    let mut shape = shape.clone();
    if let Some(fill) = &mut shape.fill {
        fill.paint = coverage(&fill.paint);
    }
    if let Some(stroke) = &mut shape.stroke {
        stroke.paint = coverage(&stroke.paint);
    }
    shape
}

/// The inherited colour at the opacity of `paint`.
///
/// A gradient's opacity varies across it; it is read as opaque.
fn coverage(paint: &Paint) -> Paint {
    let alpha = match paint {
        Paint::Solid(Ink::Solid(color)) => color.alpha(),
        Paint::Solid(Ink::Inherited { alpha }) => *alpha,
        Paint::Gradient(_) => 1.0,
    };
    Paint::Solid(Ink::Inherited { alpha })
}

#[cfg(test)]
mod tests {
    use zgui_svg::{Ink, Paint};

    use super::picture;

    #[test]
    fn a_data_uri_picture_is_read_once_and_its_silhouette_keeps_each_opacity() {
        let url = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' \
                   height='6'%3E%3Crect width='6' height='6' fill='%23ff0000'/%3E%3Crect x='6' \
                   width='6' height='6' fill='blue' fill-opacity='0.5'/%3E%3C/svg%3E";
        let first = picture(url).expect("the document reads");
        assert_eq!(first.size, (12.0, 6.0));
        assert_eq!(first.shapes.len(), 2);
        let alphas: Vec<f32> = first
            .silhouette
            .iter()
            .map(|shape| match &shape.fill.as_ref().unwrap().paint {
                Paint::Solid(Ink::Inherited { alpha }) => *alpha,
                other => panic!("a silhouette paints the inherited colour: {other:?}"),
            })
            .collect();
        assert_eq!(alphas, [1.0, 0.5]);
        let second = picture(url).unwrap();
        assert!(
            std::sync::Arc::ptr_eq(&first.shapes, &second.shapes),
            "every draw of one URL shares one allocation"
        );
    }

    #[test]
    fn a_refused_url_or_a_raster_file_is_no_picture() {
        assert!(picture("zgui:///icons/missing.svg").is_none());
        assert!(picture("data:image/png;base64,iVBORw0KGgo=").is_none());
    }
}
