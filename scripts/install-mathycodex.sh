#!/bin/sh
# Build optimized native binaries and install a separate mathycodex command.
set -eu

case "$(uname -s)" in
    Darwin) ;;
    *) echo "This release installer currently supports macOS only." >&2; exit 1 ;;
esac

build=true
case "${1:-}" in
    "") ;;
    --no-build) build=false ;;
    -h|--help)
        echo "Usage: $0 [--no-build]"
        echo "Build and install the release CLI and runtime helpers. --no-build uses existing release binaries."
        exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
esac
if [ "$#" -gt 1 ]; then echo "Expected at most one option." >&2; exit 2; fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
workspace="$repo_root/codex-rs"
install_dir=${MATHYCODEX_INSTALL_DIR:-"$HOME/.local/lib/mathycodex"}
bin_dir=${MATHYCODEX_BIN_DIR:-"$HOME/.local/bin"}
target_dir=${MATHYCODEX_TARGET_DIR:-${CARGO_TARGET_DIR:-"$workspace/target"}}
case "$install_dir" in /*) ;; *) echo "MATHYCODEX_INSTALL_DIR must be absolute." >&2; exit 2 ;; esac
case "$bin_dir" in /*) ;; *) echo "MATHYCODEX_BIN_DIR must be absolute." >&2; exit 2 ;; esac
case "$target_dir" in /*) ;; *) target_dir="$workspace/$target_dir" ;; esac
command_path="$bin_dir/mathycodex"

# Never overwrite an unrelated executable or symlink, including the official codex.
if [ -e "$command_path" ] || [ -L "$command_path" ]; then
    if [ ! -L "$command_path" ] || [ "$(readlink "$command_path")" != "$install_dir/mathycodex" ]; then
        echo "Refusing to replace an unrelated path: $command_path" >&2
        exit 1
    fi
fi

if [ "$build" = true ]; then
    command -v cargo >/dev/null 2>&1 || { echo "Install Rust with rustup first." >&2; exit 1; }
    # Avoid blocking Rustlings and other projects on Cargo's package-cache lock.
    export CARGO_HOME=${CARGO_HOME:-"${XDG_CACHE_HOME:-$HOME/.cache}/mathycodex-cargo"}
    export CARGO_NET_GIT_FETCH_WITH_CLI=${CARGO_NET_GIT_FETCH_WITH_CLI:-true}
    export RUSTY_V8_MIRROR=${RUSTY_V8_MIRROR:-'https://github.com/openai/codex/releases/download/rusty-v8-{tag}/{file}'}
    (
        cd "$workspace"
        cargo build --release --locked --jobs "${MATHYCODEX_BUILD_JOBS:-4}" \
            --target-dir "$target_dir" \
            --bin codex --bin codex-code-mode-host --bin codex-exec-server
    )
fi

for binary in codex codex-code-mode-host codex-exec-server; do
    if [ ! -x "$target_dir/release/$binary" ]; then
        echo "Missing release executable: $target_dir/release/$binary" >&2
        echo "Run the installer without --no-build." >&2
        exit 1
    fi
done

mkdir -p "$install_dir" "$bin_dir"
stage=$(mktemp -d "$install_dir/.install.XXXXXX")
trap 'rm -rf "$stage"' EXIT
trap 'exit 1' HUP INT TERM
install -m 755 "$target_dir/release/codex" "$stage/mathycodex"
install -m 755 "$target_dir/release/codex-code-mode-host" "$stage/codex-code-mode-host"
install -m 755 "$target_dir/release/codex-exec-server" "$stage/codex-exec-server"
for binary in mathycodex codex-code-mode-host codex-exec-server; do
    mv -f "$stage/$binary" "$install_dir/$binary"
done
if [ ! -L "$command_path" ]; then ln -s "$install_dir/mathycodex" "$command_path"; fi
printf 'Installed release command: %s\n' "$command_path"
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) printf 'Add %s to PATH in your shell startup file.\n' "$bin_dir" ;;
esac
printf '%s\n' 'Open Kitty and run: mathycodex'
