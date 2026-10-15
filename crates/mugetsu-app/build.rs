//! Stamps the build with its commit and date, shown in the About window, and
//! on Windows embeds the app icon in the executable.

use std::process::Command;

#[allow(clippy::disallowed_methods)] // build script, no window concerns
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let date = git(&["log", "-1", "--format=%cs"]).unwrap_or_default();
    println!("cargo:rustc-env=MUGETSU_COMMIT={commit}");
    println!("cargo:rustc-env=MUGETSU_DATE={date}");
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/refs/heads");
    }
    #[cfg(windows)]
    windows_icon();
}

/// Resource icon 1: Explorer and the taskbar show it, and gpui loads it for
/// the window (`LoadImageW(module, 1)`).
#[cfg(windows)]
fn windows_icon() {
    println!("cargo:rerun-if-changed=assets/icon/mugetsu.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon/mugetsu.ico")
        .set("ProductName", "Mugetsu")
        .set("FileDescription", "Mugetsu");
    res.compile().expect("embed Windows resources");
}
