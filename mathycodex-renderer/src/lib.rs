//! Bounded, in-process LaTeX rendering. No TeX executable, shell, or browser is invoked.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag};
use ratex_layout::{LayoutOptions, layout, to_display_list, validate_layout};
use ratex_render::{RenderOptions, render_to_png};
use ratex_types::{Color, MathStyle};

pub const MAX_SOURCE_BYTES: usize = 4096;
pub const MAX_FORMULAS: usize = 16;
const MAX_DOCUMENT_BYTES: usize = 128 * 1024;
const MAX_PIXELS: f64 = 2.0 * 1024.0 * 1024.0;

/// Raster samples per terminal pixel. Layout dimensions remain in terminal pixels.
pub const TERMINAL_PIXEL_SCALE: u32 = 2;

#[derive(Debug, PartialEq, Eq)]
pub struct Formula<'a> {
    pub range: Range<usize>,
    pub source: &'a str,
}

/// Find closed $$…$$ and \[…\] blocks, including blocks surrounded by prose.
/// Markdown code, links, HTML, tables, and nested containers stay under the normal renderer.
/// Only exactly two unescaped dollars delimit math; single dollars stay with the upstream text renderer.
pub fn formulas(input: &str) -> Vec<Formula<'_>> {
    if input.len() > MAX_DOCUMENT_BYTES || (!input.contains("$$") && !input.contains(r"\[")) {
        return Vec::new();
    }
    let parser = Parser::new_ext(
        input,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    );
    let mut protected: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(_, definition)| definition.span.clone())
        .collect();
    let mut formatting = Vec::new();
    protected.extend(parser.into_offset_iter().filter_map(|(event, range)| {
        // Markdown may interpret LaTeX subscripts and superscripts as emphasis.
        // Exclude equations wrapped in formatting, not formatting inside math.
        if matches!(
            event,
            Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Heading { .. })
        ) {
            formatting.push(range);
            return None;
        }
        matches!(
            event,
            Event::Code(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::Start(
                    Tag::CodeBlock(_)
                        | Tag::Link { .. }
                        | Tag::Image { .. }
                        | Tag::List(_)
                        | Tag::BlockQuote(_)
                        | Tag::Table(_)
                )
        )
        .then_some(range)
    }));
    protected.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in protected {
        if let Some(previous) = merged.last_mut()
            && previous.end >= range.start
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    let protected_at = |offset| merged.get(merged.partition_point(|range| range.end <= offset));
    let is_delimiter = |index: usize, delimiter: &str| {
        let before = &input[..index];
        let slashes = before
            .bytes()
            .rev()
            .take_while(|byte| *byte == b'\\')
            .count();
        slashes % 2 == 0
            && (delimiter != "$$"
                || (!before.ends_with('$') && !input[index + 2..].starts_with('$')))
    };
    let mut result = Vec::new();
    let mut offset = 0;
    loop {
        let next = [("$$", "$$"), (r"\[", r"\]")]
            .into_iter()
            .filter_map(|(open, close)| {
                input[offset..]
                    .find(open)
                    .map(|relative| (offset + relative, open, close))
            })
            .min_by_key(|(start, _, _)| *start);
        let Some((start, open, close)) = next else {
            break;
        };
        offset = start + open.len();
        if !is_delimiter(start, open)
            || formatting
                .iter()
                .any(|range| range.start < start && range.contains(&start))
            || protected_at(start).is_some_and(|range| range.contains(&start))
        {
            continue;
        }
        let mut closing = offset;
        let end = loop {
            let Some(relative) = input[closing..].find(close) else {
                break None;
            };
            let end = closing + relative;
            closing = end + close.len();
            if is_delimiter(end, close) {
                break Some(end);
            }
        };
        let Some(end) = end else { break };
        offset = end + close.len();
        let source = &input[start + open.len()..end];
        if source.trim().is_empty()
            || source.len() > MAX_SOURCE_BYTES
            || protected_at(start).is_some_and(|range| range.start < offset)
        {
            continue;
        }
        result.push(Formula {
            range: start..offset,
            source,
        });
        if result.len() == MAX_FORMULAS {
            break;
        }
    }
    result
}

/// Reject excessive nesting before entering a third-party parser.
pub fn bounded_source(source: &str) -> bool {
    if source.trim().is_empty()
        || source.len() > MAX_SOURCE_BYTES
        || source
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
    {
        return false;
    }
    // Bound unbraced command chains as well as brace nesting. Macro definitions
    // are unnecessary for assistant math and can amplify a small source input.
    let mut commands = 0;
    for suffix in source.split('\\').skip(1) {
        commands += 1;
        let name: String = suffix
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect();
        if commands > 128
            || matches!(
                name.as_str(),
                "def"
                    | "gdef"
                    | "edef"
                    | "xdef"
                    | "let"
                    | "futurelet"
                    | "newcommand"
                    | "renewcommand"
                    | "providecommand"
                    | "expandafter"
                    | "csname"
            )
        {
            return false;
        }
    }
    let mut depth = 0usize;
    let mut escaped = false;
    for ch in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '{' => {
                depth += 1;
                if depth > 32 {
                    return false;
                }
            }
            '}' => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
            }
            _ => {}
        }
    }
    depth == 0
}

/// Render a transparent PNG at native resolution, subject to source and raster budgets.
pub fn render(source: &str, font_size: u16, foreground: (u8, u8, u8)) -> Option<Vec<u8>> {
    render_with_options(source, font_size, foreground, 1, None)
}

/// Render with two samples per terminal pixel. Fit wide equations by changing the
/// vector layout's font size before rasterization, avoiding a bitmap resize pass.
/// `max_width` is in terminal pixels, not supersampled raster pixels.
pub fn render_for_terminal(
    source: &str,
    font_size: u16,
    foreground: (u8, u8, u8),
    max_width: u32,
) -> Option<Vec<u8>> {
    render_with_options(
        source,
        font_size,
        foreground,
        TERMINAL_PIXEL_SCALE,
        Some(max_width),
    )
}

fn render_with_options(
    source: &str,
    font_size: u16,
    foreground: (u8, u8, u8),
    pixel_scale: u32,
    max_width: Option<u32>,
) -> Option<Vec<u8>> {
    if !bounded_source(source) || !(8..=64).contains(&font_size) {
        return None;
    }
    // Source is processed by a math parser, never executed by a TeX interpreter.
    std::panic::catch_unwind(|| {
        let nodes = ratex_parser::parse(source).ok()?;
        let (r, g, b) = foreground;
        let options = LayoutOptions::default()
            .with_style(MathStyle::Display)
            .with_color(Color::rgb(
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
            ));
        let tree = layout(&nodes, &options);
        validate_layout(&tree).ok()?;
        let display = to_display_list(&tree);
        let requested_font_size = f64::from(font_size);
        let mut font_size = requested_font_size;
        if let Some(max_width) = max_width {
            if max_width <= 8 {
                return None;
            }
            if display.width * font_size + 8.0 > f64::from(max_width) {
                // Leave a pixel of slack for the rasterizer's f32 rounding.
                font_size = (f64::from(max_width) - 9.0) / display.width;
                if font_size < requested_font_size / 2.0 {
                    return None;
                }
            }
        }
        let scale = f64::from(pixel_scale);
        let width = (display.width * font_size + 8.0) * scale;
        let height = ((display.height + display.depth) * font_size + 8.0) * scale;
        if !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
            || width > 4096.0
            || height > 1024.0
            || width.ceil() * height.ceil() > MAX_PIXELS
        {
            return None;
        }
        render_to_png(
            &display,
            &RenderOptions {
                font_size: font_size as f32,
                padding: 4.0,
                device_pixel_ratio: pixel_scale as f32,
                background_color: Color::new(0.0, 0.0, 0.0, 0.0),
                ..RenderOptions::default()
            },
        )
        .ok()
    })
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_same_line_and_multiline_blocks_with_unicode_offsets() {
        let input = "α before $$\\frac{1}{2}$$ after\n\n$$\nx^2 + y^2\n$$";
        let found = formulas(input);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].source, r"\frac{1}{2}");
        for formula in found {
            assert!(input[formula.range].starts_with("$$"));
        }
    }

    #[test]
    fn finds_bracket_blocks_and_mixed_delimiters_without_changing_source() {
        let input = "α before \\[\\frac{1}{2}\\] after\n\n$$x^2$$\n\n\\[\n\\int_0^1 x\\,dx\n\\]";
        let found = formulas(input);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].source, r"\frac{1}{2}");
        assert_eq!(&input[found[0].range.clone()], r"\[\frac{1}{2}\]");
        assert_eq!(found[1].source, "x^2");
        assert_eq!(found[2].source.trim(), r"\int_0^1 x\,dx");
        assert!(formulas(r"\[x$$").is_empty());
        assert!(formulas(r"$$x\]").is_empty());
        assert!(formulas(r"\[unclosed").is_empty());
        assert!(formulas(r"\[ \]").is_empty());
    }

    #[test]
    fn bracket_blocks_obey_markdown_and_escape_exclusions() {
        for input in [
            r"`\[x\]`",
            "```tex\n\\[x\\]\n```",
            r"- \[x\]",
            r"> \[x\]",
            r"[a \[x\]](https://example.com)",
            r"**\[x\]**",
            r"\\[x\\]",
            r"\[x\\]",
        ] {
            assert!(formulas(input).is_empty(), "{input:?}");
        }
        assert_eq!(formulas(r"\\\[x\]").len(), 1);
    }

    #[test]
    fn ignores_code_links_html_escapes_and_containers() {
        for input in [
            "`$$x$$`",
            "```tex\n$$x$$\n```",
            "    $$x$$",
            "[a $$x$$](https://example.com)",
            "<div>\n$$x$$\n</div>",
            r"\$$x\$$",
            "$$$x$$$",
            "- $$x$$",
            "> $$x$$",
            "price $5 and $10",
            "echo $$",
            "$$unclosed",
            "$$ $$",
            "**$$x$$**",
            "*$$x$$*",
            "~~$$x$$~~",
            "# $$x$$",
        ] {
            assert!(formulas(input).is_empty(), "{input:?}");
        }
    }

    #[test]
    fn bellman_subscripts_and_optimality_stars_are_math_not_markdown() {
        for source in [
            r"V^\pi(s)
=
\mathbb{E}_\pi\left[
R_{t+1}+\gamma V^\pi(S_{t+1})
\mid S_t=s
\right]",
            r"V^\pi(s)=\sum_a \pi(a\mid s)\sum_{s',r}p(s',r\mid s,a)\left[r+\gamma V^\pi(s')\right]",
            r"V^*(s)=\max_a\mathbb{E}\left[R_{t+1}+\gamma V^*(S_{t+1})\mid S_t=s,A_t=a\right]",
            r"Q^*(s,a)=\mathbb{E}\left[R_{t+1}+\gamma\max_{a'}Q^*(S_{t+1},a')\mid s,a\right]",
        ] {
            for (open, close) in [(r"\[", r"\]"), ("$$", "$$")] {
                let input =
                    format!("An equation:\n\n{open}\n{source}\n{close}\n\nwhere:\n\n- state\n");
                let found = formulas(&input);
                assert_eq!(found.len(), 1, "{input}");
                assert_eq!(found[0].source.trim(), source);
                assert!(render_for_terminal(source, 42, (0, 0, 0), 1300).is_some());
            }
        }
    }

    #[test]
    fn limits_source_nesting_and_formula_count() {
        assert!(!bounded_source(&"x".repeat(MAX_SOURCE_BYTES + 1)));
        assert!(!bounded_source(&format!(
            "{}x{}",
            "{".repeat(33),
            "}".repeat(33)
        )));
        assert!(!bounded_source("x\x1b[31m"));
        assert!(!bounded_source("{x"));
        assert!(bounded_source(r"\frac{a}{b}"));
        assert!(!bounded_source(r"\def\a{\a}\a"));
        assert!(!bounded_source(&r"\sqrt".repeat(129)));
        assert_eq!(formulas(&"$$x$$\n\n".repeat(40)).len(), MAX_FORMULAS);
    }

    #[test]
    fn scanner_offsets_stay_valid_for_mixed_markdown_and_unicode() {
        let alphabet = [
            "α", "$", "$", "\\", "`", "\n", " ", "x", "{", "}", "[", "]", "*", "界",
        ];
        let mut state = 0x5eed_u64;
        for _ in 0..500 {
            let mut input = String::new();
            for _ in 0..100 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                input.push_str(alphabet[(state >> 32) as usize % alphabet.len()]);
            }
            let mut previous_end = 0;
            for formula in formulas(&input) {
                assert!(formula.range.start >= previous_end);
                assert!(formula.range.end <= input.len());
                assert_eq!(
                    &input[formula.range.start + 2..formula.range.end - 2],
                    formula.source
                );
                assert!(!formula.source.trim().is_empty());
                previous_end = formula.range.end;
            }
        }
    }

    #[test]
    fn renders_fractions_integrals_and_matrices_to_png() {
        for source in [
            r"\frac{1}{1+x^2}",
            r"\int_{-\infty}^{\infty} e^{-x^2}\,dx=\sqrt{\pi}",
            r"\begin{pmatrix}a & b \\ c & d\end{pmatrix}",
        ] {
            let png = render(source, 24, (220, 220, 220)).expect(source);
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        }
    }

    fn png_dimensions(png: &[u8]) -> (u32, u32) {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        (
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
        )
    }

    #[test]
    fn terminal_rendering_supersamples_without_enlarging_the_equation() {
        let source = r"\frac{1}{1+x^2}";
        let normal = render(source, 24, (0, 0, 0)).unwrap();
        let high_resolution = render_for_terminal(source, 24, (0, 0, 0), 1024).unwrap();
        let (width, height) = png_dimensions(&normal);
        let (scaled_width, scaled_height) = png_dimensions(&high_resolution);
        assert!((width * 2 - 1..=width * 2).contains(&scaled_width));
        assert!((height * 2 - 1..=height * 2).contains(&scaled_height));
    }

    #[test]
    fn terminal_rendering_fits_at_the_vector_stage_and_keeps_limits() {
        let source = r"a+b+c+d+e+f+g+h+i+j+k+l";
        let natural = render_for_terminal(source, 24, (0, 0, 0), 2048).unwrap();
        let natural_width = png_dimensions(&natural).0 / TERMINAL_PIXEL_SCALE;
        let max_width = natural_width * 3 / 4;
        let fitted = render_for_terminal(source, 24, (0, 0, 0), max_width).unwrap();
        assert!(png_dimensions(&fitted).0 <= max_width * TERMINAL_PIXEL_SCALE);
        assert!(png_dimensions(&fitted).1 < png_dimensions(&natural).1);
        assert!(render_for_terminal(source, 24, (0, 0, 0), natural_width / 3).is_none());
        assert!(render_for_terminal(source, 24, (0, 0, 0), 0).is_none());
        assert!(render_for_terminal(r"\rule{999999em}{999999em}", 24, (0, 0, 0), 1024).is_none());
    }

    #[test]
    fn invalid_or_excessive_math_falls_back() {
        assert!(render(r"\thiscommanddoesnotexist{x}", 24, (255, 255, 255)).is_none());
        assert!(render(r"\input{/etc/passwd}", 24, (255, 255, 255)).is_none());
        assert!(render(r"\rule{999999em}{999999em}", 24, (255, 255, 255)).is_none());
    }
}
