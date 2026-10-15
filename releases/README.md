# Release notes

One file per release, `releases/<tag>.json` (e.g. `v0.2.0.json`), written before tagging.
The release workflow refuses to publish a tag without one; `scripts/release-notes.py check` validates them.

```json
{
  "version": "0.2.0",
  "date": "2026-11-02",
  "title": "One line that sums up the release",
  "summary": "A short paragraph.",
  "features":     [{ "id": "stats-view", "title": "Stats view", "body": "…", "scene": "graph", "highlight": true }],
  "fixes":        [{ "id": "…", "title": "…", "body": "…" }],
  "improvements": [{ "id": "…", "title": "…", "body": "…" }]
}
```

- `features`: new things the user can do. `fixes`: bugs fixed. `improvements`: existing things made better
  (speed, polish). Any of the three can be empty, not all of them.
- `id`: stable kebab-case slug of the feature; reuse it when a later release touches the same feature.
- `scene` (optional): the animation the site plays for the item, one of `graph`, `fetch`, `search`, `diff`,
  `speed`, `pr`, `sidebar`, `accent`, `eclipse`.
- `highlight: true` marks the items to feature first (at least one feature when there are features).

The workflow attaches the file to the release as `release-notes.json` and writes the release body with
`## Features`, `## Improvements` and `## Fixes` sections (`- Title: body`), followed by the download and
first-launch steps from `packaging/release-notes.md`. The site's Releases page reads both from the GitHub
releases API, so nothing needs rebuilding there.
