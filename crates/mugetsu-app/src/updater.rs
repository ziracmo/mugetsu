//! Auto-update through Sparkle (macOS). `scripts/bundle-macos.sh` embeds `Sparkle.framework`
//! and sets `SUFeedURL` / `SUPublicEDKey` when the build has an update key; the framework is
//! loaded at run time, so `cargo run` and keyless builds simply have no updater.
//! Sparkle checks the appcast published with each GitHub release, shows its own update
//! window, verifies the EdDSA signature, swaps the app and relaunches.

#[cfg(target_os = "macos")]
// objc 0.2's macros test cfgs this crate doesn't declare.
#[allow(unexpected_cfgs)]
mod imp {
    use objc::runtime::{BOOL, Class, NO, Object, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use std::sync::OnceLock;

    /// The `SPUStandardUpdaterController`, kept for the app's lifetime (never released).
    static CONTROLLER: OnceLock<usize> = OnceLock::new();

    unsafe fn ns_string(s: &std::ffi::CStr) -> *mut Object {
        unsafe { msg_send![class!(NSString), stringWithUTF8String: s.as_ptr()] }
    }

    pub fn init() {
        // SAFETY: AppKit / Sparkle calls on the main thread; every nil result is checked
        // before use and the class is looked up only once the framework is loaded.
        unsafe {
            let main: *mut Object = msg_send![class!(NSBundle), mainBundle];
            let key: *mut Object =
                msg_send![main, objectForInfoDictionaryKey: ns_string(c"SUPublicEDKey")];
            if key.is_null() {
                return;
            }
            let frameworks: *mut Object = msg_send![main, privateFrameworksPath];
            if frameworks.is_null() {
                return;
            }
            let path: *mut Object = msg_send![frameworks,
                stringByAppendingPathComponent: ns_string(c"Sparkle.framework")];
            let bundle: *mut Object = msg_send![class!(NSBundle), bundleWithPath: path];
            if bundle.is_null() {
                return;
            }
            let loaded: BOOL = msg_send![bundle, load];
            if loaded == NO {
                return;
            }
            let Some(cls) = Class::get("SPUStandardUpdaterController") else {
                return;
            };
            let controller: *mut Object = msg_send![cls, alloc];
            let nil: *mut Object = std::ptr::null_mut();
            let controller: *mut Object = msg_send![controller,
                initWithStartingUpdater: YES
                updaterDelegate: nil
                userDriverDelegate: nil];
            if !controller.is_null() {
                let _ = CONTROLLER.set(controller as usize);
            }
        }
    }

    pub fn available() -> bool {
        CONTROLLER.get().is_some()
    }

    pub fn check() {
        if let Some(&controller) = CONTROLLER.get() {
            let controller = controller as *mut Object;
            let nil: *mut Object = std::ptr::null_mut();
            // SAFETY: a live controller (never released), called on the main thread.
            unsafe {
                let _: () = msg_send![controller, checkForUpdates: nil];
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn init() {}
    pub fn available() -> bool {
        false
    }
    pub fn check() {}
}

/// Whether "Check for Updates…" has something to call.
pub use imp::available;
/// Mugetsu > Check for Updates…: Sparkle's own window, even when up to date.
pub use imp::check;
/// Starts Sparkle (background checks on its own schedule) when this build carries it.
pub use imp::init;
