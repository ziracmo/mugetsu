//! Dock icon and name when running outside the `.app` bundle (e.g. `cargo run`), where
//! macOS would otherwise show the terminal's icon.

#[cfg(target_os = "macos")]
// objc 0.2's macros test cfgs this crate doesn't declare.
#[allow(unexpected_cfgs)]
pub fn set() {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};

    static PNG: &[u8] = include_bytes!("../assets/icon/mugetsu.png");
    // SAFETY: plain AppKit calls on the main thread with valid arguments;
    // a null image (undecodable data) is checked before use.
    unsafe {
        let data: *mut Object =
            msg_send![class!(NSData), dataWithBytes: PNG.as_ptr() length: PNG.len()];
        let image: *mut Object = msg_send![class!(NSImage), alloc];
        let image: *mut Object = msg_send![image, initWithData: data];
        if image.is_null() {
            return;
        }
        // Name shown in the Dock and the menu bar when run unbundled.
        let name: *mut Object =
            msg_send![class!(NSString), stringWithUTF8String: c"Mugetsu".as_ptr()];
        let info: *mut Object = msg_send![class!(NSProcessInfo), processInfo];
        let _: () = msg_send![info, setProcessName: name];
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setApplicationIconImage: image];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set() {}
