# Releasing

Releases are fully automated by `.github/workflows/release.yml`; nothing is uploaded by hand.

1. Bump `version` in `Cargo.toml` (`[workspace.package]`), `crates/mugetsu-app/Cargo.toml` and
   `crates/mugetsu-git/Cargo.toml`, then `cargo check` so `Cargo.lock` follows.
2. Write `releases/vX.Y.Z.json` (format in [`releases/README.md`](../releases/README.md)) and check it with
   `python3 scripts/release-notes.py check`.
3. Merge to `main` with CI green.
4. Push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`, or run the **Release** workflow on `main` with
   input `tag = vX.Y.Z` (it creates the tag itself). Running it on an existing tag rebuilds that release.

The workflow builds a universal `Mugetsu.dmg` (macOS) and `Mugetsu-Setup.exe` (Inno Setup, Windows), then
publishes the GitHub release with `release-notes.json` attached and a body rendered from the JSON plus the
first-launch steps from `packaging/release-notes.md`. Tags containing a `-` (e.g. `v0.2.0-rc.1`) become
prereleases, which `/releases/latest` ignores.

The [site](https://ziracmo.github.io/mugetsu/releases.html) reads the GitHub releases API live: nothing to
do there.

Pull requests touching `scripts/`, `packaging/`, `releases/` or the workflow build the dmg and installer
without publishing, so packaging breakage shows up before a release. Every push to `main` also uploads a
zipped `Mugetsu-macos` app as a CI artifact.

## Signing (not done yet)

Builds are unsigned, hence the right-click › Open / SmartScreen steps in the README.

- **macOS**: an Apple Developer Program membership ($99/year) gives a *Developer ID Application*
  certificate. The workflow would import it (secrets: the `.p12` and its password), sign with
  `codesign --options runtime --timestamp`, notarize with `xcrun notarytool` (Apple ID app-specific password
  or App Store Connect API key) and `xcrun stapler staple` the dmg. Gatekeeper then opens the app with no
  warning.
- **Windows**: an OV/EV code-signing certificate (or Azure Trusted Signing) and `signtool sign` on
  `Mugetsu.exe` and the installer. SmartScreen warnings fade as signed builds gain reputation (immediately
  with EV).
