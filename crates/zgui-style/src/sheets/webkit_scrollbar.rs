//! `::-webkit-scrollbar` rules, carried to the scroll container they style.
//!
//! A sheet written for a browser sizes and colours its scrollbars through the `::-webkit-scrollbar`
//! pseudo-elements, which Chromium and WebKit implement and the style engine in this build does not
//! know: a selector naming one fails to parse, and the whole rule is dropped. The bars then keep the
//! theme's width, and every scroll container's content is laid out narrower than the sheet's author
//! measured it.
//!
//! The pseudo-elements style one element, the scroll container they belong to, so each rule is
//! rewritten before parsing into a rule on that element, with the declarations this framework reads
//! for its bars:
//!
//! | pseudo-element | declaration | becomes |
//! |---|---|---|
//! | `::-webkit-scrollbar` | `width` (and `height` for a horizontal bar) | `--zgui-scrollbar-width` |
//! | `::-webkit-scrollbar` | `display: none` | `--zgui-scrollbar-width: 0px` |
//! | `::-webkit-scrollbar`, `-track` | `background`, `background-color` | `--zgui-scrollbar-track` |
//! | `::-webkit-scrollbar-thumb` | `background`, `background-color` | `--zgui-scrollbar-thumb` |
//!
//! `S::-webkit-scrollbar…` becomes a rule on `S`; a bare `::-webkit-scrollbar…` becomes a rule on
//! `*`, every element. A pseudo-class after the pseudo-element names a state of the bar itself
//! (`:hover`, `:active`, `:window-inactive`); a rule with one is dropped, because the bar has no
//! such state here. `:vertical` and `:horizontal` are kept and name the same declarations. Other
//! declarations, and the other pseudo-elements (`-corner`, `-button`, `-track-piece`), have no
//! counterpart and are dropped, as the browser's parser drops what it does not know.
//!
//! `--zgui-scrollbar-width` is registered (`@property`, a `<length>` that does not inherit), so a
//! font-relative width resolves to pixels in the cascade and a container's width does not leak to
//! the scroll containers inside it. Its initial value, `auto`, means the theme's width.

use std::borrow::Cow;

/// The custom property that carries a scroll container's bar width.
pub const WIDTH: &str = "--zgui-scrollbar-width";

const PSEUDO: &str = "::-webkit-scrollbar";

/// The registration the rewritten rules need, put ahead of them.
const REGISTRATION: &str = "@property --zgui-scrollbar-width { syntax: \"<length> | auto\"; inherits: false; \
     initial-value: auto; }\n";

/// `css` with every `::-webkit-scrollbar` rule carried to its scroll container. Borrows `css`
/// unchanged when it names no such pseudo-element.
pub fn rewrite(css: &str) -> Cow<'_, str> {
    if !css.contains(PSEUDO) {
        return Cow::Borrowed(css);
    }
    let mut out = String::with_capacity(css.len() + REGISTRATION.len());
    // An `@charset` or `@import` must stay first; the registration goes after any.
    let (head, body) = split_leading_imports(css);
    out.push_str(head);
    out.push_str(REGISTRATION);
    rewrite_block(body, &mut out);
    Cow::Owned(out)
}

/// The leading `@charset` / `@import` statements of a sheet, and the rest.
fn split_leading_imports(css: &str) -> (&str, &str) {
    let mut at = 0;
    loop {
        let rest = &css[at..];
        let trimmed = skip_space_and_comments(rest);
        let skipped = rest.len() - trimmed.len();
        if trimmed.starts_with("@charset") || trimmed.starts_with("@import") {
            match statement_end(trimmed) {
                Some(end) => at += skipped + end,
                None => return (css, ""),
            }
        } else {
            return (&css[..at], &css[at..]);
        }
    }
}

/// Copies the rules of one block (a sheet, or a grouping at-rule's body) into `out`, rewriting the
/// `::-webkit-scrollbar` ones.
fn rewrite_block(block: &str, out: &mut String) {
    let mut rest = block;
    while !rest.is_empty() {
        let Some(open) = find_top_level(rest, &['{', ';']) else {
            out.push_str(rest);
            return;
        };
        if rest.as_bytes()[open] == b';' {
            // A statement at-rule (`@import`, `@namespace`, `@layer a, b;`).
            out.push_str(&rest[..=open]);
            rest = &rest[open + 1..];
            continue;
        }
        let Some(close) = matching_brace(rest, open) else {
            out.push_str(rest);
            return;
        };
        let prelude = &rest[..open];
        let body = &rest[open + 1..close];
        let name = skip_space_and_comments(prelude);
        if name.starts_with('@') {
            if is_grouping(name) {
                out.push_str(prelude);
                out.push('{');
                rewrite_block(body, out);
                out.push('}');
            } else {
                out.push_str(&rest[..=close]);
            }
        } else if prelude.contains(PSEUDO) {
            rewrite_rule(prelude, body, out);
        } else {
            out.push_str(&rest[..=close]);
        }
        rest = &rest[close + 1..];
    }
}

/// Whether an at-rule holds rules rather than declarations.
fn is_grouping(at_rule: &str) -> bool {
    [
        "@media",
        "@supports",
        "@layer",
        "@container",
        "@document",
        "@scope",
    ]
    .iter()
    .any(|name| at_rule.starts_with(name))
}

/// Which bar part a selector's pseudo-element names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Bar,
    Track,
    Thumb,
}

/// Rewrites one rule whose selector list names a scrollbar pseudo-element.
fn rewrite_rule(prelude: &str, body: &str, out: &mut String) {
    let mut plain: Vec<&str> = Vec::new();
    for selector in split_top_level(prelude, ',') {
        let selector = selector.trim();
        let Some(at) = selector.find(PSEUDO) else {
            plain.push(selector);
            continue;
        };
        let originating = selector[..at].trim();
        let after = &selector[at + PSEUDO.len()..];
        let (part, states) = if let Some(states) = after.strip_prefix("-thumb") {
            (Part::Thumb, states)
        } else if let Some(states) = after.strip_prefix("-track") {
            if states.starts_with("-piece") {
                continue;
            }
            (Part::Track, states)
        } else if after.starts_with('-') {
            // `-corner`, `-button`, `-resizer`: no counterpart.
            continue;
        } else {
            (Part::Bar, after)
        };
        if !states_are_kept(states) {
            continue;
        }
        let declarations = carried(part, body);
        if declarations.is_empty() {
            continue;
        }
        out.push_str(if originating.is_empty() {
            "*"
        } else {
            originating
        });
        out.push_str(" {");
        out.push_str(&declarations);
        out.push_str("}\n");
    }
    if !plain.is_empty() {
        out.push_str(&plain.join(", "));
        out.push('{');
        out.push_str(body);
        out.push_str("}\n");
    }
}

/// Whether the pseudo-classes after a scrollbar pseudo-element name nothing but an orientation.
fn states_are_kept(states: &str) -> bool {
    let mut rest = states.trim();
    while let Some(next) = rest.strip_prefix(':') {
        let end = next
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or(next.len());
        match &next[..end] {
            "vertical" | "horizontal" => rest = next[end..].trim(),
            _ => return false,
        }
    }
    rest.is_empty()
}

/// The declarations a bar part's rule carries to its scroll container.
fn carried(part: Part, body: &str) -> String {
    let mut out = String::new();
    for declaration in split_top_level(body, ';') {
        let Some(colon) = declaration.find(':') else {
            continue;
        };
        let name = declaration[..colon].trim().to_ascii_lowercase();
        let value = declaration[colon + 1..].trim();
        if value.is_empty() {
            continue;
        }
        let target = match (part, name.as_str()) {
            (Part::Bar, "width") => Some((WIDTH, value.to_owned())),
            (Part::Bar, "display") if value.trim_end_matches("!important").trim() == "none" => {
                Some((WIDTH, important_like(value, "0px")))
            }
            (Part::Bar | Part::Track, "background" | "background-color") => {
                Some(("--zgui-scrollbar-track", value.to_owned()))
            }
            (Part::Thumb, "background" | "background-color") => {
                Some(("--zgui-scrollbar-thumb", value.to_owned()))
            }
            _ => None,
        };
        if let Some((property, value)) = target {
            out.push(' ');
            out.push_str(property);
            out.push_str(": ");
            out.push_str(&value);
            out.push(';');
        }
    }
    out
}

/// `replacement`, keeping `!important` when `value` had it.
fn important_like(value: &str, replacement: &str) -> String {
    if value.contains("!important") {
        format!("{replacement} !important")
    } else {
        replacement.to_owned()
    }
}

/// `text` without its leading whitespace and comments.
fn skip_space_and_comments(mut text: &str) -> &str {
    loop {
        let trimmed = text.trim_start();
        if let Some(comment) = trimmed.strip_prefix("/*") {
            match comment.find("*/") {
                Some(end) => text = &comment[end + 2..],
                None => return "",
            }
        } else {
            return trimmed;
        }
    }
}

/// The byte just past the `;` that ends a statement, outside strings and brackets.
fn statement_end(text: &str) -> Option<usize> {
    find_top_level(text, &[';']).map(|at| at + 1)
}

/// The first of `targets` at nesting depth zero, outside strings and comments.
fn find_top_level(text: &str, targets: &[char]) -> Option<usize> {
    let mut depth = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' | '\'' => skip_string(&mut chars, c),
            '/' if chars.peek().map(|&(_, n)| n) == Some('*') => skip_comment(&mut chars),
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 && targets.contains(&c) => return Some(at),
            _ => {}
        }
    }
    None
}

/// The `}` that closes the `{` at `open`.
fn matching_brace(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut chars = text[open..].char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' | '\'' => skip_string(&mut chars, c),
            '/' if chars.peek().map(|&(_, n)| n) == Some('*') => skip_comment(&mut chars),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + at);
                }
            }
            _ => {}
        }
    }
    None
}

/// `text` split at each `separator` at nesting depth zero, outside strings and comments.
fn split_top_level(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' | '\'' => skip_string(&mut chars, c),
            '/' if chars.peek().map(|&(_, n)| n) == Some('*') => skip_comment(&mut chars),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && c == separator => {
                parts.push(&text[start..at]);
                start = at + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn skip_string(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, quote: char) {
    while let Some((_, c)) = chars.next() {
        if c == '\\' {
            chars.next();
        } else if c == quote {
            return;
        }
    }
}

fn skip_comment(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) {
    chars.next();
    let mut star = false;
    for (_, c) in chars.by_ref() {
        if star && c == '/' {
            return;
        }
        star = c == '*';
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite;

    #[test]
    fn a_sheet_without_the_pseudo_element_is_borrowed_unchanged() {
        let css = ".a { color: red }";
        assert!(matches!(rewrite(css), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn each_rule_moves_to_its_scroll_container() {
        let out = rewrite(
            "::-webkit-scrollbar { width: 0.5rem; height: 0.5rem; background-color: transparent }\n\
             .s::-webkit-scrollbar { width: 0 }\n\
             ::-webkit-scrollbar-thumb:vertical, ::-webkit-scrollbar-thumb:horizontal { background-color: #333 }\n\
             ::-webkit-scrollbar-thumb:vertical:hover { background-color: #414141 }\n\
             ::-webkit-scrollbar-corner { background-color: #282828 }\n\
             .h::-webkit-scrollbar { display: none !important }\n\
             .a, .b::-webkit-scrollbar { width: 3px }",
        );
        assert!(out.starts_with("@property --zgui-scrollbar-width"));
        assert!(
            out.contains(
                "* { --zgui-scrollbar-width: 0.5rem; --zgui-scrollbar-track: transparent;}"
            )
        );
        assert!(out.contains(".s { --zgui-scrollbar-width: 0;}"));
        assert!(out.contains("* { --zgui-scrollbar-thumb: #333;}"));
        assert!(
            !out.contains("#414141"),
            "a state of the bar has no counterpart"
        );
        assert!(!out.contains("#282828"), "nor has the corner");
        assert!(out.contains(".h { --zgui-scrollbar-width: 0px !important;}"));
        assert!(out.contains(".b { --zgui-scrollbar-width: 3px;}"));
        assert!(
            out.contains(".a{ width: 3px }"),
            "the plain selector keeps the rule"
        );
    }

    #[test]
    fn rules_inside_a_grouping_rule_are_rewritten_and_others_are_kept() {
        let out = rewrite(
            "@import url(a.css);\n\
             @media (min-width: 10px) { .s::-webkit-scrollbar { width: 4px } .t { color: red } }\n\
             @font-face { font-family: x; src: url(\"a{b}.ttf\") }",
        );
        assert!(
            out.starts_with("@import url(a.css);"),
            "an import stays first"
        );
        assert!(out.contains("@media (min-width: 10px) {"));
        assert!(out.contains(".s { --zgui-scrollbar-width: 4px;}"));
        assert!(out.contains(".t { color: red }"));
        assert!(
            out.contains("src: url(\"a{b}.ttf\")"),
            "a brace in a string is not a block"
        );
    }
}
