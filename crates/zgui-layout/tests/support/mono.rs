//! A deterministic shaper with no font files behind it.
//!
//! Every character is one cluster of a fixed width, so an inline formatting context can be laid out
//! — and the counters it is supposed to move can be asserted on — without a font engine, a font
//! file or a machine-dependent answer anywhere.

#![allow(dead_code)]

use smallvec::SmallVec;
use zgui_geom::{Css, CssPx, Point, Size};
use zgui_text::{
    BreakRequest, BrokenParagraph, ContentWidths, InlineBoxGeometry, InlineBoxPlacement,
    LineGeometry, ParagraphContent, ParagraphKey, ParagraphShaper, ShapedGlyph, ShapedParagraph,
    ShapedRun, StrutMetrics, TextGeometry,
};
use zgui_text_style::{OverflowWrap, TextAlign, TextStyle, WordBreak, WrapMode};

/// Width of one character, as a fraction of the font size.
pub(crate) const ADVANCE_RATIO: f32 = 0.5;
/// Ascent, as a fraction of the font size.
pub(crate) const ASCENT_RATIO: f32 = 0.8;
/// Descent, as a fraction of the font size.
pub(crate) const DESCENT_RATIO: f32 = 0.2;

/// One shaped cluster.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cluster {
    /// Byte offset of the character in the generated string.
    pub(crate) offset: usize,
    /// Its advance.
    pub(crate) advance: CssPx,
    /// Whether a soft break may be taken before it.
    pub(crate) breakable: bool,
    /// Whether a break must be taken before it.
    pub(crate) forced: bool,
}

/// What this shaper keeps between shaping and breaking.
#[derive(Clone, Debug, Default)]
pub(crate) struct MonoLayout {
    /// The clusters, in logical order.
    pub(crate) clusters: Vec<Cluster>,
    /// The last break's line ranges, as cluster indices.
    pub(crate) lines: Vec<(usize, usize)>,
    /// The atomic inlines each of those lines holds: an index into the paragraph's boxes, and the
    /// advance before the box on its line.
    pub(crate) placed: Vec<Vec<(usize, f32)>>,
    /// The last break's answer, returned again when no pass is owed.
    pub(crate) last: BrokenParagraph,
}

/// The shaper.
#[derive(Debug, Default)]
pub(crate) struct MonoShaper {
    /// How many paragraphs were shaped.
    pub(crate) shapes: u32,
    /// How many breaking passes actually ran.
    pub(crate) breaks: u32,
    /// The widths every breaking pass was asked for, in order.
    pub(crate) widths: Vec<Option<f32>>,
}

impl ParagraphShaper for MonoShaper {
    type Engine = MonoLayout;

    /// A fresh shaper: the metrics are fixed, so a fork answers what the original does.
    fn fork(&self) -> Option<Self> {
        Some(Self::default())
    }

    fn shape(&mut self, content: &ParagraphContent<'_>) -> ShapedParagraph<Self::Engine> {
        self.shapes += 1;
        let size = |offset: usize| {
            content
                .runs
                .iter()
                .find(|run| run.text.contains(&offset))
                .or(content.runs.first())
                .map_or(CssPx(16.0), |run| CssPx(run.style.size.0 * content.scale))
        };

        let mut clusters = Vec::new();
        for (offset, character) in content.text.char_indices() {
            clusters.push(Cluster {
                offset,
                advance: CssPx(size(offset).0 * ADVANCE_RATIO),
                breakable: character == ' ',
                forced: character == '\n',
            });
        }

        let widths = ContentWidths {
            min: longest_word(&clusters, content.boxes),
            max: CssPx(
                clusters
                    .iter()
                    .map(|cluster| cluster.advance.0)
                    .sum::<f32>()
                    + content
                        .boxes
                        .iter()
                        .map(|geometry| geometry.width.0)
                        .sum::<f32>(),
            ),
        };
        let strut = match content.runs.first() {
            Some(run) => self.strut(&run.style),
            None => self.strut(&TextStyle::initial()),
        };
        ShapedParagraph::new(
            ParagraphKey::of(content),
            content.text.to_owned(),
            content.map.clone(),
            widths,
            strut,
            content.boxes.iter().copied(),
            MonoLayout {
                clusters,
                lines: Vec::new(),
                placed: Vec::new(),
                last: BrokenParagraph::default(),
            },
        )
    }

    fn break_lines(
        &mut self,
        shaped: &mut ShapedParagraph<Self::Engine>,
        request: &BreakRequest<'_>,
    ) -> BrokenParagraph {
        if !shaped.begin_break(request) {
            return shaped.engine.last.clone();
        }
        self.breaks += 1;
        self.widths.push(request.max_advance.map(|width| width.0));
        let (lines, placed) = break_into_lines(&shaped.engine.clusters, shaped.boxes(), request);
        shaped.engine.lines = lines;
        shaped.engine.placed = placed;
        let broken = assemble(shaped, request);
        shaped.engine.last = broken.clone();
        broken
    }

    /// One glyph per cluster of the line, laid end to end from the line box's left edge.
    fn visit_line(
        &self,
        shaped: &ShapedParagraph<Self::Engine>,
        line: u16,
        visit: &mut dyn FnMut(ShapedRun<'_>),
    ) {
        let Some((start, end)) = shaped.engine.lines.get(line as usize).copied() else {
            return;
        };
        let baseline = shaped.strut().ascent().0;
        let mut pen = 0.0;
        let mut glyphs = Vec::new();
        for (index, cluster) in shaped.engine.clusters[start..end].iter().enumerate() {
            glyphs.push(ShapedGlyph {
                glyph: (start + index) as u16,
                x: pen,
                y: baseline,
            });
            pen += cluster.advance.0;
        }
        if glyphs.is_empty() {
            return;
        }
        visit(ShapedRun {
            face: zgui_text::FaceId(0),
            size: shaped.strut().font_size.0,
            instance: zgui_text::VariationInstance::DEFAULT,
            synthetic_bold: 0.0,
            synthetic_slant: 0.0,
            has_color: false,
            brush: zgui_scene::PaintSlot(0),
            glyphs: &glyphs,
            text_start: 0,
        });
    }

    /// One cluster per character of the line, laid end to end from the line box's start edge.
    fn visit_clusters(
        &self,
        shaped: &ShapedParagraph<Self::Engine>,
        line: u16,
        visit: &mut dyn FnMut(zgui_text::ClusterRun<'_>),
    ) {
        let Some((start, end)) = shaped.engine.lines.get(line as usize).copied() else {
            return;
        };
        let mut pen = 0.0;
        let mut clusters = Vec::new();
        for (index, cluster) in shaped.engine.clusters[start..end].iter().enumerate() {
            let next = shaped
                .engine
                .clusters
                .get(start + index + 1)
                .map_or(shaped.text().len(), |next| next.offset);
            clusters.push(zgui_text::ClusterGeometry {
                text: cluster.offset..next,
                offset: CssPx(pen),
                advance: cluster.advance,
            });
            pen += cluster.advance.0;
        }
        if clusters.is_empty() {
            return;
        }
        visit(zgui_text::ClusterRun {
            direction: zgui_text::TextDirection::LeftToRight,
            start: CssPx(0.0),
            clusters: &clusters,
        });
    }

    fn strut(&mut self, style: &TextStyle) -> StrutMetrics {
        let ascent = CssPx(style.size.0 * ASCENT_RATIO);
        let descent = CssPx(style.size.0 * DESCENT_RATIO);
        StrutMetrics {
            font_ascent: ascent,
            font_descent: descent,
            line_height: style
                .line_height
                .resolve(style.size, CssPx(ascent.0 + descent.0)),
            x_height: CssPx(style.size.0 * 0.5),
            font_size: style.size,
        }
    }
}

/// The widest thing that cannot be broken: the longest run of clusters between two break
/// opportunities, or the widest box, whichever is more. A box is an opportunity either side of
/// itself, so a word glued to one ends at it.
fn longest_word(clusters: &[Cluster], boxes: &[InlineBoxGeometry]) -> CssPx {
    let mut widest: f32 = 0.0;
    let mut word: f32 = 0.0;
    for item in items(clusters, boxes) {
        match item {
            Item::Box(index) if boxes[index].glued => word += boxes[index].width.0,
            Item::Box(index) => {
                widest = widest.max(word).max(boxes[index].width.0);
                word = 0.0;
            }
            Item::Cluster(index) if clusters[index].breakable || clusters[index].forced => {
                widest = widest.max(word);
                word = 0.0;
            }
            Item::Cluster(index) => word += clusters[index].advance.0,
        }
    }
    CssPx(widest.max(word))
}

/// One thing on a line, in logical order: a character's cluster or an atomic inline.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Item {
    /// An index into the clusters.
    Cluster(usize),
    /// An index into the paragraph's boxes.
    Box(usize),
}

/// The clusters and the boxes in the order a line holds them. A box sits before the character at
/// its offset, after every character and box before it; boxes at one offset keep their order.
fn items(clusters: &[Cluster], boxes: &[InlineBoxGeometry]) -> Vec<Item> {
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by_key(|&index| boxes[index].offset);
    let mut out = Vec::with_capacity(clusters.len() + boxes.len());
    let mut next = 0;
    for (index, cluster) in clusters.iter().enumerate() {
        while next < order.len() && boxes[order[next]].offset <= cluster.offset {
            out.push(Item::Box(order[next]));
            next += 1;
        }
        out.push(Item::Cluster(index));
    }
    out.extend(order[next..].iter().map(|&index| Item::Box(index)));
    out
}

/// Greedy line breaking: take a break at the last opportunity that still fits.
///
/// The opportunities are the engine's: after a space, and before and after every atomic inline,
/// which CSS Text 3 §5.1 treats like an ideographic character (and CSS 2.1 §16.6 leaves wrapping
/// to `white-space`). Chromium 141 breaks the same places: between two inline blocks with nothing
/// between them, and between a word and an inline block that touches it. The edges of a nested
/// inline box are glued and offer none: Chromium does not break `word<span>word</span>word`.
///
/// A space hangs at the end of the line it ends, as CSS Text 3 §4.1.2 says and the engine and
/// Chromium do, so a wrapped line starts at its first word.
///
/// Returns each line's clusters as a range, and the boxes on it with the advance before each.
fn break_into_lines(
    clusters: &[Cluster],
    boxes: &[InlineBoxGeometry],
    request: &BreakRequest<'_>,
) -> (Vec<(usize, usize)>, Vec<Vec<(usize, f32)>>) {
    let wrapping = request
        .runs
        .first()
        .is_none_or(|run| run.style.wrap_mode == WrapMode::Wrap);
    // A word longer than the line is broken only when the style says it may be. Breaking it
    // regardless would make every overflow test pass for the wrong reason.
    let emergency = request.runs.first().is_some_and(|run| {
        run.style.word_break == WordBreak::BreakAll
            || matches!(
                run.style.overflow_wrap,
                OverflowWrap::BreakWord | OverflowWrap::Anywhere
            )
    });
    let items = items(clusters, boxes);
    let width = |item: Item| match item {
        Item::Cluster(index) => clusters[index].advance.0,
        Item::Box(index) => boxes[index].width.0,
    };
    // A break between `at - 1` and `at` is allowed after a space and before and after an atomic
    // inline. An edge of a nested inline box is glued: a start edge goes with what follows it and an
    // end edge with what precedes it, so the opportunity is looked for past them.
    let atomic = |at: usize| matches!(items[at], Item::Box(index) if !boxes[index].glued);
    let edge = |at: usize, opening: bool| matches!(items[at], Item::Box(index) if boxes[index].glued && boxes[index].opens == opening);
    let space = |at: usize| matches!(items[at], Item::Cluster(index) if clusters[index].breakable);
    let opportunity_before = |at: usize| {
        if at == 0 || edge(at - 1, true) || edge(at, false) {
            return false;
        }
        let mut left = at - 1;
        while edge(left, false) {
            if left == 0 {
                return false;
            }
            left -= 1;
        }
        let mut right = at;
        while right < items.len() && edge(right, true) {
            right += 1;
        }
        space(left) || atomic(left) || (right < items.len() && atomic(right))
    };
    let limit = |line: usize| {
        request
            .bands
            .at(line)
            .map(|band| band.max_advance.0)
            .or(request.max_advance.map(|width| width.0))
    };

    // Item ranges first; a forced break's own cluster belongs to neither line.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    let mut advance = request.indent().0;
    let mut last_opportunity = None;
    // The advance of the spaces that end the line so far, which hang.
    let mut hung = 0.0;
    for (at, &item) in items.iter().enumerate() {
        if let Item::Cluster(index) = item
            && clusters[index].forced
        {
            ranges.push((start, at));
            start = at + 1;
            advance = 0.0;
            hung = 0.0;
            last_opportunity = None;
            continue;
        }
        if at > start && opportunity_before(at) {
            last_opportunity = Some(at);
        }
        // A space at the end of a line hangs (CSS Text 3 §4.1.2): it never breaks the line itself,
        // and the break after it leaves it at the end of this one. An edge of an inline box after
        // it is measured without it, since the space still ends the line there.
        let glued = matches!(item, Item::Box(index) if boxes[index].glued);
        let measured = if glued { advance - hung } else { advance };
        let fits = space(at)
            || (glued && width(item) == 0.0)
            || match limit(ranges.len()).filter(|_| wrapping) {
                Some(limit) => measured + width(item) <= limit,
                None => true,
            };
        if space(at) {
            hung += width(item);
        } else if !glued {
            hung = 0.0;
        }
        if !fits && at > start && (last_opportunity.is_some() || emergency) {
            let split = last_opportunity.unwrap_or(at);
            ranges.push((start, split));
            start = split;
            advance = items[start..=at].iter().map(|&held| width(held)).sum();
            hung = 0.0;
            last_opportunity = None;
        } else {
            advance += width(item);
        }
    }
    ranges.push((start, items.len()));

    // The clusters before each item, which turns an item range into a cluster range.
    let mut before = Vec::with_capacity(items.len() + 1);
    let mut count = 0;
    for item in &items {
        before.push(count);
        if matches!(item, Item::Cluster(_)) {
            count += 1;
        }
    }
    before.push(count);
    let mut lines = Vec::with_capacity(ranges.len());
    let mut placed = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        // A forced break's cluster sits between two ranges; the next line starts after it.
        let first = before[start].max(lines.last().map_or(0, |&(_, end): &(usize, usize)| end));
        lines.push((first, before[end].max(first)));
        let mut pen = 0.0;
        let mut on_line = Vec::new();
        for &item in &items[start..end] {
            if let Item::Box(index) = item {
                on_line.push((index, pen));
            }
            pen += width(item);
        }
        placed.push(on_line);
    }
    (lines, placed)
}

/// Builds the geometry the caller reads, from the line ranges the last break produced.
fn assemble(shaped: &ShapedParagraph<MonoLayout>, request: &BreakRequest<'_>) -> BrokenParagraph {
    let strut = shaped.strut();
    let clusters = &shaped.engine.clusters;
    let mut lines = Vec::new();
    let mut boxes: SmallVec<[InlineBoxPlacement; 2]> = SmallVec::new();
    let mut top = CssPx::ZERO;
    let mut widest: f32 = 0.0;

    for (index, (start, end)) in shaped.engine.lines.iter().copied().enumerate() {
        let first = clusters.get(start).map_or(0, |cluster| cluster.offset);
        let past = clusters
            .get(end)
            .map_or(shaped.text().len(), |cluster| cluster.offset);
        let on_line: Vec<(&InlineBoxGeometry, f32)> = shaped.engine.placed[index]
            .iter()
            .map(|&(at, before)| (&shaped.boxes()[at], before))
            .collect();
        let first = match on_line.first() {
            Some((geometry, _)) if start == end => geometry.offset.min(past),
            _ => first,
        };

        let ascent = on_line
            .iter()
            .fold(strut.ascent(), |tallest, (geometry, _)| {
                CssPx(tallest.0.max(geometry.shaper_height().0))
            });
        let descent = on_line
            .iter()
            .fold(strut.descent(), |deepest, (geometry, _)| {
                CssPx(deepest.0.max(geometry.below_baseline().0))
            });
        let text_advance: f32 = clusters[start..end].iter().map(|c| c.advance.0).sum();
        let box_advance: f32 = on_line.iter().map(|(geometry, _)| geometry.width.0).sum();
        let width = text_advance + box_advance;
        let indent = if index == 0 { request.indent().0 } else { 0.0 };
        let band = request.bands.at(index);
        // The spaces that end the line hang (CSS Text 3 §4.1.2): they count in its width, as the
        // engine counts them, and alignment places the line without them, as the engine does.
        let spaces: f32 = clusters[start..end]
            .iter()
            .rev()
            .take_while(|cluster| cluster.breakable)
            .map(|cluster| cluster.advance.0)
            .sum();
        let after_spaces = width - spaces;
        let hanging = if on_line.iter().any(|(geometry, before)| {
            !geometry.glued && *before + geometry.width.0 > after_spaces
        }) {
            0.0
        } else {
            spaces
        };
        let offset = band.map_or(0.0, |band| band.offset.0)
            + align_offset(request, CssPx(width - hanging + indent), band).0
            + indent;

        let baseline = CssPx(top.0 + ascent.0);
        for (geometry, before) in on_line {
            boxes.push(InlineBoxPlacement {
                id: geometry.id,
                origin: Point::new(
                    CssPx(offset + before),
                    CssPx(baseline.0 - geometry.shaper_height().0),
                ),
                line: index,
            });
        }

        widest = widest.max(width + indent + band.map_or(0.0, |band| band.offset.0));
        lines.push(LineGeometry {
            text: first..past,
            top,
            baseline,
            height: CssPx(ascent.0 + descent.0),
            width: CssPx(width),
            offset: CssPx(offset),
        });
        top = CssPx(top.0 + ascent.0 + descent.0);
    }

    BrokenParagraph {
        geometry: std::sync::Arc::new(TextGeometry {
            size: Size::<CssPx, Css>::new(CssPx(widest), top),
            lines,
            is_rtl: false,
        }),
        boxes,
    }
}

/// How far one line is pushed in by alignment.
fn align_offset(
    request: &BreakRequest<'_>,
    width: CssPx,
    band: Option<zgui_text::LineBand>,
) -> CssPx {
    let Some(limit) = band.map(|band| band.max_advance).or(request.max_advance) else {
        return CssPx::ZERO;
    };
    let free = (limit.0 - width.0).max(0.0);
    match request.paragraph.align {
        TextAlign::Center => CssPx(free / 2.0),
        TextAlign::End | TextAlign::Right => CssPx(free),
        _ => CssPx::ZERO,
    }
}
