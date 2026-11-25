//! Backgrounds, list markers and anything else painted from an image or a ramp.

/// A position along a conic gradient's sweep: an angle, or a fraction of the whole turn.
pub use style::values::computed::AngleOrPercentage as AngleOrPercentageValue;
/// The computed value of `background-repeat`.
pub use style::values::computed::BackgroundRepeat as BackgroundRepeatValue;
/// The computed value of `background-size`.
pub use style::values::computed::BackgroundSize as BackgroundSizeValue;
/// A gradient, with its stops resolved and its interpolation space named.
///
/// A gradient's ramp is not sRGB unless it says so: the interpolation space travels with the value
/// so that a ramp asked for in Oklab is drawn in Oklab, rather than banded through sRGB on the way.
pub use style::values::computed::Gradient as GradientValue;
/// The computed value of `background-image` and every other image-valued property.
///
/// An image is a URL, a gradient, a cross-fade or an `image-set()` — the last of which has already
/// picked its entry, because the choice depends on the device pixel ratio and that is a cascade
/// input.
pub use style::values::computed::Image as ImageValue;
/// A radial gradient's ending shape: a circle or an ellipse, with its size.
pub use style::values::computed::image::EndingShape as EndingShapeValue;
/// Which way a linear gradient's line points: an angle, an edge keyword, or a corner.
pub use style::values::computed::image::LineDirection as LineDirectionValue;
/// The computed value of `background-position-x`.
pub use style::values::computed::position::HorizontalPosition as HorizontalPositionValue;
/// The computed value of `background-position-y`.
pub use style::values::computed::position::VerticalPosition as VerticalPositionValue;
/// A radial gradient's circle size: a radius, or an extent keyword.
pub use style::values::generics::image::Circle as CircleValue;
/// A radial gradient's ellipse size: two radii, or an extent keyword.
pub use style::values::generics::image::Ellipse as EllipseValue;
/// The state flags a gradient carries, of which `REPEATING` is the one that changes what is drawn.
pub use style::values::generics::image::GradientFlags;
/// One entry of a gradient's stop list: a stop, a positioned stop, or an interpolation hint.
pub use style::values::generics::image::GradientItem as GradientItemValue;
/// A radial gradient's extent keyword: how far the ending shape reaches.
pub use style::values::generics::image::ShapeExtent;
/// The horizontal half of a corner keyword.
pub use style::values::specified::position::HorizontalPositionKeyword;
/// The vertical half of a corner keyword.
pub use style::values::specified::position::VerticalPositionKeyword;

/// How large one background or mask layer is drawn, read off a `background-size` or `mask-size`
/// entry.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerSize {
    /// A width and a height, either of which may be `auto`. A percentage is of the positioning
    /// area.
    Explicit(
        Option<crate::values::length::LengthPercentage>,
        Option<crate::values::length::LengthPercentage>,
    ),
    /// As large as fits inside the positioning area, keeping the image's aspect ratio.
    Contain,
    /// As small as covers the positioning area, keeping the image's aspect ratio.
    Cover,
}

impl LayerSize {
    /// The size one `background-size` or `mask-size` entry states.
    pub fn of(value: &BackgroundSizeValue) -> Self {
        use style::values::generics::background::GenericBackgroundSize as Size;
        use style::values::generics::length::GenericLengthPercentageOrAuto as OrAuto;
        let side = |value: &OrAuto<
            crate::values::length::NonNegative<crate::values::length::LengthPercentage>,
        >| match value {
            OrAuto::LengthPercentage(length) => Some(length.0.clone()),
            OrAuto::Auto => None,
        };
        match value {
            Size::ExplicitSize { width, height } => Self::Explicit(side(width), side(height)),
            Size::Contain => Self::Contain,
            Size::Cover => Self::Cover,
        }
    }
}

/// Whether one `background-repeat` or `mask-repeat` entry repeats the image along each axis.
///
/// `space` and `round` are read as `repeat`: they tile the image too, with the gaps and the scale
/// they choose left out.
pub fn repeats(value: &BackgroundRepeatValue) -> (bool, bool) {
    use style::values::specified::background::BackgroundRepeatKeyword as Keyword;
    let tiles = |keyword: Keyword| keyword != Keyword::NoRepeat;
    (tiles(value.0), tiles(value.1))
}

/// The resolved URL an image names, when it is a valid `url()`.
pub fn image_url(value: &ImageValue) -> Option<&str> {
    match value {
        ImageValue::Url(url) => Some(url.url()?.as_str()),
        _ => None,
    }
}
