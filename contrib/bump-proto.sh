#!/bin/sh
# Pin tetron-proto to the newest vX.Y.Z tag of the core repo (or $1).
set -eu
cd "$(dirname "$0")/.."
repo=https://github.com/tetronos/tetron
tag=${1:-$(git ls-remote --tags --refs "$repo" 'v*' | sed 's|.*refs/tags/||' |
	grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -n1)}
[ -n "$tag" ] || { echo "no release tag found" >&2; exit 1; }
sed -i -E "s@^(tetron-proto = \{ git = \"[^\"]+\", )(rev|tag) = \"[^\"]+\"@\1tag = \"$tag\"@" Cargo.toml
grep -q "tag = \"$tag\"" Cargo.toml || { echo "tetron-proto line not updated" >&2; exit 1; }
cargo update -q -p tetron-proto
echo "tetron-proto -> $tag"
