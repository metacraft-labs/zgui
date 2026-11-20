//! A diagnostic probe: what opening a long list costs, and what changing one label in it costs,
//! at several list lengths.
//!
//! ```text
//! cargo run --release -p zgui-bench --bin relabel-slope
//! cargo run --release -p zgui-bench --bin relabel-slope -- 300,1000,3000 41
//! ```
//!
//! Each row is a flex row holding an index and a label, under one scroll pane, as a debugger's
//! event list builds it without windowing. For each length the probe opens the list and reports
//! the first frame's stage times and work counters, then changes one label in the middle many
//! times and reports the median stage times and the counters of one change. A change whose cost
//! does not depend on the list length has the same `layout` and `fragments` times at every
//! length. It gates nothing.

#![forbid(unsafe_code)]

use std::time::Duration;

use zgui::app::Fonts;
use zgui::geom::{DevicePx, Size};
use zgui::platform::SurfaceEvent;
use zgui::prelude::*;
use zgui::reactive::{LocalStorage, RwSignal};
use zgui::render::{RenderTarget, Renderer};
use zgui::runtime::{App, AppError, Runtime};
use zgui::view::{Anchor, BuildCx};
use zgui_platform_headless::Harness;
use zgui_profile::{Counter, counter, latency};

/// How wide the window opens, in CSS pixels.
const WIDTH: f32 = 1000.0;

/// How tall it opens.
const HEIGHT: f32 = 700.0;

/// The sheet.
const SHEET: &str = zgui::css!(
    ":root { background-color: #101318; color: #e6eaf2; font-family: sans-serif; font-size: 13px }
     .pane { display: block; overflow: auto; height: 100% }
     .row { display: flex; flex-direction: row; gap: 8px; padding: 1px 6px; height: 18px }
     .row:nth-child(odd) { background-color: #151a24 }
     .idx { color: #7d879b; width: 48px }"
);

/// The stages a frame passes through, by the latency mark each begins with.
const STAGES: [&str; 5] = ["f.restyle", "f.boxes", "f.layout", "f.fragments", "f.paint"];

/// The counters reported.
const COUNTED: [Counter; 10] = [
    Counter::SelectorMatches,
    Counter::MatchCacheHits,
    Counter::MatchCacheMisses,
    Counter::MatchCacheSkips,
    Counter::StylesShared,
    Counter::ElementsRestyled,
    Counter::NodesRelaidOut,
    Counter::FragmentBoxesVisited,
    Counter::FragmentChildrenAsked,
    Counter::NodesVisited,
];

/// A texture sink that accepts every upload and holds nothing.
struct NullSink;

impl zgui::atlas::TextureSink for NullSink {
    fn create_texture(
        &mut self,
        _texture: zgui::atlas::TextureId,
        _size: Size<i32, zgui::geom::Device>,
        _format: zgui::atlas::TextureFormat,
    ) -> Result<(), zgui::atlas::SinkError> {
        Ok(())
    }

    fn write_texture(
        &mut self,
        _texture: zgui::atlas::TextureId,
        _bounds: zgui::geom::Rect<i32, zgui::geom::Device>,
        _format: zgui::atlas::TextureFormat,
        _bytes: &[u8],
    ) -> Result<(), zgui::atlas::SinkError> {
        Ok(())
    }

    fn destroy_texture(&mut self, _texture: zgui::atlas::TextureId) {}
}

/// A renderer that accepts a frame and does nothing with it, so the number is the pipeline's.
struct Nowhere {
    /// The surface it was configured for.
    target: Option<RenderTarget>,
    /// Where tiles are uploaded.
    sink: NullSink,
}

impl Renderer for Nowhere {
    fn capabilities(&self) -> zgui::render::RenderCapabilities {
        zgui::render::RenderCapabilities::MINIMAL
    }

    fn configure(&mut self, target: RenderTarget) {
        self.target = Some(target);
    }

    fn target(&self) -> Option<RenderTarget> {
        self.target
    }

    fn draw(
        &mut self,
        _scene: &zgui::scene::Scene,
        _damage: &zgui::bits::DamageSet,
    ) -> zgui::render::FrameOutcome {
        zgui::render::FrameOutcome::Presented(zgui::render::FrameStats::default())
    }

    fn register_external(
        &mut self,
        _texture: zgui::render::ExternalTexture,
    ) -> zgui::render::TextureHandle {
        zgui::render::TextureHandle(0)
    }

    fn release_external(&mut self, _handle: zgui::render::TextureHandle) {}

    fn memory(&self) -> zgui::render::MemoryReport {
        zgui::render::MemoryReport::ZERO
    }

    fn texture_sink(&mut self) -> &mut dyn zgui::atlas::TextureSink {
        &mut self.sink
    }
}

/// Builds the renderer a window draws through.
fn nowhere(
    _surface: &std::sync::Arc<dyn zgui::platform::Surface>,
    target: RenderTarget,
) -> Result<Box<dyn Renderer>, AppError> {
    Ok(Box::new(Nowhere {
        target: Some(target),
        sink: NullSink,
    }))
}

/// The label of row `index`.
fn label(index: usize) -> String {
    format!("event {index}: step into fn_{}", index % 97)
}

/// Opens a list of `rows` rows whose middle label reads `changed`, and settles it.
fn opened(rows: usize, changed: RwSignal<usize, LocalStorage>) -> Harness<Runtime> {
    let fonts = Fonts::system();
    let metrics = fonts.clone();
    let shaping = fonts.clone();
    let raster = fonts.clone();
    let middle = rows / 2;
    let handler = App::new()
        .with_title("relabel-slope")
        .with_size(WIDTH, HEIGHT)
        .with_stylesheet(SHEET)
        .with_renderer(Box::new(nowhere))
        .with_metrics(Box::new(move || metrics.metrics()))
        .with_text_engine(Box::new(move || {
            Box::new(zgui_layout::Paragraphs::new(shaping.shaper()))
        }))
        .with_glyph_raster(Box::new(move || raster.raster()))
        .into_handler(move |cx: &mut BuildCx<'_>| -> Box<dyn Anchor> {
            let mut pane = zgui::elements::scroll().class("pane");
            for index in 0..rows {
                let index_text = zgui::elements::text().class("idx").child(index.to_string());
                let row = zgui::elements::row().class("row").child(index_text);
                pane = if index == middle {
                    pane.child(row.child(move || format!("event {}: changed", changed.get())))
                } else {
                    pane.child(row.child(label(index)))
                };
            }
            Box::new(pane.into_view().build(cx))
        })
        .expect("the reactive runtime installs");
    let mut harness = Harness::new(handler);
    harness.deliver_to_first(SurfaceEvent::Resized(Size::new(
        DevicePx(WIDTH),
        DevicePx(HEIGHT),
    )));
    harness.settle(512);
    harness
}

/// The time each stage took since the marks were cleared, in microseconds.
fn stage_times() -> [f64; STAGES.len()] {
    let marks = latency::recent();
    let mut totals = [0u128; STAGES.len()];
    for pair in marks.windows(2) {
        if let Some(stage) = STAGES.iter().position(|name| pair[0].stage == *name) {
            totals[stage] += pair[1].at_ns.saturating_sub(pair[0].at_ns);
        }
    }
    totals.map(|ns| ns as f64 / 1_000.0)
}

/// The counters' current values.
fn counters() -> [u64; COUNTED.len()] {
    COUNTED.map(counter::get)
}

/// What the counters moved by since `before`.
fn moved(before: [u64; COUNTED.len()]) -> String {
    let after = counters();
    COUNTED
        .iter()
        .zip(before.iter().zip(after.iter()))
        .map(|(name, (before, after))| format!("{name:?}={}", after - before))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The median of `values`.
fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Opens one list and changes its label `iterations` times, printing what each cost.
fn measure(rows: usize, iterations: usize) {
    let changed = RwSignal::new_local(0usize);
    latency::clear();
    let before = counters();
    let opened_at = std::time::Instant::now();
    let mut harness = opened(rows, changed);
    let open_ms = opened_at.elapsed().as_secs_f64() * 1_000.0;
    let stages = stage_times();
    println!(
        "OPEN rows={rows} open_ms={open_ms:.1} restyle_us={:.0} boxes_us={:.0} layout_us={:.0} \
         fragments_us={:.0} paint_us={:.0} {}",
        stages[0],
        stages[1],
        stages[2],
        stages[3],
        stages[4],
        moved(before)
    );
    let mut per_stage: Vec<Vec<f64>> = vec![Vec::new(); STAGES.len()];
    let mut last = String::new();
    for step in 1..=iterations {
        latency::clear();
        let before = counters();
        changed.set(step);
        harness.advance(Duration::from_micros(16_667));
        harness.settle(64);
        for (stage, time) in stage_times().iter().enumerate() {
            per_stage[stage].push(*time);
        }
        last = moved(before);
    }
    let medians: Vec<f64> = per_stage.into_iter().map(median).collect();
    println!(
        "RELABEL rows={rows} restyle_us={:.1} boxes_us={:.1} layout_us={:.1} fragments_us={:.1} \
         paint_us={:.1} {last}",
        medians[0], medians[1], medians[2], medians[3], medians[4]
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let sizes: Vec<usize> = args
        .next()
        .map(|sizes| {
            sizes
                .split(',')
                .filter_map(|size| size.parse().ok())
                .collect()
        })
        .unwrap_or_else(|| vec![300, 1_000, 3_000]);
    let iterations = args.next().and_then(|n| n.parse().ok()).unwrap_or(41);
    latency::retain_marks(1 << 18);
    // The first window in a process loads the fonts and warms the engine's caches.
    drop(opened(10, RwSignal::new_local(0)));
    for rows in sizes {
        measure(rows, iterations);
    }
}
