use crate::legacy_core::config::Config;
use codex_features::Feature;

pub(crate) const TERMINAL_VISUALIZATION_INSTRUCTIONS: &str = "\
- This surface is a terminal. When the formatting rules require a visual, include one in the final answer using compact ASCII diagrams, trees, timelines, or tables.
- Use tables for exact mappings or comparisons rather than collapsing known mappings into prose.
- Use trees for hierarchy or one-to-many relationships, and diagrams or timelines for sequence, change, or state transferred between records across event order.
- Use only ASCII characters in visuals.";

/// Advertise math support independently of the optional ASCII visualization policy.
pub(crate) const TERMINAL_MATH_INSTRUCTIONS: &str = r"This MathyCodex terminal can typeset LaTeX mathematics. When explaining a mathematical topic, use LaTeX equations naturally wherever they help, without requiring the user to ask for LaTeX. Put display equations in standalone \[ ... \] or $$ ... $$ blocks, outside code fences, Markdown lists, and blockquotes. Use \( ... \) for short inline math. Keep the surrounding explanation in ordinary prose. Mathematical notation is supported in addition to any ASCII diagrams required by other terminal formatting instructions.";

pub(crate) fn with_terminal_visualization_instructions(
    config: &Config,
    control_instructions: Option<String>,
) -> Option<String> {
    let visualizations = config
        .features
        .enabled(Feature::TerminalVisualizationInstructions);
    let math =
        config.tui_rendering.math && config.tui_fullscreen_transcript && graphical_math_available();
    if !visualizations && !math {
        return control_instructions;
    }
    let mut instructions = control_instructions
        .or_else(|| config.developer_instructions.clone())
        .unwrap_or_default();
    for note in [
        visualizations.then_some(TERMINAL_VISUALIZATION_INSTRUCTIONS),
        math.then_some(TERMINAL_MATH_INSTRUCTIONS),
    ]
    .into_iter()
    .flatten()
    {
        if instructions.contains(note) {
            continue;
        }
        if !instructions.trim().is_empty() {
            instructions.push_str("\n\n");
        }
        instructions.push_str(note);
    }
    Some(instructions)
}

#[cfg(not(test))]
fn graphical_math_available() -> bool {
    std::env::var_os("MATHYCODEX_DISABLE_LATEX").is_none()
        && crate::pets::detect_pet_image_support().protocol()
            == Some(crate::pets::ImageProtocol::Kitty)
}

// Match the renderer preference tests: capability overrides are thread-local so
// request-payload tests are deterministic even when run inside a real Kitty window.
#[cfg(test)]
thread_local! {
    static GRAPHICAL_MATH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn graphical_math_available() -> bool {
    GRAPHICAL_MATH.get()
}

#[cfg(test)]
pub(crate) fn with_test_graphical_math<T>(enabled: bool, run: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            GRAPHICAL_MATH.set(self.0);
        }
    }
    let _restore = Restore(GRAPHICAL_MATH.replace(enabled));
    run()
}
