//! The pictures of `background-image` and `mask-image`, placed on a box and drawn as vector
//! content.
//!
//! A picture layer is placed in CSS terms: `background-size` decides how large one copy is,
//! `background-position` where the first copy sits in the positioning area, and
//! `background-repeat` whether copies tile the area. The positioning area is the padding box for a
//! background and the border box for a mask. Each copy is fitted with the picture's aspect ratio
//! kept and centred, as an SVG document with no `preserveAspectRatio` of its own is.
//!
//! Copies are not cut to the box: a picture larger than its box, or the copies at the edge of a
//! tiled area, overflow it.

use std::sync::Arc;

use zgui_color::Color;
use zgui_css::values::image::LayerSize;
use zgui_css::values::length::{LengthPercentage, evaluate_at};
use zgui_geom::{CssPx, Device, DevicePx, Point, Rect, Size};
use zgui_scene::{Scene, VectorId};

use crate::content::VectorMaskSource;
use crate::content::pictures::picture;
use crate::emit::box_::BoxPlacement;
use crate::emit::vector::{ShapePaint, VectorPlacement, fit};
use crate::lower::PaintStyle;
use crate::lower::background::PictureLayer;

/// The most copies one tiled layer draws, so a tiny picture tiled over a huge box stays bounded.
const MAX_COPIES: usize = 4096;

/// Emits the picture layers of a box's background, the first written on top.
///
/// `id` names the box; each copy's vector identity is derived from it.
pub fn background(
    scene: &mut Scene,
    style: &PaintStyle,
    placement: BoxPlacement,
    masks: &dyn VectorMaskSource,
    id: u32,
) -> usize {
    let area = placement.border_box.inset(placement.border);
    let mut pushed = 0;
    for (index, layer) in style.background.pictures.iter().enumerate().rev() {
        let Some(picture) = picture(&layer.url) else {
            continue;
        };
        // A picture's own `currentColor` is black, as it is for a picture a browser loads as an
        // image: the document it is in is not its context.
        let paint = ShapePaint {
            fill: Color::BLACK,
            stroke: None,
            stroke_width: placement.scale,
        };
        pushed += draw(
            scene,
            &picture.shapes,
            picture.size,
            layer,
            area,
            paint,
            placement,
            masks,
            salted(id, 0x5EED_0001, index),
        );
    }
    pushed
}

/// Emits a masked box: each mask picture's coverage, painted in the box's background colour.
///
/// A mask whose pictures cannot be read hides the box, as a mask image that fails to load is
/// transparent.
pub fn masked(
    scene: &mut Scene,
    style: &PaintStyle,
    placement: BoxPlacement,
    masks: &dyn VectorMaskSource,
    id: u32,
) -> usize {
    let Some(mask) = &style.mask else {
        return 0;
    };
    let color = style.background.color;
    if color.alpha() == 0.0 {
        return 0;
    }
    let paint = ShapePaint {
        fill: color,
        stroke: None,
        stroke_width: placement.scale,
    };
    let mut pushed = 0;
    for (index, layer) in mask.layers.iter().enumerate().rev() {
        let Some(picture) = picture(&layer.url) else {
            continue;
        };
        pushed += draw(
            scene,
            &picture.silhouette,
            picture.size,
            layer,
            placement.border_box,
            paint,
            placement,
            masks,
            salted(id, 0x5EED_0002, index),
        );
    }
    pushed
}

/// A vector identity for one layer of one box, apart from the box's own drawing.
fn salted(id: u32, salt: u32, index: usize) -> u32 {
    id.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_add((index as u32).wrapping_mul(0x85EB_CA6B))
}

/// Draws every copy of one layer over `area`, and returns how many primitives were pushed.
#[allow(clippy::too_many_arguments)]
fn draw(
    scene: &mut Scene,
    shapes: &Arc<[zgui_svg::Shape]>,
    intrinsic: (f32, f32),
    layer: &PictureLayer,
    area: Rect<DevicePx, Device>,
    paint: ShapePaint,
    placement: BoxPlacement,
    masks: &dyn VectorMaskSource,
    id: u32,
) -> usize {
    let copies = copies(layer, intrinsic, area, placement.scale);
    let view_box = Some([0.0, 0.0, intrinsic.0, intrinsic.1]);
    let mut pushed = 0;
    for (index, copy) in copies.into_iter().enumerate() {
        let drawing = crate::content::Drawing::fitted_shared(
            Arc::clone(shapes),
            fit::onto(copy, view_box, placement.scale),
        );
        pushed += crate::emit::vector::draw_drawing(
            scene,
            VectorId(id.wrapping_add((index as u32).wrapping_mul(0x2545_F491))),
            &drawing,
            paint,
            masks,
            VectorPlacement {
                clip: placement.clip,
                transform: placement.transform,
                scale: placement.scale,
            },
        );
    }
    pushed
}

/// The rectangles the copies of one layer are drawn in, in device pixels.
pub fn copies(
    layer: &PictureLayer,
    intrinsic: (f32, f32),
    area: Rect<DevicePx, Device>,
    scale: f32,
) -> Vec<Rect<DevicePx, Device>> {
    let (width, height) = size(&layer.size, intrinsic, area, scale);
    if !(width > 0.0 && height > 0.0) {
        return Vec::new();
    }
    let x = area.origin.x.0 + offset(&layer.x, area.size.width.0 - width, scale);
    let y = area.origin.y.0 + offset(&layer.y, area.size.height.0 - height, scale);
    let starts = |start: f32, step: f32, from: f32, extent: f32, tiles: bool| -> Vec<f32> {
        if !tiles {
            return vec![start];
        }
        let first = start - ((start - from) / step).ceil() * step;
        let mut at = Vec::new();
        let mut position = first;
        while position < from + extent && at.len() < MAX_COPIES {
            at.push(position);
            position += step;
        }
        at
    };
    let columns = starts(x, width, area.origin.x.0, area.size.width.0, layer.repeat.0);
    let rows = starts(
        y,
        height,
        area.origin.y.0,
        area.size.height.0,
        layer.repeat.1,
    );
    let mut out = Vec::with_capacity(columns.len() * rows.len());
    'rows: for row in &rows {
        for column in &columns {
            if out.len() >= MAX_COPIES {
                break 'rows;
            }
            out.push(Rect::new(
                Point::new(DevicePx(*column), DevicePx(*row)),
                Size::new(DevicePx(width), DevicePx(height)),
            ));
        }
    }
    out
}

/// `value` resolved against `basis` device pixels, in device pixels.
fn offset(value: &LengthPercentage, basis: f32, scale: f32) -> f32 {
    evaluate_at(value, CssPx(basis / scale)).0 * scale
}

/// How large one copy is, in device pixels.
fn size(
    size: &LayerSize,
    intrinsic: (f32, f32),
    area: Rect<DevicePx, Device>,
    scale: f32,
) -> (f32, f32) {
    let (area_width, area_height) = (area.size.width.0, area.size.height.0);
    let (own_width, own_height) = (intrinsic.0 * scale, intrinsic.1 * scale);
    let ratio = if own_height > 0.0 {
        own_width / own_height
    } else {
        1.0
    };
    match size {
        LayerSize::Contain | LayerSize::Cover => {
            if own_width <= 0.0 || own_height <= 0.0 {
                return (area_width, area_height);
            }
            let by_width = area_width / own_width;
            let by_height = area_height / own_height;
            let factor = if matches!(size, LayerSize::Contain) {
                by_width.min(by_height)
            } else {
                by_width.max(by_height)
            };
            (own_width * factor, own_height * factor)
        }
        LayerSize::Explicit(width, height) => {
            let width = width.as_ref().map(|width| offset(width, area_width, scale));
            let height = height
                .as_ref()
                .map(|height| offset(height, area_height, scale));
            match (width, height) {
                (Some(width), Some(height)) => (width, height),
                (Some(width), None) => (width, width / ratio),
                (None, Some(height)) => (height * ratio, height),
                (None, None) => (own_width, own_height),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use zgui_css::values::image::LayerSize;
    use zgui_css::values::length::percent;
    use zgui_geom::{Device, DevicePx, Point, Rect, Size};

    use super::copies;
    use crate::lower::background::PictureLayer;

    fn area(x: f32, y: f32, width: f32, height: f32) -> Rect<DevicePx, Device> {
        Rect::new(
            Point::new(DevicePx(x), DevicePx(y)),
            Size::new(DevicePx(width), DevicePx(height)),
        )
    }

    fn layer(size: LayerSize, x: f32, y: f32, repeat: (bool, bool)) -> PictureLayer {
        PictureLayer {
            url: "data:,".into(),
            x: percent(x),
            y: percent(y),
            size,
            repeat,
        }
    }

    fn rects(found: &[Rect<DevicePx, Device>]) -> Vec<[f32; 4]> {
        found
            .iter()
            .map(|r| [r.origin.x.0, r.origin.y.0, r.size.width.0, r.size.height.0])
            .collect()
    }

    #[test]
    fn a_centred_icon_at_its_own_size_sits_in_the_middle_of_the_box() {
        let found = copies(
            &layer(LayerSize::Explicit(None, None), 0.5, 0.5, (false, false)),
            (12.0, 12.0),
            area(10.0, 20.0, 32.0, 32.0),
            2.0,
        );
        assert_eq!(rects(&found), [[14.0, 24.0, 24.0, 24.0]]);
    }

    #[test]
    fn contain_and_cover_keep_the_aspect_ratio() {
        let box_ = area(0.0, 0.0, 40.0, 20.0);
        let contain = copies(
            &layer(LayerSize::Contain, 0.5, 0.5, (false, false)),
            (10.0, 10.0),
            box_,
            1.0,
        );
        assert_eq!(rects(&contain), [[10.0, 0.0, 20.0, 20.0]]);
        let cover = copies(
            &layer(LayerSize::Cover, 0.0, 0.0, (false, false)),
            (10.0, 10.0),
            box_,
            1.0,
        );
        assert_eq!(rects(&cover), [[0.0, 0.0, 40.0, 40.0]]);
    }

    #[test]
    fn an_explicit_width_scales_the_height_with_it_and_a_repeat_tiles_the_area() {
        let tiled = copies(
            &layer(
                LayerSize::Explicit(Some(percent(0.5)), None),
                0.0,
                0.0,
                (true, false),
            ),
            (10.0, 5.0),
            area(0.0, 0.0, 40.0, 30.0),
            1.0,
        );
        assert_eq!(
            rects(&tiled),
            [[0.0, 0.0, 20.0, 10.0], [20.0, 0.0, 20.0, 10.0]]
        );
    }
}
