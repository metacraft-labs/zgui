//! `mask-image`, lowered into the pictures that cut a box out.
//!
//! The mask an application writes is nearly always an icon: a box filled with a colour, cut to the
//! shape of a picture with `mask-image: url(icon.svg)`. The box's background is then the colour of
//! the icon, and its coverage is the picture's. That is what is drawn: each mask picture's shapes
//! are painted in the box's background colour, each at the opacity its own paint has, which is
//! what an alpha mask over a flat colour shows.
//!
//! Only `url()` layers are pictures. A mask made of gradients alone is not drawn as a mask, and the
//! box paints as if it had none.

use zgui_css::parity::Support;
use zgui_css::{ComputedStyle, register_properties};

use crate::lower::background::{PictureLayer, pictures_of};

register_properties! {
    mask_image      => Support::Implemented("zgui-paint::lower::mask"),
    mask_position_x => Support::Implemented("zgui-paint::lower::mask"),
    mask_position_y => Support::Implemented("zgui-paint::lower::mask"),
    mask_size       => Support::Implemented("zgui-paint::lower::mask"),
    mask_repeat     => Support::Implemented("zgui-paint::lower::mask"),
}

/// The pictures that cut a box out.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskStyle {
    /// The picture layers, in the order they were written. Their union is what stays visible.
    pub layers: Vec<PictureLayer>,
}

/// Lowers a style's mask, or nothing when it names no picture.
pub fn of(style: &ComputedStyle) -> Option<Box<MaskStyle>> {
    let svg = style.get_svg();
    let layers = pictures_of(
        &svg.mask_image.0,
        &svg.mask_position_x.0,
        &svg.mask_position_y.0,
        &svg.mask_size.0,
        &svg.mask_repeat.0,
    );
    (!layers.is_empty()).then(|| Box::new(MaskStyle { layers }))
}
