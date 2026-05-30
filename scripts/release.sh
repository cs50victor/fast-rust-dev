#!/usr/bin/env bash
# Cut a release: bump the version, regenerate CHANGELOG.md from Conventional Commits,
# commit, and tag. Review the result, then push to trigger the Release workflow.
#
#   scripts/release.sh 0.2.0
#   git push origin main v0.2.0
#
# Requires git-cliff (cargo binstall git-cliff) and cargo-set-version (cargo install cargo-edit).
set -euo pipefail

version="${1:-}"
if [[ -z "$version" ]]; then
  echo "usage: scripts/release.sh <version>   e.g. scripts/release.sh 0.2.0" >&2
  exit 2
fi
tag="v${version}"

cd "$(dirname "$0")/.."

for tool in git-cliff cargo; do
  command -v "$tool" >/dev/null || { echo "missing required tool: $tool" >&2; exit 1; }
done

if [[ -n "$(git status --porcelain)" ]]; then
  echo "working tree is dirty; commit or stash before releasing" >&2
  exit 1
fi

cargo set-version "$version"
# --tag assigns the unreleased commits to this version so the new section is dated now.
git-cliff --config cliff.toml --tag "$tag" --output CHANGELOG.md

git add CHANGELOG.md Cargo.toml Cargo.lock
git commit -m "chore(release): $tag"
git tag "$tag"

echo
echo "Tagged $tag. Review the commit, then push:"
echo "  git push origin main $tag"
