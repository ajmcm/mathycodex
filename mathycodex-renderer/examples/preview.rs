//! cargo run --manifest-path mathycodex-renderer/Cargo.toml --example preview -- /tmp/mathycodex-preview.png
fn main() {
    let path = std::env::args().nth(1).expect("pass an output PNG path");
    let source = r"\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}";
    let png = mathycodex_renderer::render_for_terminal(source, 32, (220, 220, 220), 1024)
        .expect("example equation should render");
    std::fs::write(path, png).expect("write preview PNG");
}
