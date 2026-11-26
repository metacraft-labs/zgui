//! The natural size and ratio of a vector document, read off its outermost `<svg>` element.
//!
//! A `vector` element carrying a whole document sizes the way an inline `<svg>` does in a
//! browser: its document is its content, and that content has natural dimensions decided by the
//! root element's `width`, `height` and `viewBox` (SVG 2 §8.2, "Intrinsic sizing properties of the
//! viewport of SVG content"; CSS Images 3 §4, "Sizing Images and Objects in CSS"). An icon written
//! `<svg viewBox="0 0 18 17">` and styled `height: 1em; width: auto` therefore takes its width from
//! the ratio, as it does in Chromium, instead of collapsing to nothing.
//!
//! Only the root start tag is read. The rest of the document decides what is drawn, never how big
//! the box is, so a scan that stops at the first `>` is the whole question — and it is cheap enough
//! to ask again on every write of the document.

use zgui_geom::{CssPx, Size};

use crate::host::replaced::Intrinsic;

/// The CSS default object size (CSS Images 3 §4.3): what content with no natural size on an axis
/// takes on that axis when nothing else resolves it.
const DEFAULT_OBJECT_SIZE: (f32, f32) = (300.0, 150.0);

/// The natural sizing of the vector document `source`, or nothing when it is not an SVG document.
///
/// - The natural ratio is `width / height` when both attributes are absolute lengths and neither is
///   zero, and otherwise the `viewBox`'s width over its height.
/// - The natural width and height are the `width` and `height` attributes when they are absolute
///   lengths. A percentage, a font-relative length, `auto` or a missing attribute is no natural
///   size on that axis.
/// - An axis with no natural size takes it from the other axis through the ratio when there is
///   one, and otherwise from the default object size, 300 by 150. A document with a ratio and
///   neither length reports the ratio alone, which leaves the box's width to the layout around it.
///
/// The size is filled in here, rather than left to layout, because the [`Intrinsic`] a replaced
/// box is sized from carries a size for both axes or for neither.
pub fn natural_sizing(source: &str) -> Option<Intrinsic> {
    let root = root_start_tag(source)?;
    let mut width = None;
    let mut height = None;
    let mut view_box = None;
    for (name, value) in attributes(root) {
        match name {
            "width" => width = absolute_length(value),
            "height" => height = absolute_length(value),
            "viewBox" => view_box = zgui_vocab::prop::drawing::view_box(value),
            _ => {}
        }
    }
    let ratio = match (width, height) {
        (Some(width), Some(height)) if width > 0.0 && height > 0.0 => Some(width / height),
        _ => view_box.map(|[_, _, width, height]| width / height),
    };
    let size = match (width, height, ratio) {
        (Some(width), Some(height), _) => Some((width, height)),
        (Some(width), None, Some(ratio)) => Some((width, width / ratio)),
        (Some(width), None, None) => Some((width, DEFAULT_OBJECT_SIZE.1)),
        (None, Some(height), Some(ratio)) => Some((height * ratio, height)),
        (None, Some(height), None) => Some((DEFAULT_OBJECT_SIZE.0, height)),
        (None, None, Some(_)) => None,
        (None, None, None) => Some(DEFAULT_OBJECT_SIZE),
    };
    Some(Intrinsic {
        size: size.map(|(width, height)| Size::new(CssPx(width), CssPx(height))),
        ratio,
        baseline: None,
    })
}

/// The text between the root element's name and the `>` closing its start tag, if the root is an
/// `svg` element.
///
/// Skips what may precede the root in a document: a byte-order mark, whitespace, the XML
/// declaration, processing instructions, comments and a document type declaration.
fn root_start_tag(source: &str) -> Option<&str> {
    let mut rest = source.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("<?") {
            rest = &after[after.find("?>")? + 2..];
        } else if let Some(after) = rest.strip_prefix("<!--") {
            rest = &after[after.find("-->")? + 3..];
        } else if let Some(after) = rest.strip_prefix("<!") {
            rest = &after[declaration_end(after)?..];
        } else {
            break;
        }
    }
    let after = rest.strip_prefix('<')?;
    let name_end = after
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(after.len());
    let name = &after[..name_end];
    let local = name.rsplit(':').next().unwrap_or(name);
    if local != "svg" {
        return None;
    }
    let tag = &after[name_end..];
    Some(&tag[..tag_end(tag)?])
}

/// Where a `<!…>` declaration ends, as the offset just past its `>`, skipping an internal subset.
fn declaration_end(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (at, c) in text.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            '>' if depth == 0 => return Some(at + 1),
            _ => {}
        }
    }
    None
}

/// The offset of the `>` that closes a start tag, ignoring any inside quoted attribute values.
fn tag_end(tag: &str) -> Option<usize> {
    let mut quote = None;
    for (at, c) in tag.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), _) if c == open => quote = None,
            (None, '>') => return Some(at),
            _ => {}
        }
    }
    None
}

/// The `name="value"` pairs of a start tag's attribute text. Stops at the first malformed one.
fn attributes(mut text: &str) -> impl Iterator<Item = (&str, &str)> {
    std::iter::from_fn(move || {
        text = text.trim_start().trim_start_matches('/').trim_start();
        let name_end = text.find(|c: char| c.is_whitespace() || c == '=')?;
        let name = &text[..name_end];
        let after = text[name_end..]
            .trim_start()
            .strip_prefix('=')?
            .trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let body = &after[1..];
        let close = body.find(quote)?;
        text = &body[close + 1..];
        Some((name, &body[..close]))
    })
}

/// An absolute, non-negative length in CSS pixels, or nothing for anything else.
///
/// A bare number is in user units, which are CSS pixels for the outermost `<svg>`. The absolute
/// units convert at CSS's fixed ratios (CSS Values 4 §6.2). Percentages and font- or
/// viewport-relative lengths resolve against things a document does not know, so they are no
/// natural size.
fn absolute_length(text: &str) -> Option<f32> {
    const UNITS: &[(&str, f32)] = &[
        ("px", 1.0),
        ("in", 96.0),
        ("cm", 96.0 / 2.54),
        ("mm", 96.0 / 25.4),
        ("q", 96.0 / 101.6),
        ("pt", 96.0 / 72.0),
        ("pc", 16.0),
    ];
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let (number, scale) = UNITS
        .iter()
        .find_map(|(unit, scale)| lower.strip_suffix(unit).map(|number| (number, *scale)))
        .unwrap_or((lower.as_str(), 1.0));
    let value = number.trim_end().parse::<f32>().ok()? * scale;
    (value.is_finite() && value >= 0.0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::natural_sizing;

    /// The size and ratio of `source`, as plain numbers.
    fn sizing(source: &str) -> (Option<(f32, f32)>, Option<f32>) {
        let intrinsic = natural_sizing(source).expect("an SVG document");
        (
            intrinsic.size.map(|size| (size.width.0, size.height.0)),
            intrinsic.ratio,
        )
    }

    #[test]
    fn a_view_box_alone_is_a_ratio_and_no_size() {
        assert_eq!(
            sizing(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 17"><path d="M0 0"/></svg>"#
            ),
            (None, Some(18.0 / 17.0))
        );
    }

    #[test]
    fn absolute_width_and_height_are_the_natural_size_and_ratio() {
        assert_eq!(
            sizing(r#"<svg width="24" height="10px" viewBox="0 0 18 17"/>"#),
            (Some((24.0, 10.0)), Some(2.4))
        );
        assert_eq!(
            sizing(r#"<svg width='12pt' height="1in"></svg>"#),
            (Some((16.0, 96.0)), Some(16.0 / 96.0))
        );
    }

    #[test]
    fn one_length_takes_the_other_from_the_ratio_or_the_default_object_size() {
        // Chromium 147: `<svg width="36" viewBox="0 0 18 17">` is 36 x 34; `<svg width="24">` is
        // 24 x 150; `<svg height="24">` is 300 x 24.
        assert_eq!(
            sizing(r#"<svg width="36" viewBox="0 0 18 17">"#),
            (Some((36.0, 34.0)), Some(18.0 / 17.0))
        );
        assert_eq!(sizing(r#"<svg width="24">"#), (Some((24.0, 150.0)), None));
        assert_eq!(sizing(r#"<svg height="24">"#), (Some((300.0, 24.0)), None));
    }

    #[test]
    fn nothing_at_all_is_the_default_object_size() {
        // Chromium 147: a bare `<svg>` is 300 x 150.
        assert_eq!(sizing("<svg></svg>"), (Some((300.0, 150.0)), None));
    }

    #[test]
    fn relative_lengths_and_broken_view_boxes_are_no_natural_size() {
        assert_eq!(
            sizing(r#"<svg width="50%" height="2em" viewBox="0 0 18 17">"#),
            (None, Some(18.0 / 17.0))
        );
        assert_eq!(
            sizing(r#"<svg viewBox="0 0 0 17">"#),
            (Some((300.0, 150.0)), None)
        );
        assert_eq!(
            sizing(r#"<svg width="-4" viewBox="0 0 18">"#),
            (Some((300.0, 150.0)), None)
        );
    }

    #[test]
    fn what_precedes_the_root_is_skipped_and_a_prefixed_root_is_read() {
        let source = "\u{feff}<?xml version=\"1.0\"?>\n<!-- icon -->\n\
             <!DOCTYPE svg [ <!ENTITY a \"b\"> ]>\n\
             <svg:svg xmlns:svg=\"http://www.w3.org/2000/svg\" data-x=\"a>b\" width=\"8\" height=\"4\">";
        assert_eq!(sizing(source), (Some((8.0, 4.0)), Some(2.0)));
    }

    #[test]
    fn a_document_that_is_not_svg_has_no_natural_sizing() {
        assert!(natural_sizing("<html></html>").is_none());
        assert!(natural_sizing("not markup").is_none());
        assert!(
            natural_sizing("<svg width=\"4\"").is_none(),
            "an unterminated tag"
        );
    }
}
