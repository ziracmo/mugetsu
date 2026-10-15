//! Spawning external programs from a GUI app.

use std::ffi::OsStr;
use std::process::Command;

/// A [`Command`] for `program` that never opens a console window: on Windows
/// the app has none, so each console program (git, gh, glab…) would otherwise
/// pop up its own. Every process the app spawns goes through this (clippy's
/// `disallowed-methods` forbids a bare `Command::new`).
#[allow(clippy::disallowed_methods, unused_mut)]
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
