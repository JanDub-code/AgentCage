#!/usr/bin/env sh
set -eu

prefix="${PREFIX:-$HOME/.local}"
bindir="${BINDIR:-}"
build=1

usage() {
    cat <<'EOF'
Usage: ./install.sh [--prefix DIR] [--bin-dir DIR] [--no-build]

Installs the ac binary.

Options:
  --prefix DIR   install under DIR/bin (default: $HOME/.local)
  --bin-dir DIR  install directly into DIR
  --no-build     do not run cargo build; install an existing release binary
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            [ "$#" -ge 2 ] || { echo "error: --prefix needs a value" >&2; exit 2; }
            prefix="$2"
            shift 2
            ;;
        --bin-dir)
            [ "$#" -ge 2 ] || { echo "error: --bin-dir needs a value" >&2; exit 2; }
            bindir="$2"
            shift 2
            ;;
        --no-build)
            build=0
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$script_dir"

if [ -z "$bindir" ]; then
    bindir="$prefix/bin"
fi

if [ "$build" -eq 1 ]; then
    command -v cargo >/dev/null 2>&1 || {
        echo "error: cargo not found; install Rust or use a binary release with --no-build" >&2
        exit 1
    }
    cargo build --release
fi

binary="$script_dir/target/release/ac"
if [ ! -x "$binary" ]; then
    echo "error: release binary not found: $binary" >&2
    echo "hint: run ./install.sh, or extract a binary release package and run ./install.sh --no-build" >&2
    exit 1
fi

mkdir -p "$bindir"
install -m 0755 "$binary" "$bindir/ac"

echo "installed: $bindir/ac"
case ":$PATH:" in
    *":$bindir:"*) ;;
    *)
        echo "warning: $bindir is not in PATH"
        echo "add this to your shell profile:"
        echo "  export PATH=\"$bindir:\$PATH\""
        ;;
esac

echo
echo "Next:"
echo "  ac"
