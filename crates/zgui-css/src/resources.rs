//! The files and `data:` URIs a style sheet names, and the rule that decides which may be read.
//!
//! A style sheet names resources with `url()`: the faces of an `@font-face` rule, the pictures of
//! `background-image` and `mask-image`. The parser resolves each reference against the base of the
//! sheet it is written in, so a computed value holds an absolute URL. This module reads that URL.
//!
//! # The policy
//!
//! - A `data:` URI is always read. It carries its own bytes.
//! - A sheet installed with a [`ResourceBase`] resolves its relative references inside the base's
//!   root directory and nowhere else. The base is a URL of the scheme [`ROOT_SCHEME`] whose host
//!   names the root, so resolving a reference cannot leave it: a `..` that would climb above the
//!   root stops at the root, as a `..` above `/` stops at `/`. Reading the file then checks again,
//!   after symbolic links are followed, that it is inside the root.
//! - An absolute `file:` URL is read only when the file is inside the root of some sheet's base.
//! - Every other scheme is refused. There is no network loader.
//!
//! A sheet installed with no base is parsed against `zgui:///`. Its relative references resolve to
//! that scheme, so they are refused, and only its `data:` URIs are read.
//!
//! ```
//! use zgui_css::resources::{self, ResourceError};
//!
//! let bytes = resources::fetch("data:text/plain;base64,aGk=").expect("a data URI is read");
//! assert_eq!(bytes, b"hi");
//! assert!(matches!(
//!     resources::fetch("https://example.com/icon.svg"),
//!     Err(ResourceError::Scheme(_))
//! ));
//! ```

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The scheme of a [`ResourceBase`]'s URL.
pub const ROOT_SCHEME: &str = "zgui-root";

/// Why a resource could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceError {
    /// The URL uses a scheme no loader reads.
    Scheme(String),
    /// The URL names a file outside the directory it may be read from.
    Outside(PathBuf),
    /// The URL is not well formed.
    Malformed(String),
    /// A directory or a file could not be read.
    Io(String),
}

impl core::fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Scheme(url) => write!(formatter, "no loader reads the scheme of `{url}`"),
            Self::Outside(path) => write!(
                formatter,
                "`{}` is outside the directory the style sheet may read",
                path.display()
            ),
            Self::Malformed(url) => write!(formatter, "`{url}` is not a well-formed URL"),
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ResourceError {}

/// Where a sheet's relative references resolve, and which directory its resources may be read
/// from.
///
/// The two are separate because a sheet often names files beside it and above it: a sheet in
/// `build/styles/` that names `../../fonts/a.ttf` resolves against its own directory and reads from
/// the project root.
///
/// ```
/// use zgui_css::resources::ResourceBase;
///
/// let root = std::env::temp_dir().join(format!("zgui-doc-base-{}", std::process::id()));
/// std::fs::create_dir_all(root.join("styles")).unwrap();
/// let base = ResourceBase::directory(root.join("styles")).unwrap().with_root(&root).unwrap();
/// assert!(base.url().starts_with("zgui-root://") && base.url().ends_with("/styles/"));
/// assert_eq!(base.root(), root.canonicalize().unwrap());
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceBase {
    /// The URL relative references resolve against.
    url: String,
    /// The directory the sheet is read from, canonical.
    dir: PathBuf,
    /// The directory resources may be read inside, canonical.
    root: PathBuf,
}

impl ResourceBase {
    /// A base at the directory `dir`, which is also the root.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::Io`] when `dir` does not exist.
    pub fn directory(dir: impl AsRef<Path>) -> Result<Self, ResourceError> {
        let dir = canonical(dir.as_ref())?;
        Ok(Self {
            url: base_url(&dir, &dir),
            root: dir.clone(),
            dir,
        })
    }

    /// The same base with `root` as the directory its resources may be read inside.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::Io`] when `root` does not exist, and [`ResourceError::Outside`]
    /// when the base's directory is not inside `root`.
    pub fn with_root(self, root: impl AsRef<Path>) -> Result<Self, ResourceError> {
        let root = canonical(root.as_ref())?;
        if !self.dir.starts_with(&root) {
            return Err(ResourceError::Outside(self.dir));
        }
        Ok(Self {
            url: base_url(&root, &self.dir),
            dir: self.dir,
            root,
        })
    }

    /// The URL relative references resolve against.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The directory the sheet is read from.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The directory resources may be read inside.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Every root a base was made for, numbered by its position.
static ROOTS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The number `root` goes by in a base URL, registering it the first time.
fn root_number(root: &Path) -> usize {
    let mut roots = ROOTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match roots.iter().position(|held| held == root) {
        Some(at) => at,
        None => {
            roots.push(root.to_path_buf());
            roots.len() - 1
        }
    }
}

/// The root a base URL's host names.
fn root_named(host: &str) -> Option<PathBuf> {
    let number: usize = host.strip_prefix('r')?.parse().ok()?;
    ROOTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(number)
        .cloned()
}

/// The base URL of the directory `dir` inside `root`, with a final `/`.
fn base_url(root: &Path, dir: &Path) -> String {
    let mut url = format!("{ROOT_SCHEME}://r{}/", root_number(root));
    let relative = dir.strip_prefix(root).unwrap_or(Path::new(""));
    for part in relative.components() {
        encode(&part.as_os_str().to_string_lossy(), &mut url);
        url.push('/');
    }
    url
}

/// Appends one path segment, percent-encoded.
fn encode(segment: &str, out: &mut String) {
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
}

/// Whether `path`, canonical, is inside the root of some base.
fn inside_some_root(path: &Path) -> bool {
    ROOTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|root| path.starts_with(root))
}

/// The bytes `url` names, under the policy in the [module documentation](self).
///
/// # Errors
///
/// Returns [`ResourceError`] for a refused scheme, a file outside the directory it may be read
/// from, a malformed URL or a file that cannot be read.
pub fn fetch(url: &str) -> Result<Vec<u8>, ResourceError> {
    if let Some(rest) = strip_scheme(url, "data:") {
        return data(rest).ok_or_else(|| ResourceError::Malformed(url.to_owned()));
    }
    if let Some(rest) = strip_scheme(url, ROOT_SCHEME).and_then(|rest| rest.strip_prefix("://")) {
        let malformed = || ResourceError::Malformed(url.to_owned());
        let rest = rest.split(['?', '#']).next().unwrap_or(rest);
        let (host, path) = rest.split_once('/').ok_or_else(malformed)?;
        let root = root_named(host).ok_or_else(malformed)?;
        let mut file = root.clone();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            let segment =
                String::from_utf8(percent_decode(segment.as_bytes())).map_err(|_| malformed())?;
            if segment == ".." || segment == "." || segment.contains(['/', '\\']) {
                return Err(malformed());
            }
            file.push(segment);
        }
        let file = canonical(&file)?;
        if !file.starts_with(&root) {
            return Err(ResourceError::Outside(file));
        }
        return read(&file);
    }
    if let Some(rest) = strip_scheme(url, "file:") {
        let path = file_path(rest).ok_or_else(|| ResourceError::Malformed(url.to_owned()))?;
        let path = canonical(&path)?;
        if !inside_some_root(&path) {
            return Err(ResourceError::Outside(path));
        }
        return read(&path);
    }
    Err(ResourceError::Scheme(url.to_owned()))
}

/// The bytes of the file at `path`.
fn read(path: &Path) -> Result<Vec<u8>, ResourceError> {
    std::fs::read(path).map_err(|error| ResourceError::Io(format!("{}: {error}", path.display())))
}

/// `url` after `scheme`, matched without regard to case.
fn strip_scheme<'a>(url: &'a str, scheme: &str) -> Option<&'a str> {
    let head = url.get(..scheme.len())?;
    head.eq_ignore_ascii_case(scheme)
        .then(|| &url[scheme.len()..])
}

/// The canonical form of `path`, which is what a root and a check compare.
fn canonical(path: &Path) -> Result<PathBuf, ResourceError> {
    path.canonicalize()
        .map_err(|error| ResourceError::Io(format!("{}: {error}", path.display())))
}

/// The path of a `file:` URL, given what follows the scheme.
///
/// A host other than none or `localhost` names another machine, which is not a file here. A query
/// or a fragment is not part of the path.
fn file_path(rest: &str) -> Option<PathBuf> {
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let path = match rest.strip_prefix("//") {
        Some(authority) => {
            let slash = authority.find('/')?;
            let host = &authority[..slash];
            if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
                return None;
            }
            &authority[slash..]
        }
        None => rest,
    };
    if !path.starts_with('/') {
        return None;
    }
    let bytes = percent_decode(path.as_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        // `/C:/dir/file` on Windows.
        let text = String::from_utf8(bytes).ok()?;
        let text = text.strip_prefix('/').unwrap_or(&text);
        Some(PathBuf::from(text))
    }
}

/// The bytes of a `data:` URI, given what follows the scheme.
fn data(rest: &str) -> Option<Vec<u8>> {
    let comma = rest.find(',')?;
    let (header, payload) = (&rest[..comma], &rest[comma + 1..]);
    let base64 = header
        .rsplit(';')
        .next()
        .is_some_and(|last| last.trim().eq_ignore_ascii_case("base64"));
    let decoded = percent_decode(payload.as_bytes());
    if base64 {
        base64_decode(&decoded)
    } else {
        Some(decoded)
    }
}

/// `bytes` with every `%XX` escape replaced by the byte it names.
fn percent_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(index + 1).and_then(|b| hex(*b)),
                bytes.get(index + 2).and_then(|b| hex(*b)),
            )
        {
            out.push(high << 4 | low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    out
}

/// The value of one hexadecimal digit.
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Decodes base64, ignoring white space and accepting missing padding.
fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for &byte in text {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => continue,
            _ => return None,
        };
        buffer = buffer << 6 | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// One `@font-face` rule, as the faces a font collection registers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFace {
    /// The family name the rule declares, which is what `font-family` names to reach the face.
    pub family: String,
    /// The `src` list, in the order written. A loader takes the first it can read.
    pub sources: Vec<FontSource>,
}

/// One entry of an `@font-face` rule's `src` list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontSource {
    /// A `url()` source, resolved, with its `format()` hint in lower case when one was written.
    Url {
        /// The absolute URL.
        url: String,
        /// The format hint, such as `truetype` or `woff2`.
        format: Option<String>,
    },
    /// A `local()` source: a face installed on the machine, by name.
    Local(String),
}

/// The formats a font collection here reads.
const READABLE_FORMATS: [&str; 4] = ["truetype", "opentype", "collection", "truetype-variations"];

impl FontFace {
    /// The bytes of the first `url()` source that can be read as TrueType or OpenType, with its URL.
    ///
    /// A browser takes the first source it can load, and so does this. A source with a format hint
    /// this collection does not read is skipped without being fetched. A fetched file that is
    /// WOFF or WOFF2 is skipped too: the font stack reads neither, so a rule whose only sources
    /// are WOFF loads nothing.
    ///
    /// # Errors
    ///
    /// When no source loads, returns why each `url()` source was passed over, in order.
    pub fn load(&self) -> Result<(String, Vec<u8>), Vec<String>> {
        let mut passed = Vec::new();
        for source in &self.sources {
            let FontSource::Url { url, format } = source else {
                continue;
            };
            if format
                .as_deref()
                .is_some_and(|format| !READABLE_FORMATS.contains(&format))
            {
                passed.push(format!(
                    "`{url}`: the format is not one this font stack reads"
                ));
                continue;
            }
            match fetch(url) {
                Ok(bytes) if is_sfnt(&bytes) => return Ok((url.clone(), bytes)),
                Ok(_) => passed.push(format!("`{url}`: not a TrueType or OpenType file")),
                Err(error) => passed.push(error.to_string()),
            }
        }
        Err(passed)
    }
}

/// Whether `bytes` start like a TrueType, OpenType or collection file.
fn is_sfnt(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0, 1, 0, 0] | b"OTTO" | b"true" | b"ttcf" | b"typ1")
    )
}

#[cfg(test)]
mod tests {
    use super::{FontFace, FontSource, ResourceBase, ResourceError, fetch};

    /// A directory of its own under the system's temporary directory.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("zgui-resources-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("the scratch directory can be made");
        dir
    }

    #[test]
    fn a_percent_encoded_data_uri_is_read() {
        let bytes = fetch("data:image/svg+xml,%3Csvg%20fill='%23DDDDDD'%3E").expect("readable");
        assert_eq!(bytes, b"<svg fill='#DDDDDD'>");
    }

    #[test]
    fn a_base64_data_uri_is_read_with_or_without_padding() {
        assert_eq!(fetch("data:;base64,aGVsbG8=").unwrap(), b"hello");
        assert_eq!(fetch("data:;base64,aGVsbG8").unwrap(), b"hello");
        assert_eq!(fetch("DATA:,plain").unwrap(), b"plain");
    }

    #[test]
    fn a_base_names_its_directory_inside_its_root_and_reads_only_there() {
        let root = scratch("rooted");
        std::fs::create_dir_all(root.join("styles dir")).unwrap();
        std::fs::create_dir_all(root.join("icons")).unwrap();
        std::fs::write(root.join("icons/a.svg"), b"<svg/>").unwrap();
        let base = ResourceBase::directory(root.join("styles dir"))
            .unwrap()
            .with_root(&root)
            .unwrap();
        assert!(base.url().ends_with("/styles%20dir/"), "{}", base.url());
        let host = base.url().split('/').nth(2).unwrap().to_owned();
        let at = |path: &str| format!("zgui-root://{host}/{path}");
        assert_eq!(fetch(&at("icons/a.svg")).unwrap(), b"<svg/>");
        assert!(matches!(
            fetch(&at("icons/missing.svg")),
            Err(ResourceError::Io(_))
        ));
        assert!(matches!(
            fetch(&at("icons/%2E%2E/a.svg")),
            Err(ResourceError::Malformed(_))
        ));
        assert!(matches!(
            fetch("zgui-root://r999999/a.svg"),
            Err(ResourceError::Malformed(_))
        ));

        // A directory outside the root is no base inside it.
        let elsewhere = scratch("elsewhere");
        assert!(matches!(
            ResourceBase::directory(&elsewhere)
                .unwrap()
                .with_root(&root),
            Err(ResourceError::Outside(_))
        ));
    }

    #[test]
    fn a_file_url_is_read_only_inside_the_root_of_some_base() {
        let root = scratch("file-root");
        std::fs::write(root.join("b.svg"), b"<svg/>").unwrap();
        let outside = scratch("file-outside");
        std::fs::write(outside.join("c.svg"), b"<svg/>").unwrap();
        let _base = ResourceBase::directory(&root).unwrap();
        let url = |path: std::path::PathBuf| format!("file://{}", path.display());
        assert_eq!(fetch(&url(root.join("b.svg"))).unwrap(), b"<svg/>");
        assert!(matches!(
            fetch(&url(outside.join("c.svg"))),
            Err(ResourceError::Outside(_))
        ));
    }

    #[test]
    fn another_machine_and_another_scheme_are_refused() {
        assert!(matches!(
            fetch("file://server/share/a.svg"),
            Err(ResourceError::Malformed(_))
        ));
        assert!(matches!(
            fetch("zgui:///icons/a.svg"),
            Err(ResourceError::Scheme(_))
        ));
    }

    #[test]
    fn a_face_loads_its_first_readable_source() {
        let dir = scratch("faces");
        std::fs::write(dir.join("a.woff2"), b"wOF2....").unwrap();
        std::fs::write(dir.join("b.ttf"), [0u8, 1, 0, 0, 9, 9]).unwrap();
        std::fs::write(dir.join("c.ttf"), b"wOFF....").unwrap();
        let base = ResourceBase::directory(&dir).unwrap();
        let url = |name: &str| format!("{}{name}", base.url());
        let face = FontFace {
            family: "Face".to_owned(),
            sources: vec![
                FontSource::Local("Face".to_owned()),
                FontSource::Url {
                    url: url("a.woff2"),
                    format: Some("woff2".to_owned()),
                },
                FontSource::Url {
                    url: url("c.ttf"),
                    format: None,
                },
                FontSource::Url {
                    url: url("b.ttf"),
                    format: Some("truetype".to_owned()),
                },
            ],
        };
        let (chosen, bytes) = face.load().expect("one source is TrueType");
        assert!(chosen.ends_with("b.ttf"));
        assert_eq!(bytes[..4], [0, 1, 0, 0]);
        let woff_only = FontFace {
            family: "Face".to_owned(),
            sources: vec![FontSource::Url {
                url: url("c.ttf"),
                format: None,
            }],
        };
        assert_eq!(woff_only.load().unwrap_err().len(), 1);
    }
}
