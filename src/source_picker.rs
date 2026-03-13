use std::{
    fs,
    path::{Path, PathBuf},
};

const SAVE_FILE_CACHE_DIR: &str = "pokemon-team-display";
const SAVE_FILE_CACHE_FILE: &str = "last-save-path.txt";

pub fn load_cached_save_path() -> Option<PathBuf> {
    let cache_file = save_file_cache_path()?;
    let raw = fs::read_to_string(cache_file).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

pub fn persist_cached_save_path(path: &Path) {
    let Some(cache_file) = save_file_cache_path() else {
        return;
    };
    let Some(parent) = cache_file.parent() else {
        return;
    };
    if let Err(err) = fs::create_dir_all(parent) {
        eprintln!("Failed to create save-path cache directory: {}", err);
        return;
    }
    if let Err(err) = fs::write(cache_file, path.to_string_lossy().as_ref()) {
        eprintln!("Failed to persist save-path cache: {}", err);
    }
}

pub fn prompt_for_save_file(default_path: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter("Pokemon save file", &["sav"]);
    if let Some(path) = default_path {
        if let Some(parent) = path.parent() {
            dialog = dialog.set_directory(parent);
        }
        if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
            dialog = dialog.set_file_name(file_name);
        }
    }
    let selected = dialog.pick_file()?;
    persist_cached_save_path(&selected);
    Some(selected)
}

pub fn prompt_for_valid_save_file(default_path: Option<&Path>) -> Option<PathBuf> {
    let mut default = default_path.map(Path::to_path_buf);
    loop {
        let selected = prompt_for_save_file(default.as_deref())?;
        match validate_save_file(&selected) {
            Ok(()) => return Some(selected),
            Err(err) => {
                eprintln!("Failed to load selected save file '{}': {}", selected.display(), err);
                default = Some(selected);
            }
        }
    }
}

pub fn validate_save_file(path: &Path) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    crate::savefile::read_party(&data).map(|_| ())
}

fn save_file_cache_path() -> Option<PathBuf> {
    let config_dir = dirs::config_dir()?;
    Some(
        config_dir
            .join(SAVE_FILE_CACHE_DIR)
            .join(SAVE_FILE_CACHE_FILE),
    )
}
