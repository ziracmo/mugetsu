//! Author pictures (GitHub / Gravatar), downloaded once and cached on disk.
//! Falls back to coloured initials while loading, offline, or when unset.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::{
    AnyElement, App, AppContext as _, Entity, Global, IntoElement, ParentElement as _, Styled as _,
    div, img, px,
};

use crate::theme;

const FETCH_SIZE: u32 = 64;

enum State {
    Loading,
    Ready(PathBuf),
    Missing,
}

pub struct Avatars {
    states: HashMap<String, State>,
    dir: Option<PathBuf>,
}

struct GlobalAvatars(Entity<Avatars>);
impl Global for GlobalAvatars {}

/// Enables downloads (the app calls this; tests don't, so they stay offline).
pub fn init(cx: &mut App) {
    let dir = dirs::cache_dir().map(|d| d.join("Mugetsu").join("avatars"));
    let store = cx.new(|_| Avatars {
        states: HashMap::new(),
        dir,
    });
    cx.set_global(GlobalAvatars(store));
}

/// The store, for views that want to re-render when pictures arrive.
pub fn store(cx: &App) -> Option<Entity<Avatars>> {
    cx.try_global::<GlobalAvatars>().map(|g| g.0.clone())
}

/// FNV-1a: stable across runs, for file names and colours.
fn hash(email: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in email.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

fn file_name(email: &str) -> String {
    format!("{:016x}.img", hash(email))
}

fn request(store: Entity<Avatars>, email: String, cx: &mut App) {
    let Some(dir) = store.read(cx).dir.clone() else {
        return;
    };
    store.update(cx, |s, _| {
        s.states.insert(email.clone(), State::Loading);
    });
    let path = dir.join(file_name(&email));
    let task = cx.background_spawn({
        let path = path.clone();
        let email = email.clone();
        async move {
            if path.exists() {
                return true;
            }
            match mugetsu_forge::fetch_avatar(&email, FETCH_SIZE) {
                Ok(Some(bytes)) => {
                    std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&path, bytes).is_ok()
                }
                _ => false,
            }
        }
    });
    cx.spawn(async move |cx| {
        let found = task.await;
        store.update(cx, |s, cx| {
            s.states.insert(
                email,
                if found {
                    State::Ready(path)
                } else {
                    State::Missing
                },
            );
            cx.notify();
        });
    })
    .detach();
}

fn initials(name: &str) -> String {
    let mut parts = name.split_whitespace().filter_map(|w| w.chars().next());
    let first = parts.next().unwrap_or('?');
    let last = parts.next_back();
    first
        .to_uppercase()
        .chain(last.into_iter().flat_map(char::to_uppercase))
        .collect()
}

/// A round picture of `size` px for the author, or their initials.
pub fn avatar(email: &str, name: &str, size: f32, cx: &mut App) -> AnyElement {
    let key = email.trim().to_lowercase();
    if let Some(store) = store(cx) {
        match store.read(cx).states.get(&key) {
            Some(State::Ready(path)) => {
                return img(path.clone())
                    .flex_none()
                    .size(px(size))
                    .rounded_full()
                    .into_any_element();
            }
            Some(_) => {}
            None if !key.is_empty() => {
                // Not during this render pass: start the download right after.
                let key = key.clone();
                cx.defer(move |cx| request(store, key, cx));
            }
            None => {}
        }
    }
    let color = theme::lane_color((hash(&key) % 10) as u16);
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::alpha(color, 0.3))
        .text_color(color)
        .text_size(px(size * 0.42))
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .child(initials(name))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_of_names() {
        assert_eq!(initials("Roussange Alexandre"), "RA");
        assert_eq!(initials("jif"), "J");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("ana maría lópez"), "AL");
    }
}
