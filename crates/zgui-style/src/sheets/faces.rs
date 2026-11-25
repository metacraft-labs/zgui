//! The `@font-face` rules a parsed sheet holds, as the faces a font collection loads.
//!
//! The style engine keeps an `@font-face` rule and does nothing with it: fetching a face is the
//! embedder's. This reads the rules out of a sheet once it is parsed, with every `src` URL already
//! resolved against the sheet's base, and the window hands them to whatever holds its faces.
//!
//! Rules inside `@media`, `@supports` and `@layer` blocks are read whatever their condition, and so
//! are the rules of imported sheets. A face is only loaded, never drawn with, until `font-family`
//! names it, so a face whose condition does not hold costs a load and changes nothing on screen.

use style::shared_lock::{SharedRwLock, SharedRwLockReadGuard};
use style::stylesheets::{CssRule, DocumentStyleSheet, StylesheetInDocument};
use zgui_css::resources::{FontFace, FontSource};

/// Every `@font-face` rule of `sheet` and of the sheets it imports, in the order written.
pub(crate) fn of(sheet: &DocumentStyleSheet, lock: &SharedRwLock) -> Vec<FontFace> {
    let guard = lock.read();
    let mut faces = Vec::new();
    walk(sheet.contents(&guard).rules(&guard), &guard, &mut faces);
    faces
}

/// The faces of `rules`, appended to `faces`.
fn walk(rules: &[CssRule], guard: &SharedRwLockReadGuard<'_>, faces: &mut Vec<FontFace>) {
    for rule in rules {
        match rule {
            CssRule::FontFace(rule) => faces.extend(face(&rule.read_with(guard).descriptors)),
            CssRule::Import(import) => {
                let import = import.read_with(guard);
                if let Some(sheet) = import.stylesheet.as_sheet() {
                    walk(sheet.contents(guard).rules(guard), guard, faces);
                }
            }
            CssRule::Media(media) => walk(&media.rules.read_with(guard).0, guard, faces),
            CssRule::Supports(supports) => walk(&supports.rules.read_with(guard).0, guard, faces),
            CssRule::LayerBlock(layer) => walk(&layer.rules.read_with(guard).0, guard, faces),
            _ => {}
        }
    }
}

/// One rule's face, or nothing when it names no family or no source.
fn face(descriptors: &style::font_face::Descriptors) -> Option<FontFace> {
    let family = descriptors.font_family.as_ref()?.name.to_string();
    let sources = descriptors
        .src
        .as_ref()?
        .0
        .iter()
        .filter_map(|source| match source {
            style::font_face::Source::Url(source) => {
                let url = source.url.url()?.as_str().to_owned();
                let format = source.format_hint.as_ref().map(format_name);
                Some(FontSource::Url { url, format })
            }
            style::font_face::Source::Local(name) => Some(FontSource::Local(name.name.to_string())),
        })
        .collect::<Vec<_>>();
    (!sources.is_empty()).then_some(FontFace { family, sources })
}

/// A `format()` hint as the lower-case name a style sheet writes.
fn format_name(hint: &style::font_face::FontFaceSourceFormat) -> String {
    use style::font_face::{FontFaceSourceFormat, FontFaceSourceFormatKeyword as Keyword};
    match hint {
        FontFaceSourceFormat::String(name) => name.to_ascii_lowercase(),
        FontFaceSourceFormat::Keyword(keyword) => match keyword {
            Keyword::Collection => "collection",
            Keyword::EmbeddedOpentype => "embedded-opentype",
            Keyword::Opentype => "opentype",
            Keyword::Svg => "svg",
            Keyword::Truetype => "truetype",
            Keyword::Woff => "woff",
            Keyword::Woff2 => "woff2",
            Keyword::None | Keyword::Unknown => "unknown",
        }
        .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use style::shared_lock::SharedRwLock;
    use zgui_css::resources::{FontSource, ResourceBase};
    use zgui_dom::host::NoSheetLoader;

    use crate::sheets::origin::SheetOrigin;
    use crate::sheets::{SheetSource, Sheets};

    #[test]
    fn a_face_is_read_with_its_sources_resolved_against_the_sheet_base() {
        let dir = std::env::temp_dir().join(format!("zgui-faces-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("styles")).unwrap();
        let base = ResourceBase::directory(dir.join("styles"))
            .unwrap()
            .with_root(&dir)
            .unwrap();
        let sheets = Sheets::new();
        let lock = SharedRwLock::new();
        let _ = sheets.parse(
            &lock,
            &NoSheetLoader,
            SheetOrigin::Author,
            SheetSource::Based(
                r#"
                @font-face {
                    font-family: "Space Mono";
                    src: url("../fonts/a.woff2") format("woff2"), url(../fonts/a.ttf) format('truetype'),
                         local(Mono);
                }
                @media (min-width: 1px) { @font-face { font-family: B; src: url(data:,x) } }
                @font-face { font-family: NoSource }
                "#,
                &base,
            ),
        );
        let faces = sheets.take_faces();
        assert_eq!(faces.len(), 2, "{faces:?}");
        assert_eq!(faces[0].family, "Space Mono");
        let root = ResourceBase::directory(&dir).unwrap();
        assert_eq!(
            faces[0].sources,
            vec![
                FontSource::Url {
                    url: format!("{}fonts/a.woff2", root.url()),
                    format: Some("woff2".to_owned())
                },
                FontSource::Url {
                    url: format!("{}fonts/a.ttf", root.url()),
                    format: Some("truetype".to_owned())
                },
                FontSource::Local("Mono".to_owned()),
            ]
        );
        assert_eq!(faces[1].family, "B");
        assert!(
            sheets.take_faces().is_empty(),
            "taking them empties the list"
        );
    }
}
