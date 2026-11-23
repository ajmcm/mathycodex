//! Background math rasterization; terminal transport remains owned by transcript images.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::PoisonError;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use image::RgbaImage;

use crate::pets::ImageProtocol;
use crate::transcript_image::TranscriptImage;
use crate::transcript_image::terminal_cell_size;
use crate::tui::FrameRequester;

const MAX_CACHE_ENTRIES: usize = 64;
const MAX_CACHE_BYTES: usize = 8 * 1024 * 1024;
static REVISION: AtomicU64 = AtomicU64::new(0);
static RENDERER: OnceLock<Renderer> = OnceLock::new();

#[derive(Clone, PartialEq, Eq)]
struct Key {
    source: String,
    font_size: u16,
    max_width: u32,
    foreground: (u8, u8, u8),
}

enum State {
    Pending,
    Ready(Arc<RgbaImage>),
    Failed,
}

struct Renderer {
    cache: Arc<Mutex<VecDeque<(Key, State)>>>,
    requests: mpsc::SyncSender<Key>,
}

/// One serial worker and a bounded queue keep rendering off the input/paint thread.
pub(crate) fn init(frame_requester: FrameRequester) {
    RENDERER.get_or_init(|| {
        let cache = Arc::new(Mutex::new(VecDeque::<(Key, State)>::new()));
        let worker_cache = Arc::clone(&cache);
        let (requests, receiver) = mpsc::sync_channel::<Key>(16);
        let spawned = std::thread::Builder::new()
            .name("mathycodex-renderer".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                while let Ok(key) = receiver.recv() {
                    let pixels = mathycodex_renderer::render_for_terminal(
                        &key.source,
                        key.font_size,
                        key.foreground,
                        key.max_width,
                    )
                    .and_then(|png| {
                        image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()
                    })
                    .map(|image| Arc::new(image.to_rgba8()));
                    let mut cache = worker_cache.lock().unwrap_or_else(PoisonError::into_inner);
                    let retained_bytes = cache
                        .iter()
                        .map(|(_, state)| match state {
                            State::Ready(pixels) => pixels.len(),
                            _ => 0,
                        })
                        .sum::<usize>();
                    // Keep an over-budget result as a cached failure. Evicting other
                    // visible formulas here would cause redraw/re-render oscillation.
                    let pixels =
                        pixels.filter(|pixels| retained_bytes + pixels.len() <= MAX_CACHE_BYTES);
                    if let Some((_, state)) = cache.iter_mut().find(|(cached, _)| cached == &key) {
                        *state = pixels.map_or(State::Failed, State::Ready);
                    }
                    drop(cache);
                    REVISION.fetch_add(1, Ordering::Relaxed);
                    frame_requester.schedule_frame();
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "could not start math renderer; retaining text math");
        }
        Renderer { cache, requests }
    });
}

pub(crate) fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

fn request(key: Key) -> Option<Arc<RgbaImage>> {
    let renderer = RENDERER.get()?;
    let mut cache = renderer
        .cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(index) = cache.iter().position(|(cached, _)| cached == &key) {
        let entry = cache.remove(index)?;
        let pixels = match &entry.1 {
            State::Ready(pixels) => Some(Arc::clone(pixels)),
            _ => None,
        };
        cache.push_back(entry);
        return pixels;
    }
    if cache.len() >= MAX_CACHE_ENTRIES {
        cache.pop_front();
    }
    cache.push_back((key.clone(), State::Pending));
    // The lock prevents a fast worker from publishing before the pending entry exists.
    if renderer.requests.try_send(key).is_err() {
        cache.pop_back();
    }
    None
}

pub(crate) struct PreparedMath {
    pub(crate) marker: String,
    pub(crate) source: String,
    pub(crate) image: Arc<TranscriptImage>,
}

/// Completed $$ and \[ display blocks get their own rows even when embedded in prose.
/// Single-$ inline math continues through Codex's Unicode renderer.
pub(crate) fn prepare(source: &str, width: u16) -> Option<(String, Vec<PreparedMath>)> {
    if width < 24
        || !crate::markdown_render::preferences::current().math
        || std::env::var_os("MATHYCODEX_DISABLE_LATEX").is_some()
    {
        return None;
    }
    let (cell_width, cell_height) = terminal_cell_size()?;
    let foreground = crate::terminal_palette::default_fg()?;
    let font_size = cell_height.clamp(8, 64) as u16;
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let mut rewritten = String::new();
    let mut prepared = Vec::new();
    let mut offset = 0;
    for formula in mathycodex_renderer::formulas(source) {
        let Some(pixels) = request(Key {
            source: formula.source.to_owned(),
            font_size,
            max_width: u32::from(width.saturating_sub(2)) * cell_width,
            foreground,
        }) else {
            continue;
        };
        let Some(image) = cell_aligned_image(&pixels, width, (cell_width, cell_height)) else {
            continue;
        };
        let marker = format!("M{}Q{}", &nonce[..8], prepared.len());
        if source.contains(&marker) {
            continue;
        }
        rewritten.push_str(&source[offset..formula.range.start]);
        rewritten.push_str("\n\n");
        rewritten.push_str(&marker);
        rewritten.push_str("\n\n");
        prepared.push(PreparedMath {
            marker,
            source: source[formula.range.clone()].to_owned(),
            image: Arc::new(image),
        });
        offset = formula.range.end;
    }
    if prepared.is_empty() {
        return None;
    }
    rewritten.push_str(&source[offset..]);
    Some((rewritten, prepared))
}

/// Pad the supersampled raster to exact cell dimensions. Kitty's only resize is
/// now the uniform 2:1 sample reduction: glyphs are never stretched to fill cells.
fn cell_aligned_image(
    pixels: &RgbaImage,
    width: u16,
    (cell_width, cell_height): (u32, u32),
) -> Option<TranscriptImage> {
    let scale = mathycodex_renderer::TERMINAL_PIXEL_SCALE;
    let raster_cell_width = cell_width.checked_mul(scale)?;
    let raster_cell_height = cell_height.checked_mul(scale)?;
    if raster_cell_width == 0
        || raster_cell_height == 0
        || pixels.width() == 0
        || pixels.height() == 0
    {
        return None;
    }
    let columns = u16::try_from(pixels.width().div_ceil(raster_cell_width)).ok()?;
    let rows = u16::try_from(pixels.height().div_ceil(raster_cell_height)).ok()?;
    if columns > width.saturating_sub(2) || rows > 32 {
        return None;
    }
    let padded_width = u32::from(columns).checked_mul(raster_cell_width)?;
    let padded_height = u32::from(rows).checked_mul(raster_cell_height)?;
    if u64::from(padded_width) * u64::from(padded_height) > 2 * 1024 * 1024 {
        return None;
    }
    let mut padded = RgbaImage::new(padded_width, padded_height);
    image::imageops::replace(&mut padded, pixels, 0, 0);
    Some(TranscriptImage {
        protocol: ImageProtocol::Kitty,
        columns,
        rows,
        pixels: padded,
        cell_width,
        cell_height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn kitty_math_pads_cells_without_resampling_glyphs() {
        let pixels = RgbaImage::from_fn(21, 37, |x, y| Rgba([x as u8, y as u8, 90, 255]));
        let image = cell_aligned_image(&pixels, 40, (8, 16)).unwrap();
        assert_eq!((image.columns, image.rows), (2, 2));
        assert_eq!(image.pixels.dimensions(), (32, 64));
        for (x, y, pixel) in pixels.enumerate_pixels() {
            assert_eq!(image.pixels.get_pixel(x, y), pixel);
        }
        assert_eq!(image.pixels.get_pixel(21, 0).0, [0, 0, 0, 0]);
        assert_eq!(image.pixels.get_pixel(0, 37).0, [0, 0, 0, 0]);
    }

    #[test]
    fn kitty_math_padding_preserves_uniform_scale_for_clipped_rows() {
        let image = cell_aligned_image(&RgbaImage::new(83, 71), 40, (9, 19)).unwrap();
        assert_eq!(image.pixels.width(), u32::from(image.columns) * 9 * 2);
        assert_eq!(image.pixels.height(), u32::from(image.rows) * 19 * 2);
        for row in 0..u32::from(image.rows) {
            assert_eq!(
                row * image.pixels.height() / u32::from(image.rows),
                row * 19 * 2
            );
        }
    }

    #[test]
    fn kitty_math_padding_retains_text_fallback_for_invalid_geometry() {
        let pixels = RgbaImage::new(100, 100);
        assert!(cell_aligned_image(&pixels, 4, (8, 16)).is_none());
        assert!(cell_aligned_image(&pixels, 40, (0, 16)).is_none());
        assert!(cell_aligned_image(&pixels, 40, (u32::MAX, 16)).is_none());
        assert!(cell_aligned_image(&pixels, 40, (1024, 1024)).is_none());
        assert!(cell_aligned_image(&RgbaImage::new(100, 1100), 40, (8, 16)).is_none());
    }
}
