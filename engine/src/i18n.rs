//! Langue des textes produits par le moteur (conseils, états, rapports). Choisie par l'app (`set_lang`).

use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

pub fn en() -> bool {
    EN.load(Ordering::Relaxed)
}

/// "en…" -> anglais, sinon français.
pub fn set(lang: &str) {
    EN.store(lang.to_ascii_lowercase().starts_with("en"), Ordering::Relaxed);
}

/// Langue par défaut : variable d'environnement LANG (CLI), sinon français.
pub fn init_from_env() {
    if let Ok(l) = std::env::var("LANG") {
        set(&l);
    }
}

/// Texte formaté : `tr!("fr {x}", "en {x}", args…)` -> String
macro_rules! tr {
    ($fr:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if $crate::i18n::en() { format!($en $(, $a)*) } else { format!($fr $(, $a)*) }
    };
}

/// Texte statique : `trs!("fr", "en")` -> &'static str
macro_rules! trs {
    ($fr:literal, $en:literal) => {
        if $crate::i18n::en() { $en } else { $fr }
    };
}

pub(crate) use {tr, trs};
