#!/usr/bin/env bash
# The version in Cargo.toml is the next release, never one already out: ahead of the newest v* tag,
# or equal to it only on the tagged commit itself. web/package.json says the same.
set -euo pipefail

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
web=$(sed -n 's/^  "version": "\(.*\)",/\1/p' web/package.json | head -1)
[ "$version" = "$web" ] || { echo "Cargo.toml ($version) e web/package.json ($web) divergem"; exit 1; }

latest=$(git tag -l 'v*' --sort=-v:refname | head -1)
[ -n "$latest" ] || { echo "sem tags: $version"; exit 0; }
latest=${latest#v}

if git tag --points-at HEAD | grep -qx "v$version"; then
  echo "commit da tag v$version"
  exit 0
fi

newest=$(printf '%s\n%s\n' "$latest" "$version" | sort -V | tail -1)
if [ "$version" = "$latest" ] || [ "$newest" != "$version" ]; then
  echo "Cargo.toml diz $version, mas a v$latest já saiu: suba a versão para a próxima release"
  exit 1
fi
echo "$version está à frente da v$latest"
