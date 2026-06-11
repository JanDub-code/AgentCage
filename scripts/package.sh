#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)

cd "$repo_dir"

name="agentcage"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
target_triple=$(rustc -vV | sed -n 's/^host: //p')
package="$name-$version-$target_triple"
staging="dist/$package"

cargo build --release

rm -rf "$staging"
mkdir -p "$staging/target/release"

install -m 0755 "target/release/ac" "$staging/target/release/ac"
install -m 0755 "install.sh" "$staging/install.sh"
install -m 0644 "Cargo.toml" "$staging/Cargo.toml"
install -m 0644 "Cargo.lock" "$staging/Cargo.lock"
install -m 0644 "Dockerfile" "$staging/Dockerfile"
install -m 0644 "README.md" "$staging/README.md"
install -m 0644 "TUTORIAL.md" "$staging/TUTORIAL.md"
install -m 0644 "LICENSE" "$staging/LICENSE"

mkdir -p "$staging/src"
install -m 0644 src/*.rs "$staging/src/"

tarball="dist/$package.tar.gz"
tar -C dist -czf "$tarball" "$package"

echo "$tarball"
