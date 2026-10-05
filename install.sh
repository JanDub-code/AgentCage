#!/usr/bin/env sh
# AgentCage installer. Stdin bootstrap never builds the caller's directory.
set -eu

prefix="${PREFIX:-$HOME/.local}"
bindir="${BINDIR:-}"
build=1
temp_dir=
install_temp=
install_pid=

usage() {
    cat <<'EOF'
Usage: ./install.sh [--prefix DIR] [--bin-dir DIR] [--no-build]

Installs AgentCage (ac) on Linux x86_64.

Options:
  --prefix DIR   install under DIR/bin (default: $HOME/.local)
  --bin-dir DIR  install directly into DIR
  --no-build     install a binary package or an existing source release build

Source builds use --locked, the Rust host target, and CARGO_TARGET_DIR if set
(otherwise ./target). Cargo's configured target and target-dir are overridden.
Stdin/standalone bootstrap clones AgentCage; --no-build needs a local artifact.
EOF
}

fail() { echo "error: $*" >&2; exit 1; }
cleanup() {
    if [ -n "$install_pid" ]; then
        kill -TERM "$install_pid" 2>/dev/null || :
        kill -CONT "$install_pid" 2>/dev/null || :
        wait "$install_pid" 2>/dev/null || :
    fi
    [ -z "$install_temp" ] || rm -f -- "$install_temp"
    [ -z "$temp_dir" ] || rm -rf -- "$temp_dir"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix|--bin-dir)
            option=$1
            [ "$#" -ge 2 ] && [ -n "$2" ] || { echo "error: $option needs a value" >&2; exit 2; }
            case "$2" in --*) echo "error: $option needs a directory value" >&2; exit 2 ;; esac
            case "$option" in --prefix) prefix=$2 ;; --bin-dir) bindir=$2 ;; esac
            shift 2
            ;;
        --no-build) build=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "error: unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || fail "supported installer platform: Linux x86_64"
target_triple=x86_64-unknown-linux-gnu

# Keep relative destination/target paths relative to the original caller.
caller_dir=$(pwd -P)
[ -n "$bindir" ] || bindir="$prefix/bin"
case "$bindir" in /*) ;; *) bindir="$caller_dir/$bindir" ;; esac

source_identity() {
    [ -f "$1/Cargo.toml" ] && [ -f "$1/Cargo.lock" ] &&
    [ -f "$1/src/main.rs" ] && [ -f "$1/src/lib.rs" ] &&
    [ -f "$1/src/agent.rs" ] && [ -f "$1/src/opencode_account.py" ] && [ -f "$1/src/login_sync.py" ] &&
    [ -f "$1/Dockerfile" ] &&
    grep -Eq '^name[[:space:]]*=[[:space:]]*"agentcage"[[:space:]]*$' "$1/Cargo.toml" &&
    grep -Eq '^name[[:space:]]*=[[:space:]]*"ac"[[:space:]]*$' "$1/Cargo.toml"
}

script_dir=
case "$0" in
    sh|bash|dash|ash|ksh|zsh|-*) ;; # sh -s / curl | sh
    *)
        if [ -f "$0" ]; then
            script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
        fi
        ;;
esac

package=0
if [ -n "$script_dir" ] && [ -f "$script_dir/RELEASE" ]; then
    package=1
elif [ -z "$script_dir" ] || ! source_identity "$script_dir"; then
    [ "$build" -eq 1 ] || fail "--no-build needs an extracted AgentCage binary package or an existing local source release build; bootstrap cannot supply one"
    command -v git >/dev/null 2>&1 || fail "git is required for source bootstrap"
    temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/agentcage-install.XXXXXXXX")
    echo "Cloning AgentCage source..."
    git clone --depth 1 -- https://github.com/JanDub-code/AgentCage.git "$temp_dir/source" </dev/null
    script_dir="$temp_dir/source"
    source_identity "$script_dir" || fail "downloaded source is not an AgentCage checkout"
fi

if [ "$package" -eq 1 ]; then
    [ "$build" -eq 0 ] || fail "binary package: use ./install.sh --no-build"
    [ "$(sed -n 's/^name=//p' "$script_dir/RELEASE")" = agentcage ] || fail "invalid AgentCage release metadata"
    version=$(sed -n 's/^version=//p' "$script_dir/RELEASE")
    package_target=$(sed -n 's/^target=//p' "$script_dir/RELEASE")
    [ "$package_target" = "$target_triple" ] || fail "package target $package_target does not match $target_triple"
    binary="$script_dir/bin/ac"
    expected_sha=$(sed -n 's/^binary_sha256=//p' "$script_dir/RELEASE")
    [ "${#expected_sha}" -eq 64 ] || fail "invalid release binary checksum"
    case "$expected_sha" in *[!0-9a-f]*) fail "invalid release binary checksum" ;; esac
    [ -x "$binary" ] || fail "release binary not found: $binary"
    actual_sha=$(sha256sum "$binary")
    actual_sha=${actual_sha%% *}
    [ "$actual_sha" = "$expected_sha" ] || fail "release binary checksum mismatch: $binary"
else
    version=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' "$script_dir/Cargo.toml" | head -n 1)
    target_dir="${CARGO_TARGET_DIR:-$script_dir/target}"
    case "$target_dir" in /*) ;; *) target_dir="$caller_dir/$target_dir" ;; esac
    binary="$target_dir/$target_triple/release/ac"
    if [ "$build" -eq 1 ]; then
        command -v cargo >/dev/null 2>&1 || fail "cargo not found; install Rust or use a binary release with --no-build"
        command -v rustc >/dev/null 2>&1 || fail "rustc not found"
        rust_host=$(rustc -vV | sed -n 's/^host: //p')
        [ "$rust_host" = "$target_triple" ] || fail "Rust host $rust_host does not match supported target $target_triple"
        (cd "$script_dir" && cargo build --locked --release --bin ac --target "$target_triple" --target-dir "$target_dir" </dev/null)
    fi
    [ -x "$binary" ] || fail "release binary not found: $binary; run ./install.sh or use an extracted binary package with --no-build"
fi

case "$version" in ''|*[!0-9A-Za-z.+-]*) fail "invalid AgentCage version in release metadata" ;; esac
printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$' || fail "invalid AgentCage version in release metadata"
mkdir -p -- "$bindir"
# Finish the copy on the destination filesystem before replacing a working ac.
install_temp=$(mktemp "$bindir/.agentcage-install.XXXXXXXX")
install -m 0755 -- "$binary" "$install_temp" &
install_pid=$!
if wait "$install_pid"; then
    install_pid=
else
    install_status=$?
    install_pid=
    exit "$install_status"
fi
mv -fT -- "$install_temp" "$bindir/ac"
install_temp=
echo "installed AgentCage $version ($target_triple): $bindir/ac"
case ":$PATH:" in
    *":$bindir:"*) ;;
    *) echo "warning: $bindir is not in PATH; add this directory to your shell profile" ;;
esac
echo "Next: ac"
