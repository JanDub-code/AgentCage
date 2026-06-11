#!/usr/bin/env sh
set -eu

REPO="JanDub-code/AgentCage"
prefix="${PREFIX:-$HOME/.local}"
bindir="${BINDIR:-}"
want_build=0
explicit_no_build=0

usage() {
    cat <<'EOF'
Usage: install.sh [--prefix DIR] [--bin-dir DIR] [--build]

Installs the ac binary.

Options:
  --prefix DIR   install under DIR/bin (default: $HOME/.local)
  --bin-dir DIR  install directly into DIR
  --build        build from source instead of downloading a pre-built binary
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
        --build)
            want_build=1
            shift
            ;;
        --no-build)
            # Legacy flag: skip building, only install existing binary.
            # When used from a tarball package that already has target/release/ac.
            want_build=0
            explicit_no_build=1
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

if [ -z "$bindir" ]; then
    bindir="$prefix/bin"
fi

# ----- Helper: detect architecture -----

detect_target() {
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)
            echo "error: unsupported architecture: $arch" >&2
            echo "hint: try --build to compile from source" >&2
            exit 1
            ;;
    esac

    os=$(uname -s)
    case "$os" in
        Linux)  target="${arch}-unknown-linux-gnu" ;;
        *)
            echo "error: unsupported OS: $os" >&2
            echo "hint: try --build to compile from source" >&2
            exit 1
            ;;
    esac

    echo "$target"
}

# ----- Helper: download binary from GitHub Releases -----

download_binary() {
    target=$(detect_target)

    # Find the latest release tag
    if command -v curl >/dev/null 2>&1; then
        latest=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | head -n1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    elif command -v wget >/dev/null 2>&1; then
        latest=$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | head -n1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    else
        echo "error: curl or wget required to download the binary" >&2
        return 1
    fi

    if [ -z "$latest" ]; then
        echo "error: could not determine latest release" >&2
        return 1
    fi

    version="${latest#v}"
    package="agentcage-${version}-${target}"
    tarball="${package}.tar.gz"
    url="https://github.com/${REPO}/releases/download/${latest}/${tarball}"

    echo "Downloading ${tarball}..."

    temp_dir=$(mktemp -d)
    trap 'rm -rf "$temp_dir"' EXIT

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$temp_dir/$tarball" "$url"
    else
        wget -q -O "$temp_dir/$tarball" "$url"
    fi

    tar -xzf "$temp_dir/$tarball" -C "$temp_dir"

    binary="$temp_dir/$package/target/release/ac"
    if [ ! -x "$binary" ]; then
        echo "error: binary not found in release archive" >&2
        return 1
    fi

    mkdir -p "$bindir"
    install -m 0755 "$binary" "$bindir/ac"

    # Also install Dockerfile and source files for `ac init` image builds
    share_dir="${prefix}/share/agentcage"
    mkdir -p "$share_dir"
    for f in Dockerfile Cargo.toml Cargo.lock LICENSE README.md SECURITY.md LOGIN_PERSISTENCE.md TUTORIAL.md; do
        if [ -f "$temp_dir/$package/$f" ]; then
            install -m 0644 "$temp_dir/$package/$f" "$share_dir/$f"
        fi
    done
    if [ -d "$temp_dir/$package/src" ]; then
        mkdir -p "$share_dir/src"
        for f in "$temp_dir/$package/src/"*.rs; do
            [ -f "$f" ] && install -m 0644 "$f" "$share_dir/src/"
        done
    fi

    return 0
}

# ----- Helper: build from source -----

do_build_from_source() {
    command -v cargo >/dev/null 2>&1 || {
        echo "error: cargo not found; install Rust (https://rustup.rs) or omit --build to download a binary" >&2
        exit 1
    }

    # Check for a C linker (cc / gcc / clang) — required by the libc crate
    if ! command -v cc >/dev/null 2>&1 && ! command -v gcc >/dev/null 2>&1; then
        echo "error: a C compiler (cc/gcc) is required to build from source" >&2
        echo "hint: on Debian/Ubuntu run: sudo apt install build-essential" >&2
        echo "hint: on Fedora/RHEL run: sudo dnf groupinstall 'Development Tools'" >&2
        exit 1
    fi

    # If we're running from the repo checkout, use it directly
    script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
    if [ -f "$script_dir/Cargo.toml" ]; then
        cd "$script_dir"
    else
        echo "Cloning repository..."
        temp_dir=$(mktemp -d)
        trap 'rm -rf "$temp_dir"' EXIT
        git clone "https://github.com/${REPO}.git" "$temp_dir"
        cd "$temp_dir"
    fi

    # Use a fresh target directory to avoid stale/corrupt build artifacts
    export CARGO_TARGET_DIR="${PWD}/target"
    cargo clean 2>/dev/null || true
    cargo build --release

    binary="target/release/ac"
    if [ ! -x "$binary" ]; then
        echo "error: release binary not found after build" >&2
        exit 1
    fi

    mkdir -p "$bindir"
    install -m 0755 "$binary" "$bindir/ac"
}

# ----- Helper: PATH setup -----

path_contains() {
    case ":${PATH:-}:" in
        *":$1:"*) return 0 ;;
        *) return 1 ;;
    esac
}

ensure_path() {
    bin_dir="$1"

    if path_contains "$bin_dir"; then
        return 0
    fi

    shell_name=$(basename "${SHELL:-sh}")
    profile="$HOME/.profile"

    case "$shell_name" in
        bash)
            profile="$HOME/.bashrc"
            ;;
        zsh)
            profile="$HOME/.zshrc"
            ;;
        fish)
            profile="$HOME/.config/fish/config.fish"
            mkdir -p "$(dirname "$profile")"
            ;;
    esac

    added_line=0
    if [ "$shell_name" = "fish" ]; then
        line="fish_add_path $bin_dir"
    else
        line='export PATH="$HOME/.local/bin:$PATH"'
    fi

    if ! grep -Fqs "$line" "$profile" 2>/dev/null; then
        printf '\n%s\n' "$line" >> "$profile"
        added_line=1
    fi

    echo
    echo "warning: $bin_dir is not in PATH for this shell"
    if [ "$added_line" -eq 1 ]; then
        echo "Added this line to $profile for future shells:"
        echo "  $line"
    else
        echo "$profile already has a PATH entry for $bin_dir."
    fi
    echo
    echo "This installer cannot change PATH in your current terminal session."
    echo "To use ac now, reload your shell profile or open a new shell:"
    echo "  source $profile"
    echo
    echo "Or run ac directly:"
    echo "  $bin_dir/ac"
}

os_release_value() {
    key="$1"
    if [ -r /etc/os-release ]; then
        sed -n "s/^${key}=//p" /etc/os-release | head -n 1 | sed 's/^"//; s/"$//'
    fi
}

podman_install_hint() {
    os_id=$(os_release_value ID)
    os_name=$(os_release_value NAME)

    case "$os_id" in
        fedora)
            echo "AgentCage was built first for Fedora, where Podman is usually already present."
            echo "If it is missing on this system, install it with:"
            echo
            echo "  sudo dnf install -y podman"
            ;;
        ubuntu|debian)
            echo "Install Podman first with:"
            echo
            echo "  sudo apt install -y podman"
            ;;
        arch)
            echo "Install Podman first with:"
            echo
            echo "  sudo pacman -S podman"
            ;;
        opensuse-tumbleweed|opensuse-leap)
            echo "Install Podman first with:"
            echo
            echo "  sudo zypper install -y podman"
            ;;
        *)
            if [ -n "$os_name" ]; then
                echo "Install rootless Podman on $os_name."
            else
                echo "Install rootless Podman."
            fi
            echo "See: https://podman.io"
            ;;
    esac
}

check_podman_runtime() {
    echo
    echo "Runtime check:"
    if command -v podman >/dev/null 2>&1; then
        echo "  podman: found ($(podman --version 2>/dev/null || echo podman))"
        rootless=$(podman info --format '{{.Host.Security.Rootless}}' 2>/dev/null || true)
        if [ "$rootless" = "true" ]; then
            echo "  rootless: yes"
            return 0
        fi
        echo "  rootless: no"
        echo
        echo "AgentCage requires rootless Podman."
        echo "Check:"
        echo
        echo "  podman info --format '{{.Host.Security.Rootless}}'"
        echo
        echo "That command must print:"
        echo
        echo "  true"
        return 0
    fi

    echo "  podman: missing"
    echo
    podman_install_hint
    echo
    echo "After installing Podman, verify rootless mode:"
    echo
    echo "  podman info --format '{{.Host.Security.Rootless}}'"
    echo
    echo "That command must print:"
    echo
    echo "  true"
}

# ----- Main -----

# If we're being run from a tarball that already contains the binary,
# install it directly (for backwards compatibility with package.sh tarballs)
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prebuilt="$script_dir/target/release/ac"
repo_checkout=0
if [ -f "$script_dir/Cargo.toml" ]; then
    repo_checkout=1
fi

if [ -x "$prebuilt" ] && { [ "$explicit_no_build" -eq 1 ] || [ "$repo_checkout" -eq 0 ]; }; then
    mkdir -p "$bindir"
    install -m 0755 "$prebuilt" "$bindir/ac"
elif [ "$want_build" -eq 1 ]; then
    do_build_from_source
elif [ "$repo_checkout" -eq 1 ]; then
    do_build_from_source
else
    # Default: download pre-built binary from GitHub Releases
    if ! download_binary; then
        echo ""
        echo "Pre-built binary download failed. Falling back to building from source..."
        echo ""
        want_build=1
        do_build_from_source
    fi
fi

echo "installed: $bindir/ac"
if path_contains "$bindir"; then
    echo
    echo "Next:"
    echo "  ac"
elif [ "$bindir" = "$HOME/.local/bin" ]; then
    ensure_path "$bindir"
else
    echo
    echo "warning: $bindir is not in PATH"
    echo "This installer only auto-updates shell profiles for $HOME/.local/bin."
    echo "Add it to your shell configuration if needed."
    echo
    echo "Run ac directly:"
    echo "  $bindir/ac"
fi

check_podman_runtime
