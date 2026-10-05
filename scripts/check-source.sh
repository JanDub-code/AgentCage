#!/usr/bin/env sh
# Fast source/release gate. Build caches and extraction fixtures live outside checkout.
set -eu
repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
work=$(mktemp -d "${TMPDIR:-/tmp}/agentcage-check.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
cd "$repo_dir"
target_dir="${CARGO_TARGET_DIR:-$work/target}"
case "$target_dir" in /*) ;; *) target_dir="$repo_dir/$target_dir" ;; esac
cargo test --locked --target-dir "$target_dir"
cargo build --locked --target-dir "$target_dir"
binary="$target_dir/debug/ac"
python3 tests/test_runtime.py --binary "$binary"
python3 tests/test_login_wrapper.py --binary "$binary"
python3 tests/test_shells.py --binary "$binary"
python3 tests/test_distribution.py --binary "$binary"
python3 tests/test_opencode_account.py
python3 tests/test_login_sync.py
python3 tests/test_web_commands.py

# A genuine Git archive checks Linux release inputs, tracked file modes and LF.
# In an edited checkout this intentionally tests committed HEAD separately.
# Reviewers may name a frozen Git tree made with a temporary index; no commit
# is needed. CI always defaults to its exact checked-out HEAD.
archive_ref="${AC_ARCHIVE_REF:-HEAD}"
git rev-parse --verify "$archive_ref^{tree}" >/dev/null
git archive --format=tar "$archive_ref" -o "$work/source.tar"
mkdir "$work/source"
tar -xf "$work/source.tar" -C "$work/source"
python3 "$repo_dir/tests/test_distribution.py" --source "$work/source" --binary "$binary"
(
    cd "$work/source"
    ./install.sh --help
    CARGO_TARGET_DIR="$target_dir" ./scripts/package.sh
    cd dist
    sha256sum -c ./*.tar.gz.sha256
    archive=$(find . -maxdepth 1 -name '*.tar.gz' -print)
    [ "$(printf '%s\n' "$archive" | wc -l)" -eq 1 ]
    tar -xzf "$archive"
    package=${archive%.tar.gz}
    cd "$package"
    ./install.sh --no-build --bin-dir "$work/installed"
    "$work/installed/ac" --help
)
echo "PASS: source tests and exact Git archive ($archive_ref) package/install"
