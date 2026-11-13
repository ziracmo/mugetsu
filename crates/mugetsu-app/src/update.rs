//! Updates without the DMG: at launch and every few hours, asks GitHub for a
//! newer release and offers it in a toast (top right). "Nouveautés" opens its
//! release notes; "Mettre à jour" downloads it, verifies it, installs it in
//! place and relaunches Mugetsu.
//!
//! Trust: the release's `SHA256SUMS.txt` must carry a valid Ed25519
//! signature (`SHA256SUMS.txt.sig`, made by the release workflow with the
//! `UPDATE_SIGNING_KEY` secret) from [`PUBLIC_KEY`], and the downloaded
//! installer must hash to its line there. An unsigned or altered release is
//! never installed: the error toast offers the release page instead.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, FontWeight, Global, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::i18n::tr;
use crate::theme;

const REPO: &str = "ziracmo/mugetsu";

/// Verifies `SHA256SUMS.txt.sig`; its private half is the release workflow's
/// `UPDATE_SIGNING_KEY` secret (an Ed25519 PEM key).
const PUBLIC_KEY: [u8; 32] = [
    0xb0, 0x49, 0xa9, 0xfd, 0x68, 0x62, 0xa8, 0xb5, 0x0c, 0x68, 0x27, 0x3b, 0xf3, 0x1c, 0x92, 0x7e,
    0x39, 0x72, 0x44, 0xce, 0x53, 0xdd, 0x8a, 0x7b, 0x75, 0xae, 0x68, 0xe3, 0x9d, 0x06, 0x03, 0xbd,
];

/// First check after launch, then every [`CHECK_EVERY`].
const FIRST_CHECK: Duration = Duration::from_secs(8);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Releases whose notes the dialog shows at most (newest first).
const NOTES_MAX: usize = 5;
/// Larger downloads are refused.
const MAX_INSTALLER: u64 = 512 << 20;

// ---- Versions and release data ---------------------------------------------

/// `major.minor.patch`; prereleases (`0.9.0-rc.1`) are never offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut parts = s.split('.').map(|p| p.parse::<u64>().ok());
        let v = Self(parts.next()??, parts.next()??, parts.next()??);
        parts.next().is_none().then_some(v)
    }

    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("crate version is x.y.z")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApiRelease {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

impl ApiRelease {
    fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// `releases/vX.Y.Z.json`, attached to each release as `release-notes.json`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Notes {
    pub title: String,
    pub summary: String,
    pub features: Vec<NoteItem>,
    pub improvements: Vec<NoteItem>,
    pub fixes: Vec<NoteItem>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct NoteItem {
    pub title: String,
    pub body: String,
}

/// One release newer than the running app, with what the dialog shows.
#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    pub version: Version,
    pub url: String,
    pub notes: Option<Notes>,
    /// The release body (Markdown), shown when there are no notes.
    pub body: String,
}

/// What a check found: the releases newer than this one (newest first) and
/// the newest one's assets.
#[derive(Clone, Debug)]
pub struct Update {
    pub releases: Vec<ReleaseInfo>,
    latest: ApiRelease,
}

impl Update {
    pub fn latest(&self) -> &ReleaseInfo {
        &self.releases[0]
    }
}

/// Published (not draft, not prerelease) releases newer than `current`,
/// newest first, with their versions.
pub fn newer(releases: Vec<ApiRelease>, current: Version) -> Vec<(Version, ApiRelease)> {
    let mut out: Vec<(Version, ApiRelease)> = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| Some((Version::parse(&r.tag_name)?, r)))
        .filter(|(v, _)| *v > current)
        .collect();
    out.sort_by_key(|(v, _)| std::cmp::Reverse(*v));
    out
}

/// The installer this platform updates from.
pub fn installer_name() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("Mugetsu.dmg")
    } else if cfg!(windows) {
        Some("Mugetsu-Setup.exe")
    } else {
        None
    }
}

// ---- Verification ------------------------------------------------------------

/// Checks `sig` (raw 64-byte Ed25519) over `message` against `key`.
pub fn verify_signature(key: &[u8; 32], message: &[u8], sig: &[u8]) -> anyhow::Result<()> {
    let key = ed25519_dalek::VerifyingKey::from_bytes(key).context("clé publique invalide")?;
    let sig = ed25519_dalek::Signature::from_slice(sig).context("signature illisible")?;
    key.verify_strict(message, &sig)
        .context("signature des checksums invalide")
}

/// The SHA-256 `sha256sum` listed for `name` (`<hex>  <name>`, or `*<name>`
/// in binary mode).
pub fn expected_hash(sums: &str, name: &str) -> Option<[u8; 32]> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start();
        (file.strip_prefix('*').unwrap_or(file) == name).then(|| hex32(hash))?
    })
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    let s = s.as_bytes();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in s.chunks(2).enumerate() {
        let digit = |c: u8| (c as char).to_digit(16);
        out[i] = (digit(pair[0])? * 16 + digit(pair[1])?) as u8;
    }
    Some(out)
}

/// Only assets of this repository's releases are downloaded.
fn checked_url(asset: &Asset) -> anyhow::Result<&str> {
    let url = asset.browser_download_url.as_str();
    if url.starts_with(&format!("https://github.com/{REPO}/releases/download/")) {
        Ok(url)
    } else {
        bail!("URL de téléchargement inattendue : {url}")
    }
}

// ---- Network -------------------------------------------------------------------

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn get_small(url: &str, limit: u64) -> anyhow::Result<Vec<u8>> {
    Ok(agent(Duration::from_secs(20))
        .get(url)
        .call()?
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()?)
}

/// Asks GitHub for releases newer than this build (blocking).
pub fn check() -> anyhow::Result<Option<Update>> {
    let releases: Vec<ApiRelease> = agent(Duration::from_secs(15))
        .get(&format!(
            "https://api.github.com/repos/{REPO}/releases?per_page=20"
        ))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_json()?;
    let newer = newer(releases, Version::current());
    let Some(latest) = newer.first().map(|(_, r)| r.clone()) else {
        return Ok(None);
    };
    let releases = newer
        .into_iter()
        .take(NOTES_MAX)
        .map(|(version, r)| ReleaseInfo {
            version,
            notes: r
                .asset("release-notes.json")
                .and_then(|a| checked_url(a).ok())
                .and_then(|url| get_small(url, 256 << 10).ok())
                .and_then(|bytes| serde_json::from_slice(&bytes).ok()),
            url: r.html_url,
            body: r.body.unwrap_or_default(),
        })
        .collect();
    Ok(Some(Update { releases, latest }))
}

/// What is left once the new version is downloaded and verified (and, on
/// macOS, installed): start it once this process is gone.
pub struct Relaunch(#[allow(dead_code)] PathBuf);

impl Relaunch {
    /// Starts the waiting relauncher; the app must quit right after.
    pub fn spawn(self) -> anyhow::Result<()> {
        #[cfg(target_os = "macos")]
        {
            // Waits for this process to exit, then opens the new bundle.
            mugetsu_git::process::command("/bin/sh")
                .args([
                    "-c",
                    "while kill -0 \"$0\" 2>/dev/null; do sleep 0.2; done; open \"$1\"",
                ])
                .arg(std::process::id().to_string())
                .arg(&self.0)
                .spawn()?;
        }
        #[cfg(windows)]
        {
            // Inno Setup closes what's left of the app, installs over it and
            // starts it again (`/RELAUNCH=1`, see packaging/windows/mugetsu.iss).
            mugetsu_git::process::command(&self.0)
                .args([
                    "/VERYSILENT",
                    "/SUPPRESSMSGBOXES",
                    "/NORESTART",
                    "/CLOSEAPPLICATIONS",
                    "/RELAUNCH=1",
                ])
                .spawn()?;
        }
        Ok(())
    }
}

/// Downloads the newest release's installer, verifies it and installs it
/// (blocking).
pub fn download_and_install(update: &Update) -> anyhow::Result<Relaunch> {
    let name = installer_name().context(tr(
        "Pas de mise à jour automatique sur ce système",
        "No automatic update on this system",
    ))?;
    let release = &update.latest;
    let asset = |n: &str| {
        release.asset(n).with_context(|| {
            format!(
                "{n} {}",
                tr("absent de la version", "missing from the release")
            )
        })
    };
    let installer = asset(name)?;
    let sums = get_small(checked_url(asset("SHA256SUMS.txt")?)?, 64 << 10)?;
    let sig_asset = release.asset("SHA256SUMS.txt.sig").context(tr(
        "Version non signée : mise à jour automatique refusée",
        "Unsigned release: automatic update refused",
    ))?;
    let sig = get_small(checked_url(sig_asset)?, 1 << 10)?;
    verify_signature(&PUBLIC_KEY, &sums, &sig)?;
    let expected = expected_hash(&String::from_utf8_lossy(&sums), name)
        .with_context(|| format!("{name} absent de SHA256SUMS.txt"))?;

    let dir = std::env::temp_dir().join(format!("mugetsu-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(name);
    if let Err(e) = download(checked_url(installer)?, &file, &expected) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    install(&file, &dir)
}

/// Streams `url` into `path`, failing unless it hashes to `expected`.
fn download(url: &str, path: &Path, expected: &[u8; 32]) -> anyhow::Result<()> {
    let mut response = agent(Duration::from_secs(15 * 60)).get(url).call()?;
    let mut reader = response.body_mut().as_reader().take(MAX_INSTALLER);
    let mut out = std::fs::File::create(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    out.sync_all()?;
    if hasher.finalize().as_slice() != expected {
        bail!(tr(
            "Le fichier téléchargé ne correspond pas à son checksum",
            "The download doesn't match its checksum"
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn install(dmg: &Path, dir: &Path) -> anyhow::Result<Relaunch> {
    let result = install_dmg(dmg, dir);
    let _ = std::fs::remove_dir_all(dir);
    result.map(Relaunch)
}

/// Copies `Mugetsu.app` out of the DMG next to the running bundle, then swaps
/// the two (the old one is moved aside first and put back on failure).
#[cfg(target_os = "macos")]
fn install_dmg(dmg: &Path, dir: &Path) -> anyhow::Result<PathBuf> {
    let bundle = running_bundle().context(tr(
        "Mugetsu ne tourne pas depuis Mugetsu.app",
        "Mugetsu isn't running from Mugetsu.app",
    ))?;
    let parent = bundle.parent().context("no parent")?;
    let mount = dir.join("mount");
    std::fs::create_dir_all(&mount)?;
    run(mugetsu_git::process::command("hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-readonly",
            "-noautoopen",
            "-quiet",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(dmg))?;
    let staging = parent.join(".Mugetsu-update.app");
    let copied = (|| {
        let src = mount.join("Mugetsu.app");
        if !src.join("Contents/MacOS/Mugetsu").is_file() {
            bail!("Mugetsu.app absent de l'image disque");
        }
        let _ = std::fs::remove_dir_all(&staging);
        run(mugetsu_git::process::command("ditto")
            .arg(&src)
            .arg(&staging))
    })();
    let _ = run(mugetsu_git::process::command("hdiutil")
        .args(["detach", "-quiet", "-force"])
        .arg(&mount));
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let old = parent.join(".Mugetsu-old.app");
    let _ = std::fs::remove_dir_all(&old);
    if let Err(e) = std::fs::rename(&bundle, &old) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(anyhow::Error::from(e).context(format!(
            "{} {}",
            tr("Impossible de remplacer", "Can't replace"),
            bundle.display()
        )));
    }
    if let Err(e) = std::fs::rename(&staging, &bundle) {
        let _ = std::fs::rename(&old, &bundle);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e.into());
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(bundle)
}

/// `/Applications/Mugetsu.app` when running from it.
#[cfg(target_os = "macos")]
fn running_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension()? == "app").then(|| bundle.to_path_buf())
}

#[cfg(target_os = "macos")]
fn run(cmd: &mut std::process::Command) -> anyhow::Result<()> {
    let out = cmd.output()?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(windows)]
fn install(setup: &Path, _dir: &Path) -> anyhow::Result<Relaunch> {
    Ok(Relaunch(setup.to_path_buf()))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn install(_: &Path, dir: &Path) -> anyhow::Result<Relaunch> {
    let _ = std::fs::remove_dir_all(dir);
    bail!("No automatic update on this system")
}

// ---- UI ------------------------------------------------------------------------

/// The last update found, and whether one is being installed.
#[derive(Default)]
struct Updates {
    found: Option<Arc<Update>>,
    /// Version already offered by the periodic check (not offered again).
    offered: Option<Version>,
    installing: bool,
}
impl Global for Updates {}

struct UpdateToast;
struct ProgressToast;

/// Starts the periodic check (release builds, when enabled in Settings).
pub fn init(cx: &mut App) {
    cx.set_global(Updates::default());
    if cfg!(debug_assertions) {
        return;
    }
    cx.spawn(async move |cx| {
        let mut wait = FIRST_CHECK;
        loop {
            cx.background_executor().timer(wait).await;
            wait = CHECK_EVERY;
            if crate::settings::get().update_check {
                cx.update(|cx| check_now(false, cx));
            }
        }
    })
    .detach();
}

/// Checks now; `manual` (menu, Settings) also says when there's nothing new
/// or the check failed, and offers a version again.
pub fn check_now(manual: bool, cx: &mut App) {
    if cx.global::<Updates>().installing {
        return;
    }
    let task = cx.background_spawn(async move { check() });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok(Some(update)) => {
                let version = update.latest().version;
                let state = cx.global_mut::<Updates>();
                state.found = Some(Arc::new(update));
                if manual || state.offered != Some(version) {
                    state.offered = Some(version);
                    offer(cx);
                }
            }
            Ok(None) if manual => notify(
                Notification::success(format!(
                    "{} ({}).",
                    tr("Mugetsu est à jour", "Mugetsu is up to date"),
                    Version::current()
                ))
                .id::<UpdateToast>(),
                cx,
            ),
            Err(e) if manual => notify(
                crate::toast::error(
                    tr("Vérification impossible", "Couldn't check for updates"),
                    &e,
                    None,
                )
                .id::<UpdateToast>(),
                cx,
            ),
            _ => {}
        });
    })
    .detach();
}

/// Pushes `note` in the main window (top right).
fn notify(note: Notification, cx: &mut App) {
    let handle = cx
        .try_global::<crate::app_menu::MainWorkspace>()
        .map(|m| m.0)
        .or_else(|| cx.active_window());
    if let Some(handle) = handle {
        let _ = handle.update(cx, |_, window, cx| window.push_notification(note, cx));
    }
}

/// The toast offering the update found.
fn offer(cx: &mut App) {
    let Some(update) = cx.global::<Updates>().found.clone() else {
        return;
    };
    let latest = update.latest();
    let message = latest
        .notes
        .as_ref()
        .map(|n| n.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            tr(
                "Une nouvelle version de Mugetsu est prête.",
                "A new version of Mugetsu is ready.",
            )
            .to_string()
        });
    let note = Notification::info(message)
        .id::<UpdateToast>()
        .title(format!(
            "Mugetsu {} {}",
            latest.version,
            tr("est disponible", "is available")
        ))
        .autohide(false)
        .content(move |_, _, cx| {
            h_flex()
                .pt_2()
                .gap_2()
                .child(
                    Button::new("update-install")
                        .primary()
                        .small()
                        .label(tr("Mettre à jour", "Update"))
                        .on_click(cx.listener(|note, _, window, cx| {
                            note.dismiss(window, cx);
                            install_now(window, cx);
                        })),
                )
                .child(
                    Button::new("update-notes")
                        .outline()
                        .small()
                        .label(tr("Nouveautés", "What's new"))
                        .on_click(cx.listener(|note, _, window, cx| {
                            note.dismiss(window, cx);
                            open_notes(window, cx);
                        })),
                )
                .child(
                    Button::new("update-later")
                        .ghost()
                        .small()
                        .label(tr("Plus tard", "Later"))
                        .on_click(cx.listener(|note, _, window, cx| note.dismiss(window, cx))),
                )
                .into_any_element()
        });
    notify(note, cx);
}

/// The dialog listing what changed since this version.
pub fn open_notes(window: &mut Window, cx: &mut App) {
    let Some(update) = cx.global::<Updates>().found.clone() else {
        return;
    };
    window.open_dialog(cx, move |dialog, _, _| {
        let latest = update.latest().version;
        dialog
            .bg(theme::sheet())
            .title(format!(
                "Mugetsu {latest} · {}",
                tr("Nouveautés", "What's new")
            ))
            .w(px(560.))
            .child(
                v_flex()
                    .id("update-notes")
                    .max_h(px(440.))
                    .overflow_y_scroll()
                    .gap_5()
                    .pr_2()
                    .children(update.releases.iter().map(render_release)),
            )
            .footer(
                DialogFooter::new()
                    .child(
                        DialogClose::new().trigger(|b| b.ghost().label(tr("Plus tard", "Later"))),
                    )
                    .child(
                        DialogAction::new().child(
                            Button::new("update-ok")
                                .primary()
                                .label(tr("Mettre à jour et relancer", "Update and relaunch")),
                        ),
                    ),
            )
            .on_ok(|_, window, cx| {
                install_now(window, cx);
                true
            })
    });
}

fn render_release(r: &ReleaseInfo) -> gpui_kit::AnyElement {
    let head = h_flex()
        .gap_2()
        .items_baseline()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(theme::accent())
                .child(format!("v{}", r.version)),
        )
        .children(r.notes.as_ref().filter(|n| !n.title.is_empty()).map(|n| {
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(n.title.clone())
        }));
    let Some(notes) = &r.notes else {
        return v_flex()
            .gap_2()
            .child(head)
            .child(gpui_kit::component::text::TextView::markdown(
                SharedString::from(format!("update-body-{}", r.version)),
                // The download steps after the notes don't apply here.
                r.body
                    .split("\n## Download")
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            ))
            .into_any_element();
    };
    let section = |label: &'static str, items: &[NoteItem]| {
        (!items.is_empty()).then(|| {
            v_flex()
                .gap_1p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::muted())
                        .child(label.to_uppercase()),
                )
                .children(items.iter().map(|item| {
                    v_flex()
                        .pl_2()
                        .border_l_2()
                        .border_color(theme::alpha(theme::accent(), 0.5))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(item.title.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::muted())
                                .child(item.body.clone()),
                        )
                }))
        })
    };
    v_flex()
        .gap_3()
        .child(head)
        .children((!notes.summary.is_empty()).then(|| {
            div()
                .text_sm()
                .text_color(theme::muted())
                .child(notes.summary.clone())
        }))
        .children(section(tr("Nouveautés", "New"), &notes.features))
        .children(section(
            tr("Améliorations", "Improvements"),
            &notes.improvements,
        ))
        .children(section(tr("Corrections", "Fixes"), &notes.fixes))
        .into_any_element()
}

/// Downloads, verifies and installs the update found, then relaunches.
fn install_now(window: &mut Window, cx: &mut App) {
    let state = cx.global::<Updates>();
    let Some(update) = state.found.clone().filter(|_| !state.installing) else {
        return;
    };
    let page = update.latest().url.clone();
    if installer_name().is_none() {
        cx.open_url(&page);
        return;
    }
    cx.global_mut::<Updates>().installing = true;
    window.push_notification(
        Notification::info(tr(
            "Téléchargement et vérification…",
            "Downloading and verifying…",
        ))
        .id::<ProgressToast>()
        .title(format!(
            "{} {}",
            tr("Mise à jour vers", "Updating to"),
            update.latest().version
        ))
        .autohide(false),
        cx,
    );
    let task = cx.background_spawn(async move { download_and_install(&update) });
    cx.spawn(async move |cx| {
        let result = task.await.and_then(Relaunch::spawn);
        cx.update(|cx| {
            cx.global_mut::<Updates>().installing = false;
            match result {
                Ok(()) => cx.quit(),
                Err(e) => {
                    let open: crate::toast::Action = (
                        tr("Ouvrir la page de la version", "Open the release page").into(),
                        Rc::new(move |_, cx: &mut App| cx.open_url(&page)),
                    );
                    for handle in cx.windows() {
                        let _ = handle.update(cx, |_, window, cx| {
                            window.remove_notification::<ProgressToast>(cx)
                        });
                    }
                    notify(
                        crate::toast::error(
                            tr("Mise à jour impossible", "Update failed"),
                            &e,
                            Some(open),
                        )
                        .id::<UpdateToast>(),
                        cx,
                    );
                }
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> ApiRelease {
        ApiRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            html_url: String::new(),
            body: None,
            assets: Vec::new(),
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(Version::parse("v0.10.0"), Some(Version(0, 10, 0)));
        assert!(Version::parse("0.10.0") > Version::parse("0.9.3"));
        assert_eq!(Version::parse("v0.9.0-rc.1"), None);
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert!(Version::parse(env!("CARGO_PKG_VERSION")).is_some());
    }

    #[test]
    fn only_newer_published_releases_are_offered() {
        let found = newer(
            vec![
                release("v0.7.0", false),
                release("v0.9.0-rc.1", true),
                release("v0.8.0", false),
                release("v0.8.1", false),
                release("v0.7.1", false),
            ],
            Version(0, 7, 1),
        );
        let tags: Vec<_> = found.iter().map(|(_, r)| r.tag_name.as_str()).collect();
        assert_eq!(tags, ["v0.8.1", "v0.8.0"]);
    }

    #[test]
    fn checksums_are_read_like_sha256sum_writes_them() {
        let h = "ab".repeat(32);
        let sums = format!("{}  Mugetsu.dmg\n{h} *Mugetsu-Setup.exe\n", "00".repeat(32));
        assert_eq!(expected_hash(&sums, "Mugetsu.dmg"), Some([0; 32]));
        assert_eq!(expected_hash(&sums, "Mugetsu-Setup.exe"), Some([0xab; 32]));
        assert_eq!(expected_hash(&sums, "Mugetsu"), None);
        assert_eq!(hex32("zz"), None);
    }

    /// Signed with `openssl pkeyutl -sign -rawin`, like the release workflow.
    #[test]
    fn openssl_signatures_verify() {
        const KEY: [u8; 32] = [
            0x25, 0xcf, 0x95, 0x27, 0xe1, 0xab, 0xd9, 0x62, 0x90, 0xee, 0xe8, 0x3d, 0xcb, 0xac,
            0x3c, 0xd6, 0x0b, 0x8c, 0x4a, 0x2a, 0xb9, 0xa3, 0xa0, 0x91, 0x42, 0x98, 0x82, 0x9f,
            0xe4, 0x77, 0x96, 0xc6,
        ];
        const SIG: [u8; 64] = [
            0x42, 0x01, 0xfc, 0x6d, 0xcf, 0x37, 0xca, 0x83, 0x7e, 0xf0, 0xc2, 0x74, 0xf3, 0xaa,
            0xe3, 0x1c, 0xac, 0x03, 0x17, 0xf3, 0x5f, 0x20, 0x59, 0x15, 0x11, 0xc8, 0xbe, 0xce,
            0x48, 0xc9, 0x92, 0x5a, 0x34, 0xa9, 0xa6, 0x3d, 0x6f, 0x4f, 0xff, 0xce, 0xba, 0x58,
            0xbb, 0xcc, 0x0a, 0x25, 0xd3, 0x9b, 0xb8, 0xbb, 0x9d, 0x81, 0x2e, 0xfb, 0xfb, 0x07,
            0xd8, 0xe0, 0x3e, 0x4e, 0xaf, 0xcb, 0xa3, 0x0d,
        ];
        let sums = b"abc  Mugetsu.dmg\n";
        assert!(verify_signature(&KEY, sums, &SIG).is_ok());
        assert!(verify_signature(&KEY, b"abd  Mugetsu.dmg\n", &SIG).is_err());
        assert!(verify_signature(&PUBLIC_KEY, sums, &SIG).is_err());
        assert!(verify_signature(&KEY, sums, &SIG[..63]).is_err());
    }

    #[test]
    fn downloads_stay_on_the_repository() {
        let asset = |url: &str| Asset {
            name: "Mugetsu.dmg".into(),
            browser_download_url: url.into(),
        };
        assert!(
            checked_url(&asset(&format!(
                "https://github.com/{REPO}/releases/download/v0.8.0/Mugetsu.dmg"
            )))
            .is_ok()
        );
        assert!(checked_url(&asset("https://evil.example/Mugetsu.dmg")).is_err());
        assert!(
            checked_url(&asset(
                "http://github.com/ziracmo/mugetsu/releases/download/x"
            ))
            .is_err()
        );
    }

    #[test]
    fn release_notes_parse() {
        let notes: Notes = serde_json::from_str(
            r#"{"version":"0.8.0","title":"T","summary":"S",
                "features":[{"id":"x","title":"A","body":"B","scene":"graph"}]}"#,
        )
        .unwrap();
        assert_eq!(notes.features[0].title, "A");
        assert!(notes.fixes.is_empty());
    }
}
