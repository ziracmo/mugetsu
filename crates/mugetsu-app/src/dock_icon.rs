//! Dock icon when running outside the `.app` bundle (e.g. `cargo run`), where
//! macOS would otherwise show the terminal's icon.

#[cfg(target_os = "macos")]
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
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setApplicationIconImage: image];
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set() {}
