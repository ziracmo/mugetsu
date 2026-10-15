//! French / English UI strings. Strings live next to their use as
//! `tr("français", "English")`; the language comes from the settings.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_english(english: bool) {
    ENGLISH.store(english, Ordering::Relaxed);
}

pub fn is_fr() -> bool {
    !ENGLISH.load(Ordering::Relaxed)
}

/// Picks the string for the current language.
pub fn tr(fr: &'static str, en: &'static str) -> &'static str {
    if is_fr() { fr } else { en }
}

/// `format!` with one pattern per language: `trf!("{n} fichier(s)", "{n} file(s)")`.
#[macro_export]
macro_rules! trf {
    ($fr:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::is_fr() {
            format!($fr $(, $arg)*)
        } else {
            format!($en $(, $arg)*)
        }
    };
}
