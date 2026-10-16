// Release builds on Windows: no console window behind the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod about;
mod ai_review;
mod app_menu;
mod avatars;
mod branch_tree;
mod dialogs;
mod dock_icon;
mod fetch_policy;
mod fmt;
#[macro_use]
mod i18n;
mod graph_view;
mod model;
mod notify;
mod open_repo;
mod overview;
mod palette;
mod pr;
mod recap;
mod repo_icon;
mod repo_view;
mod session;
mod settings;
mod settings_view;
mod theme;
mod toast;
mod welcome;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use gpui_kit::{
    AppContext as _, WindowBackgroundAppearance, WindowBounds, WindowOptions, px, size,
};

use session::Session;
use workspace::Workspace;

/// Apps launched from Finder get a minimal PATH; borrow the login shell's so
/// `git`, `gh`, credential helpers and ssh agents resolve like in the terminal.
fn inherit_shell_path() {
    #[cfg(target_os = "macos")]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        if let Ok(out) = mugetsu_git::process::command(shell)
            .args(["-l", "-c", "printf %s \"$PATH\""])
            .output()
            && out.status.success()
            && let Ok(path) = String::from_utf8(out.stdout)
            && !path.is_empty()
        {
            // SAFETY: called first thing in main, before any thread is spawned.
            unsafe { std::env::set_var("PATH", path) };
        }
    }
}

/// Repository windows still open; the app quits when the last one closes
/// (the Settings and About windows don't keep it alive).
struct OpenWorkspaces(Vec<gpui_kit::WindowId>);
impl gpui_kit::Global for OpenWorkspaces {}

/// Opens a repository window on `session`; only the first one (`persist`)
/// saves its tabs and workspaces.
pub fn open_workspace_window(
    session: Session,
    persist: bool,
    cx: &mut gpui_kit::App,
) -> anyhow::Result<(gpui_kit::AnyWindowHandle, gpui_kit::Entity<Workspace>)> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
        window_min_size: Some(size(px(960.), px(600.))),
        app_id: Some("dev.mugetsu".into()),
        // Glass look: the translucent theme lets the blurred desktop through.
        window_background: WindowBackgroundAppearance::Blurred,
        // Taller title bar (see workspace::TITLE_HEIGHT): lights centred in it.
        titlebar: Some(gpui_kit::TitlebarOptions {
            traffic_light_position: Some(gpui_kit::point(px(14.), px(15.))),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    };
    let (handle, workspace) = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("Mugetsu");
        cx.new(|cx| Workspace::new(session, persist, window, cx))
    })?;
    let id = handle.window_id();
    if !cx.has_global::<OpenWorkspaces>() {
        cx.set_global(OpenWorkspaces(Vec::new()));
        cx.on_window_closed(|cx, id| {
            let open = &mut cx.global_mut::<OpenWorkspaces>().0;
            open.retain(|w| *w != id);
            if open.is_empty() {
                cx.quit()
            }
        })
        .detach();
    }
    cx.global_mut::<OpenWorkspaces>().0.push(id);
    Ok((handle, workspace))
}

fn main() {
    inherit_shell_path();
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let session = Session::load().with_args(args);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            dock_icon::set();
            settings::install(settings::Settings::load(), cx);
            avatars::init(cx);
            workspace::init(cx);
            repo_view::init(cx);
            app_menu::init(cx);
            palette::init(cx);
            recap::init(cx);
            let main =
                open_workspace_window(session, true, cx).expect("failed to open the main window");
            cx.set_global(app_menu::MainWorkspace(main.0, main.1.clone()));
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests;
