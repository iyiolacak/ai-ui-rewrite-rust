#!/usr/bin/env python3
"""Checks docs/guide: front matter keys, order matches the file name, and relative links resolve."""
import re, sys, pathlib

root = pathlib.Path(__file__).resolve().parent.parent / "docs" / "guide"
problems = []
for p in sorted(root.glob("*.md")):
    text = p.read_text()
    m = re.match(r"---\n(.*?)\n---\n", text, re.S)
    if not m:
        problems.append(f"{p.name}: no front matter"); continue
    meta = dict(l.split(":", 1) for l in m.group(1).splitlines() if ":" in l)
    meta = {k.strip(): v.strip() for k, v in meta.items()}
    for key in ("title", "description", "order", "section"):
        if key not in meta:
            problems.append(f"{p.name}: front matter lacks {key}")
    if p.name != "README.md" and meta.get("order") and not p.name.startswith(f"{int(meta['order']):02d}-"):
        problems.append(f"{p.name}: order {meta['order']} does not match the file name")
    for target in re.findall(r"\]\(([^)#]+)(?:#[^)]*)?\)", text):
        if re.match(r"[a-z]+://|mailto:", target):
            continue
        if not (p.parent / target).resolve().exists():
            problems.append(f"{p.name}: broken link {target}")
print("\n".join(problems) if problems else f"ok: {len(list(root.glob('*.md')))} pages")
sys.exit(1 if problems else 0)
