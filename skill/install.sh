#!/bin/sh
# Installs the ai-ui skill for coding agents. Usage: sh install.sh [target-dir]
# Default target: .claude/skills/ai-ui   (use ~/.claude/skills/ai-ui for all projects)
set -eu
BASE="https://iyiolacak.github.io/ai-ui-rewrite-rust/skill/ai-ui"
DEST="${1:-.claude/skills/ai-ui}"
FILES="
  SKILL.md
  reference/backend.md
  reference/crates.md
  reference/runtime.md
  reference/slint-ui.md
  reference/troubleshooting.md
  reference/web.md
"
for f in $FILES; do
  mkdir -p "$DEST/$(dirname "$f")"
  echo "fetching $f"
  curl -fsSL "$BASE/$f" -o "$DEST/$f"
done
echo "Installed the ai-ui skill into $DEST"
