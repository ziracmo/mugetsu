# Notes for AI sessions

## Releasing a new version

Releases are fully automated by `.github/workflows/release.yml`; nothing is uploaded by hand.

1. Bump `version` in `Cargo.toml` (`[workspace.package]`) and in `crates/mugetsu-app/Cargo.toml` and
   `crates/mugetsu-git/Cargo.toml` (they set it themselves), plus the `version = "…"` of the `mugetsu-forge` and
   `mugetsu-git` path dependencies in `crates/mugetsu-app/Cargo.toml`, then `cargo check` so `Cargo.lock` follows.
2. Write `releases/vX.Y.Z.json` (format in `releases/README.md`): `title`, `summary`, then `features`,
   `improvements`, `fixes`, each item `{id, title, body, scene?, highlight?}`. Look at the commits since the
   previous tag to fill it. Only real new capabilities go in `features`. `scene` picks the animation on the
   site's Releases page (`graph`, `fetch`, `search`, `diff`, `speed`, `pr`, `sidebar`, `accent`, `eclipse`).
   Check with `python3 scripts/release-notes.py check`.
3. Merge that to `main` (PR, CI green).
4. Trigger the release, either:
   - push a tag: `git tag vX.Y.Z && git push origin vX.Y.Z`, or
   - from a cloud session (tag pushes are refused there): run the **Release** workflow on `main` with input
     `tag = vX.Y.Z` (GitHub MCP `actions_run_trigger`, `run_workflow`). It creates the tag itself.
5. The workflow builds a universal `Mugetsu.dmg` (macOS) and `Mugetsu-Setup.exe` (Inno Setup, Windows), then
   publishes the GitHub release with `release-notes.json` attached and a body rendered from the JSON. Tags with
   a `-` (e.g. `v0.2.0-rc.1`) become prereleases, which `/releases/latest` ignores.
6. Nothing to do on the site (`ziracmo/mugetsu-site`): its Releases page and download buttons read the GitHub
   releases API live.

Pull requests that touch `scripts/`, `packaging/`, `releases/` or the workflow build the dmg and installer
without publishing, so packaging breakage shows up before a release.

Builds are unsigned for now (README › Download explains first launch and what signing would need).

## Conventions

- Commits are authored by Alexandre (`12600085+ziracmo@users.noreply.github.com`), with no AI trailers.
- Other sessions push to `main` concurrently: fetch and rebase often.
