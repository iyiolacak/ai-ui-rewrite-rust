#!/usr/bin/env bash
# Publish the landing page to GitHub Pages: build site/ for the repository's subpath and force-push only the built
# files, as a single commit, to the gh-pages branch of the public repository. No source history is pushed.
#   bash site/scripts/build-wasm.sh     # the demo, into site/public/wasm (git-ignored)
#   bash site/scripts/deploy-pages.sh
# Overrides: PAGES_REMOTE (push URL), SITE_BASE (default /ai-ui-rewrite-rust/), PAGES_NAME / PAGES_EMAIL (committer).
set -euo pipefail
site="$(cd "$(dirname "$0")/.." && pwd)"
remote="${PAGES_REMOTE:-https://github.com/iyiolacak/ai-ui-rewrite-rust.git}"
base="${SITE_BASE:-/ai-ui-rewrite-rust/}"
name="${PAGES_NAME:-iyiolacak}"
email="${PAGES_EMAIL:-142077242+iyiolacak@users.noreply.github.com}"

[ -f "$site/public/wasm/ai_ui_example_thread_bg.wasm" ] || { echo "build the demo first: bash site/scripts/build-wasm.sh" >&2; exit 1; }
cd "$site"
SITE_BASE="$base" npm run build

# Nothing from this machine may be published: no home-directory path in any built file, the .wasm included.
if grep -rlaF "$HOME" dist >&2; then echo "dist contains $HOME paths (listed above); not publishing" >&2; exit 1; fi
rm -f dist/_headers   # Cloudflare Pages configuration; GitHub Pages ignores it
touch dist/.nojekyll  # serve files as they are, including _astro/

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cp -r dist/. "$tmp/"
cd "$tmp"
git init -q -b gh-pages
git add -A
git -c user.name="$name" -c user.email="$email" commit -q -m "Landing page build, $(date -u +%Y-%m-%dT%H:%MZ)"
git push -q -f "$remote" gh-pages
echo "pushed $(git rev-parse --short HEAD) to $remote gh-pages"
