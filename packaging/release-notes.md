## Download

| Platform | File |
| --- | --- |
| macOS 12+ (Apple silicon and Intel) | `Mugetsu.dmg` |
| Windows 10/11 (x64) | `Mugetsu-Setup.exe` |

### First launch on macOS

The app is not yet signed with an Apple Developer ID, so Gatekeeper blocks a plain double-click.

1. Open `Mugetsu.dmg` and drag **Mugetsu** onto **Applications**.
2. In Applications, **right-click Mugetsu › Open**, then **Open** in the dialog. Only needed once.

On macOS 15 and later, if no Open button shows: try to open it once, then go to **System Settings › Privacy & Security** and click **Open Anyway**. Or in a terminal: `xattr -dr com.apple.quarantine /Applications/Mugetsu.app`.

### First launch on Windows

The installer is not signed yet, so SmartScreen may warn: click **More info › Run anyway**.

Mugetsu needs `git` on the PATH (Xcode Command Line Tools on macOS, Git for Windows on Windows).
