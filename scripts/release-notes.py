#!/usr/bin/env python3
"""Release notes from releases/<tag>.json (format: releases/README.md).

  release-notes.py check                 validate every releases/*.json
  release-notes.py body <tag> <out.md>   write the GitHub release body for <tag>
                                         (SIGNED=true drops the unsigned first-launch steps)
  release-notes.py appcast <tag> <dmg> <edSignature> <out.xml>
                                         write the Sparkle appcast for <tag> (see src/updater.rs)
  release-notes.py latest                print the newest tag that has release notes
"""
import datetime
import email.utils
import html
import json
import os
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
    out.append(install_notes(os.environ.get("SIGNED") == "true"))
    return "\n".join(out) + "\n"


def install_notes(signed):
    """packaging/release-notes.md, minus the Gatekeeper workaround once the build is notarized."""
    text = (ROOT / "packaging/release-notes.md").read_text().rstrip()
    if signed:
        text = re.sub(r"### First launch on macOS\n.*?(?=\n### |\Z)", "", text, flags=re.S)
    return re.sub(r"\n{3,}", "\n\n", text).rstrip()


REPO = "https://github.com/ziracmo/mugetsu"


def appcast(data, tag, dmg, signature):
    """One-item Sparkle feed, published as a release asset: the app reads it from
    /releases/latest/download/appcast.xml, so prereleases never reach it."""
    notes = [f"<p>{html.escape(data['summary'])}</p>"]
    for key, heading in SECTIONS:
        if data[key]:
            notes.append(f"<h3>{heading}</h3><ul>")
            notes += [
                f"<li><b>{html.escape(it['title'])}</b>: {html.escape(it['body'])}</li>"
                for it in data[key]
            ]
            notes.append("</ul>")
    date = email.utils.format_datetime(
        datetime.datetime.fromisoformat(f"{data['date']}T12:00:00+00:00")
    )
    version = html.escape(data["version"])
    return f"""<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Mugetsu</title>
    <link>{REPO}</link>
    <item>
      <title>{html.escape(data['title'])}</title>
      <pubDate>{date}</pubDate>
      <sparkle:version>{version}</sparkle:version>
      <sparkle:shortVersionString>{version}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>12.0</sparkle:minimumSystemVersion>
      <sparkle:fullReleaseNotesLink>{REPO}/releases/tag/{tag}</sparkle:fullReleaseNotesLink>
      <description><![CDATA[{"".join(notes)}]]></description>
      <enclosure url="{REPO}/releases/download/{tag}/Mugetsu.dmg" length="{dmg.stat().st_size}"
        type="application/octet-stream" sparkle:edSignature="{html.escape(signature)}"/>
    </item>
  </channel>
</rss>
"""


def notes_for(tag):
    path = ROOT / "releases" / f"{tag}.json"
    if not path.exists():
        sys.exit(f"missing {path.relative_to(ROOT)}: write the release notes before tagging")
    return load(path)


if __name__ == "__main__":
    cmd = sys.argv[1:2]
    if cmd == ["check"]:
        files = sorted((ROOT / "releases").glob("v*.json"))
        for f in files:
            load(f)
        print(f"{len(files)} release file(s) OK")
    elif cmd == ["body"] and len(sys.argv) == 4:
        pathlib.Path(sys.argv[3]).write_text(body(notes_for(sys.argv[2])))
    elif cmd == ["appcast"] and len(sys.argv) == 6:
        tag, dmg, signature, out = sys.argv[2:]
        pathlib.Path(out).write_text(appcast(notes_for(tag), tag, pathlib.Path(dmg), signature))
    elif cmd == ["latest"]:
        tags = [f.stem for f in (ROOT / "releases").glob("v*.json")]
        print(max(tags, key=lambda t: [int(n) for n in re.findall(r"\d+", t)]))
    else:
        sys.exit(__doc__)
