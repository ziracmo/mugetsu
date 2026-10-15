#!/usr/bin/env python3
"""Release notes from releases/<tag>.json (format: releases/README.md).

  release-notes.py check                 validate every releases/*.json
  release-notes.py body <tag> <out.md>   write the GitHub release body for <tag>
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SECTIONS = [("features", "Features"), ("improvements", "Improvements"), ("fixes", "Fixes")]
SCENES = {"graph", "fetch", "search", "diff", "speed", "pr", "sidebar", "accent", "eclipse"}
ID = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")


def load(path):
    data = json.loads(path.read_text())
    errors = []
    tag = path.stem
    if f"v{data.get('version')}" != tag:
        errors.append(f"version {data.get('version')!r} does not match the file name {tag}")
    if not re.match(r"^\d{4}-\d{2}-\d{2}$", str(data.get("date", ""))):
        errors.append("date must be YYYY-MM-DD")
    for key in ("title", "summary"):
        if not isinstance(data.get(key), str) or not data[key].strip():
            errors.append(f"{key} is required")
    total = 0
    for key, _ in SECTIONS:
        items = data.get(key)
        if not isinstance(items, list):
            errors.append(f"{key} must be a list")
            continue
        total += len(items)
        for i, item in enumerate(items):
            for field in ("id", "title", "body"):
                if not isinstance(item.get(field), str) or not item[field].strip():
                    errors.append(f"{key}[{i}].{field} is required")
            if isinstance(item.get("id"), str) and not ID.match(item["id"]):
                errors.append(f"{key}[{i}].id must be kebab-case")
            if "scene" in item and item["scene"] not in SCENES:
                errors.append(f"{key}[{i}].scene must be one of {sorted(SCENES)}")
    if total == 0:
        errors.append("features, improvements and fixes are all empty")
    if data.get("features") and not any(f.get("highlight") for f in data["features"]):
        errors.append("highlight at least one feature")
    if errors:
        sys.exit(f"{path.relative_to(ROOT)}:\n  " + "\n  ".join(errors))
    return data


def body(data):
    out = [f"# {data['title']}", "", data["summary"], ""]
    for key, heading in SECTIONS:
        if data[key]:
            out += [f"## {heading}", ""]
            out += [f"- {it['title']}: {it['body']}" for it in data[key]]
            out.append("")
    out.append((ROOT / "packaging/release-notes.md").read_text().rstrip())
    return "\n".join(out) + "\n"


if __name__ == "__main__":
    cmd = sys.argv[1:2]
    if cmd == ["check"]:
        files = sorted((ROOT / "releases").glob("v*.json"))
        for f in files:
            load(f)
        print(f"{len(files)} release file(s) OK")
    elif cmd == ["body"] and len(sys.argv) == 4:
        path = ROOT / "releases" / f"{sys.argv[2]}.json"
        if not path.exists():
            sys.exit(f"missing {path.relative_to(ROOT)}: write the release notes before tagging")
        pathlib.Path(sys.argv[3]).write_text(body(load(path)))
    else:
        sys.exit(__doc__)
