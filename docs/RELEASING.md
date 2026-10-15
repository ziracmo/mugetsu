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

## Signing, notarization and auto-update

The macOS job does each of these only when its secrets exist, so the workflow works with none of them
(ad-hoc signed app, no updater, Gatekeeper workaround in the notes).

| Secret / variable | What |
| --- | --- |
| `MACOS_CERTIFICATE_P12` (secret) | *Developer ID Application* certificate + private key exported as `.p12`, base64 (`base64 -i cert.p12 \| pbcopy`) |
| `MACOS_CERTIFICATE_PASSWORD` (secret) | the `.p12` export password |
| `APPLE_ID` (secret) | Apple ID email of the developer account |
| `APPLE_APP_PASSWORD` (secret) | app-specific password for it (account.apple.com › Sign-In and Security) |
| `APPLE_TEAM_ID` (secret) | 10-character Team ID (developer.apple.com › Membership) |
| `SPARKLE_ED_PRIVATE_KEY` (secret) | Sparkle EdDSA private key (below) |
| `SPARKLE_PUBLIC_ED_KEY` (variable) | the matching public key |

- **Signing**: with the certificate, `bundle-macos.sh` signs Sparkle and the app with the hardened runtime
  and a timestamp, `dmg-macos.sh` signs the dmg.
- **Notarization**: with the Apple ID secrets too, `xcrun notarytool` notarizes the dmg and `stapler` staples
  it. The release notes then drop the right-click › Open steps (`SIGNED=true` in `release-notes.py body`).
- **Auto-update** (`src/updater.rs`): with the Sparkle key pair, the app embeds Sparkle
  (`SPARKLE_VERSION` in `bundle-macos.sh`) and reads
  `https://github.com/ziracmo/mugetsu/releases/latest/download/appcast.xml`. The workflow signs the dmg
  with `sign_update` and attaches `appcast.xml` to the release, so every published (non-pre)release is
  offered to installed apps; *Mugetsu › Check for Updates…* checks by hand. Builds from before the key
  existed have no updater: those users download once by hand. Keys come from Sparkle's own tool:
  `bin/generate_keys` in the [Sparkle release](https://github.com/sparkle-project/Sparkle/releases)
  stores the private key in the login Keychain and prints the public key; `generate_keys -x key.txt`
  exports the private one for the secret. Losing it means shipping one more manual-download release with
  a new key.
- **Windows**: not signed. New OV/EV certificates live on hardware tokens, which a hosted runner can't use;
  Azure Trusted Signing would be the route (`azure/trusted-signing-action` on `Mugetsu.exe` and the
  installer) once the account is eligible. SmartScreen warns until then.
