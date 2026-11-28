//! Greedy line breaking over the clusters and the inline boxes.

use zgui_text::{BreakRequest, InlineBoxGeometry};
use zgui_text_style::WrapMode;

use crate::shaper::cluster::Cluster;

/// One thing on a line, in logical order: a character's cluster or an inline box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Item {
    /// An index into the clusters.
    Cluster(usize),
    /// An index into the paragraph's boxes.
    Box(usize),
}

/// The clusters and the boxes in the order a line holds them. A box sits before the character at
/// its offset, after every character and box before it; boxes at one offset keep their order.
pub(crate) fn items(clusters: &[Cluster], boxes: &[InlineBoxGeometry]) -> Vec<Item> {
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

/// What one break produced: each line's clusters as a half-open range, and the boxes on each line
/// with the advance before each box on it.
pub type Lines = (Vec<(usize, usize)>, Vec<Vec<(usize, f32)>>);

/// Breaks `clusters` and the request's boxes into lines for `request`.
///
/// The rule is greedy: take the last break opportunity that fits, and if none does, overflow the
/// line rather than splitting a word. The opportunities are the ones a browser takes: after a
/// space, and before and after an atomic inline (CSS Text 3 §5.1 treats one like an ideographic
/// character). An edge of a nested inline box is glued: a start edge goes with what follows it and
/// an end edge with what precedes it. A space at the end of a line hangs (CSS Text 3 §4.1.2): it
/// never breaks the line itself, and the break after it leaves it on this line, so a wrapped line
/// starts at its first word. A paragraph that cannot wrap at all — `text-wrap-mode: nowrap` — is
/// one line however wide it is.
pub fn into_lines(clusters: &[Cluster], request: &BreakRequest<'_>) -> Lines {
    let boxes = request.boxes;
    let items = items(clusters, boxes);
    let wrapping = request
        .runs
        .first()
        .is_none_or(|run| run.style.wrap_mode == WrapMode::Wrap);
    let limit = request.max_advance.filter(|_| wrapping);
    let width = |item: Item| match item {
        Item::Cluster(index) => clusters[index].advance.0,
        Item::Box(index) => boxes[index].width.0,
    };
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

    let mut ranges = Vec::new();
    let mut start = 0;
    let mut advance = request.indent().0;
    let mut last_opportunity = None;
    // The advance of the spaces that end the line so far, which hang.
    let mut hung = 0.0;
    if let Some(limit) = limit {
        for (at, &item) in items.iter().enumerate() {
            if at > start && opportunity_before(at) {
                last_opportunity = Some(at);
            }
            // A line with no break opportunity on it overflows rather than splitting a word: there
            // is nowhere legal to break, and inventing a break inside a word is a different feature.
            // A space hangs; an edge of an inline box after it is measured without it.
            let glued = matches!(item, Item::Box(index) if boxes[index].glued);
            let measured = if glued { advance - hung } else { advance };
            let fits =
                space(at) || (glued && width(item) == 0.0) || measured + width(item) <= limit.0;
            if space(at) {
                hung += width(item);
            } else if !glued {
                hung = 0.0;
            }
            if let Some(split) = last_opportunity.filter(|_| !fits && at > start) {
                ranges.push((start, split));
                start = split;
                advance = items[start..=at].iter().map(|&held| width(held)).sum();
                hung = 0.0;
                last_opportunity = None;
            } else {
                advance += width(item);
            }
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
        lines.push((before[start], before[end]));
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use zgui_geom::CssPx;
    use zgui_scene::PaintSlot;
    use zgui_text::{BreakRequest, StyledRun};
    use zgui_text_style::{ParagraphStyle, TextStyle, WrapMode};

    use crate::shaper::cluster::{Cluster, shape};

    use super::into_lines;

    /// The clusters of `text`, in the initial style.
    fn clusters(text: &str) -> Vec<Cluster> {
        let map = zgui_text::TextMap::new();
        let runs = [StyledRun {
            text: 0..text.len(),
            style: Arc::new(TextStyle::initial()),
            brush: PaintSlot(0),
        }];
        let paragraph = ParagraphStyle::initial();
        shape(&zgui_text::ParagraphContent {
            text,
            map: &map,
            runs: &runs,
            boxes: &[],
            paragraph: &paragraph,
            scale: 1.0,
        })
    }

    /// A request at one width, in one style.
    fn request<'a>(
        runs: &'a [StyledRun],
        paragraph: &'a ParagraphStyle,
        width: Option<CssPx>,
    ) -> BreakRequest<'a> {
        BreakRequest {
            runs,
            boxes: &[],
            paragraph,
            max_advance: width,
            indent_basis: width,
            bands: zgui_text::LineBands::NONE,
            probe: false,
        }
    }

    #[test]
    fn a_break_is_taken_at_the_last_space_that_fits() {
        // Eight pixels a cluster: "aaa bbb" is 56 wide, so 40 fits "aaa " and breaks before "bbb".
        // The space hangs at the end of the first line (CSS Text 3 §4.1.2), so the second starts
        // at its word, as Chromium 141 starts it.
        let text = "aaa bbb";
        let runs = [StyledRun {
            text: 0..text.len(),
            style: Arc::new(TextStyle::initial()),
            brush: PaintSlot(0),
        }];
        let paragraph = ParagraphStyle::initial();
        let (lines, _) = into_lines(
            &clusters(text),
            &request(&runs, &paragraph, Some(CssPx(40.0))),
        );
        assert_eq!(lines, vec![(0, 4), (4, 7)]);
    }

    #[test]
    fn a_word_wider_than_the_line_overflows_rather_than_splitting() {
        let text = "aaaaaaaa";
        let runs = [StyledRun {
            text: 0..text.len(),
            style: Arc::new(TextStyle::initial()),
            brush: PaintSlot(0),
        }];
        let paragraph = ParagraphStyle::initial();
        let (lines, _) = into_lines(
            &clusters(text),
            &request(&runs, &paragraph, Some(CssPx(16.0))),
        );
        assert_eq!(lines, vec![(0, 8)]);
    }

    #[test]
    fn nowrap_is_one_line_however_narrow_the_width() {
        let text = "aaa bbb";
        let mut style = TextStyle::initial();
        style.wrap_mode = WrapMode::NoWrap;
        let runs = [StyledRun {
            text: 0..text.len(),
            style: Arc::new(style),
            brush: PaintSlot(0),
        }];
        let paragraph = ParagraphStyle::initial();
        let (lines, _) = into_lines(
            &clusters(text),
            &request(&runs, &paragraph, Some(CssPx(8.0))),
        );
        assert_eq!(lines, vec![(0, 7)]);
    }

    #[test]
    fn no_width_at_all_is_one_line() {
        let text = "aaa bbb";
        let runs = [StyledRun {
            text: 0..text.len(),
            style: Arc::new(TextStyle::initial()),
            brush: PaintSlot(0),
        }];
        let paragraph = ParagraphStyle::initial();
        let (lines, _) = into_lines(&clusters(text), &request(&runs, &paragraph, None));
        assert_eq!(lines, vec![(0, 7)]);
    }
}
