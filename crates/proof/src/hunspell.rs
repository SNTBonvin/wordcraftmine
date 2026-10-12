//! Hunspell dictionaries installed on the system, read at runtime through the shared
//! `craft-proof` crate.
//!
//! WordCraft doesn't bundle large spelling dictionaries. For languages with an affix dictionary
//! (Polish, Portuguese), it reads the standard Hunspell `.aff`/`.dic` pair installed by the
//! operating system, a package manager or LibreOffice. When none is installed the language has no
//! spelling check (no marks, no errors), like Word without the language's proofing tools.
//!
//! Search order: the directory set with [`set_dictionary_dir`] (a setting), then
//! `WORDCRAFT_DICTIONARIES`, `%LOCALAPPDATA%\WordCraft\dictionaries` on Windows, and the usual
//! system locations. On the web there are none.

use std::path::PathBuf;
use std::sync::RwLock;

pub use craft_proof::hunspell::{decode, find_in, load};

/// An extra directory searched first (a setting), set by the front end.
static EXTRA_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Search `dir` before the system locations (a "dictionaries folder" setting); `None` clears it.
/// Takes effect for dictionaries not loaded yet.
pub fn set_dictionary_dir(dir: Option<PathBuf>) {
    *EXTRA_DIR.write().unwrap_or_else(|e| e.into_inner()) = dir;
}

/// WordCraft's search: its own folder setting, `WORDCRAFT_DICTIONARIES` and `WordCraft` folders.
fn search() -> craft_proof::hunspell::Search {
    let extra_dir = EXTRA_DIR.read().unwrap_or_else(|e| e.into_inner()).clone();
    craft_proof::hunspell::Search { extra_dir, ..craft_proof::hunspell::Search::for_app("WordCraft") }
}

/// Directories that may hold Hunspell dictionaries, most specific first. `lo_exts`, when a
/// language is being searched for, adds LibreOffice's extension folders for it (`dict-pt-BR`).
pub fn search_dirs(lo_exts: Option<&[String]>) -> Vec<PathBuf> {
    search().dirs(lo_exts)
}

/// The first `<name>.aff`/`<name>.dic` pair found for any of `names` (`pl_PL`, `pl`).
pub fn find(names: &[&str]) -> Option<(PathBuf, PathBuf)> {
    search().find(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn wordcraft_names_its_folder_and_variable() {
        let s = search();
        assert_eq!(s.env_var.as_deref(), Some("WORDCRAFT_DICTIONARIES"));
        assert_eq!(s.app_dir.as_deref(), Some("WordCraft"));
    }

    #[test]
    fn the_folder_setting_is_searched_first() {
        let dir = std::env::temp_dir().join(format!("wordcraft-hunspell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("xx_XX.aff"), "SET UTF-8\nSFX A Y 1\nSFX A 0 y .\n").unwrap();
        std::fs::write(dir.join("xx_XX.dic"), "1\nkot/A\n").unwrap();
        set_dictionary_dir(Some(dir.clone()));
        assert_eq!(search_dirs(None).first(), Some(&dir));
        let (aff, dic) = find(&["xx_XX"]).unwrap();
        assert!(load(&aff, &dic).unwrap().check("koty"));
        assert!(find_in(Path::new(&dir), &["yy"]).is_none());
        set_dictionary_dir(None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
