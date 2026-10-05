#!/usr/bin/env sh
set -eu

fail() { echo "error: $*" >&2; exit 1; }
usage() {
    echo "Usage: ./scripts/package.sh [--target x86_64-unknown-linux-gnu]"
    echo "Build a locked Linux x86_64 release, archive, and SHA-256 checksum."
}

requested_target=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --target)
            [ "$#" -ge 2 ] && [ -n "$2" ] || fail "--target needs a value"
            requested_target=$2; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) fail "unknown option: $1" ;;
    esac
done

caller_dir=$(pwd -P)
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd -P)
cd "$repo_dir"

command -v cargo >/dev/null 2>&1 || fail "cargo not found"
command -v rustc >/dev/null 2>&1 || fail "rustc not found"
target_triple=$(rustc -vV | sed -n 's/^host: //p')
[ "$target_triple" = x86_64-unknown-linux-gnu ] || fail "supported release target: x86_64-unknown-linux-gnu (Rust host: $target_triple)"
[ -z "$requested_target" ] || [ "$requested_target" = "$target_triple" ] || fail "cross-target packaging is not supported; requested $requested_target, Rust host $target_triple"

version=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"[[:space:]]*$/\1/p' Cargo.toml | head -n 1)
case "$version" in ''|*[!0-9A-Za-z.+-]*) fail "invalid package version in Cargo.toml" ;; esac
printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$' || fail "invalid package version in Cargo.toml"
package="agentcage-$version-$target_triple"
target_dir="${CARGO_TARGET_DIR:-$repo_dir/target}"
case "$target_dir" in /*) ;; *) target_dir="$caller_dir/$target_dir" ;; esac

cargo build --locked --release --bin ac --target "$target_triple" --target-dir "$target_dir" </dev/null
binary="$target_dir/$target_triple/release/ac"
[ -x "$binary" ] || fail "release binary not found for $target_triple: $binary"

mkdir -p dist
temp_dir=$(mktemp -d "$repo_dir/dist/.package.XXXXXXXX")
publish_lock=
cleanup() {
    [ -z "$publish_lock" ] || rmdir -- "$publish_lock"
    rm -rf -- "$temp_dir"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
staging="$temp_dir/$package"
mkdir -p "$staging/bin"
install -m 0755 "$binary" "$staging/bin/ac"
install -m 0755 install.sh "$staging/install.sh"
for file in Dockerfile README.md SECURITY.md LOGIN_PERSISTENCE.md TUTORIAL.md LICENSE; do
    install -m 0644 "$file" "$staging/$file"
done
binary_sha=$(sha256sum "$staging/bin/ac"); binary_sha=${binary_sha%% *}
image_sha=$(sha256sum Dockerfile); image_sha=${image_sha%% *}
source_revision=unknown
source_state=unknown
git_root=$(git rev-parse --show-toplevel 2>/dev/null || true)
if [ "$git_root" = "$repo_dir" ] && source_revision=$(git rev-parse --verify HEAD 2>/dev/null); then
    if git_status=$(git status --porcelain 2>/dev/null); then
        source_state=clean
        [ -z "$git_status" ] || source_state=modified
    fi
else
    source_revision=unknown
fi
cat > "$staging/RELEASE" <<EOF
name=agentcage
version=$version
target=$target_triple
binary_sha256=$binary_sha
image_inputs_sha256=$image_sha
source_revision=$source_revision
source_state=$source_state
EOF

tarball="$package.tar.gz"
tar -C "$temp_dir" -czf "$temp_dir/$tarball" "$package"
(cd "$temp_dir" && sha256sum "$tarball" > "$tarball.sha256")
lock_dir="$repo_dir/dist/.$package.publish-lock"
mkdir -- "$lock_dir" || fail "package publication is busy: $lock_dir"
publish_lock="$lock_dir"
mv -- "$temp_dir/$tarball" "$temp_dir/$tarball.sha256" dist/
rmdir -- "$publish_lock"
publish_lock=
echo "dist/$tarball"
echo "dist/$tarball.sha256"
