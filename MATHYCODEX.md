# MathyCodex

This fork adds Kitty graphics rendering for completed `$$…$$` and `\[…\]` math in ordinary assistant replies.

## Install a release build (macOS)

Install Kitty for rendered equations, and Git, Python 3, and build tools for compiling:

```sh
brew install --cask kitty
brew install git python ripgrep
xcode-select --install
```

Skip the Xcode command if its Command Line Tools are already installed. Install Rust with
[rustup](https://rustup.rs/) if needed. The workspace's `rust-toolchain.toml` selects its pinned
toolchain when building. Rustup can install that toolchain on first use.

### Build the optimized binaries

Clone this fork and build the CLI with its matching runtime helpers:

```sh
git clone https://github.com/ajmcm/mathycodex.git
cd mathycodex/codex-rs
export CARGO_HOME="$HOME/.cache/mathycodex-cargo"
export CARGO_NET_GIT_FETCH_WITH_CLI=true
export RUSTY_V8_MIRROR='https://github.com/openai/codex/releases/download/rusty-v8-{tag}/{file}'
cargo build --release --locked --jobs 4 \
  --bin codex --bin codex-code-mode-host --bin codex-exec-server
```

`--release` selects the optimized build. The first build downloads upstream Codex dependencies,
including V8 submodules, and can take a while. The separate Cargo cache avoids blocking Rustlings
and other Rust projects during those downloads. Using system Git can also avoid Cargo's
built-in Git transport errors. The V8 mirror selects OpenAI’s matching native runtime
archives; these are separate downloads from the Rust crates and Git submodules.

### Install the files

From the same `codex-rs` directory, copy the three release binaries:

```sh
mkdir -p "$HOME/.local/lib/mathycodex" "$HOME/.local/bin"
install -m 755 target/release/codex "$HOME/.local/lib/mathycodex/mathycodex"
install -m 755 target/release/codex-code-mode-host \
  target/release/codex-exec-server "$HOME/.local/lib/mathycodex/"
```

The first command creates the installation directories. The next commands copy the CLI and
its helpers there, naming the CLI `mathycodex`. They do not replace the upstream `codex` command.
Keep the helpers alongside the CLI so it can find its JavaScript runtime and execution server.

For the **first installation**, create the command symlink:

```sh
ln -s "$HOME/.local/lib/mathycodex/mathycodex" "$HOME/.local/bin/mathycodex"
```

This command refuses to replace an existing file. If the symlink already points to this
installation, leave it in place when updating.

If `~/.local/bin` is not already on your PATH, add it once to `~/.zshrc`:

```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

Open **Kitty**, then run:

```sh
mathycodex
```

Sign in when prompted, or use `mathycodex login`. This fork uses the existing Codex account
and configuration location. Kitty is required for image-based equations; Apple's built-in
Terminal can build and launch MathyCodex, but does not support the Kitty graphics protocol.

Math rendering is advertised to the model automatically in supported fullscreen sessions.
You can ask “Explain the Gaussian integral” without requesting LaTeX formatting. Both
`$$ ... $$` and `\[ ... \]` display blocks work, including multiline blocks. A block embedded
in a sentence is placed on its own display rows. Single-dollar and `\( ... \)` inline math
continue through Codex's text renderer.

Fullscreen transcript mode defaults to enabled. If your existing configuration disables it:

```sh
mathycodex -c tui.fullscreen_transcript=true
```

### Update

Pull updates, then repeat the Cargo build and file-copy commands above. The existing command
symlink keeps pointing to the updated installation.

### Optional convenience installer

If you prefer a single command, the repository also contains
[`scripts/install-mathycodex.sh`](scripts/install-mathycodex.sh). You can read it before running
it; the manual steps above are the primary installation method.

```sh
cd ..  # return from codex-rs to the repository root
less scripts/install-mathycodex.sh
./scripts/install-mathycodex.sh
```

It runs the release build and installs the same files and command symlink. To use release
artifacts that are already built, run `./scripts/install-mathycodex.sh --no-build`.
It refuses to replace an unrelated `mathycodex` command and does not edit your shell configuration.

The installer defaults to `~/.cache/mathycodex-cargo` and system Git. It honors explicitly set
`CARGO_HOME` and `CARGO_NET_GIT_FETCH_WITH_CLI`. Optional settings:

| Variable | Default | Purpose |
| --- | --- | --- |
| `MATHYCODEX_INSTALL_DIR` | `~/.local/lib/mathycodex` | Absolute installation directory |
| `MATHYCODEX_BIN_DIR` | `~/.local/bin` | Absolute directory for the command symlink |
| `MATHYCODEX_BUILD_JOBS` | `4` | Parallel build jobs |
| `MATHYCODEX_TARGET_DIR` | `codex-rs/target` | Cargo build directory; also honors `CARGO_TARGET_DIR` |

The release installer currently supports native macOS builds. For other platforms, use the
upstream packaging tools; Linux additionally needs its bundled sandbox helpers. Cross-compiling
with this installer is not supported.

### Uninstall

For the default installation:

```sh
rm ~/.local/bin/mathycodex
rm -r ~/.local/lib/mathycodex
```

Your Codex account/configuration and source checkout are retained.

## Behavior

- Only finalized assistant messages are rasterized; streaming remains text until completion.
- A serial background worker renders with RaTeX and embedded KaTeX fonts. No system TeX,
  browser, shell command, or external rendering service is invoked.
- The existing transcript image transport handles Kitty uploads, viewport clipping, clearing,
  and redraws. Each equation has its own placement.
- Original math source remains in the transcript's search/copy model and the original reply
  remains available for copying and export. Native terminal selection cannot extract image pixels
  as LaTeX; use Codex's copy actions.
- Equations render at 2× resolution and are padded to exact terminal-cell boundaries,
  preserving glyph proportions and giving Retina displays more detail. Wide equations are
  fitted before rasterization, without an intermediate bitmap downsampling pass.
- Rendering uses the detected terminal foreground and actual cell dimensions. Unknown dimensions,
  missing color information, unsupported terminals, very narrow viewports, invalid syntax, and
  oversized equations retain the ordinary text renderer.
- Math in code, links, HTML, tables, lists, blockquotes, headings, and emphasis retains ordinary text rendering.
  Spoken/artifact replies and interactive question titles also retain their existing presentation.
- Inputs are limited to 4 KiB per equation, 16 blocks per response, 32 levels of brace nesting, 128 control sequences,
  macro-definition exclusions, and bounded raster dimensions. The renderer request queue and raster cache are bounded.
- This is a KaTeX-style math subset, not a full LaTeX document interpreter. It adds third-party
  renderer/font dependencies; this change does not constitute a security audit of those libraries.

To disable graphical math for a run:

```sh
MATHYCODEX_DISABLE_LATEX=1 mathycodex
```

The existing `tui.rendering.math` preference also disables it. Native scrollback, raw mode,
non-Kitty protocols, and exports retain the upstream text behavior.

## Checks

```sh
CARGO_HOME="$HOME/.cache/mathycodex-cargo" cargo test --manifest-path mathycodex-renderer/Cargo.toml
cd codex-rs
CARGO_HOME="$HOME/.cache/mathycodex-cargo" cargo test -p codex-tui --lib kitty_math
CARGO_HOME="$HOME/.cache/mathycodex-cargo" cargo check -p codex-tui
```

The renderer tests cover Markdown exclusions, same-line/multiline delimiters, limits,
invalid math, and PNG output for fractions, integrals, and matrices. TUI tests cover retained
source, interleaved prose, narrow/wide reflow, spacing, and clipped image placements.

To create a standalone visual preview of the renderer:

```sh
CARGO_HOME="$HOME/.cache/mathycodex-cargo" cargo run --manifest-path mathycodex-renderer/Cargo.toml --example preview -- /tmp/mathycodex-preview.png
```
