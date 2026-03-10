use std::{
    collections::HashMap,
    fs::{self, DirEntry},
    path,
};

use super::{Pokemon, PokemonTeam};

/// Return all file names in the current directory that look like team files
/// (contain `"team"` and end with `".txt"`).
fn get_team_files() -> Vec<String> {
    fs::read_dir(path::Path::new("."))
        .expect("Failed to read current directory")
        .collect::<Vec<Result<DirEntry, std::io::Error>>>()
        .into_iter()
        .map(|res| res.unwrap())
        .filter(|entry| {
            let name = entry.file_name().into_string().unwrap_or_default();
            entry.path().is_file() && name.contains("team") && name.ends_with(".txt")
        })
        .map(|entry| entry.file_name().into_string().unwrap())
        .collect()
}

/// Parse a single team file line into a [`Pokemon`].
///
/// Format: `species` or `species:nickname` (nickname may contain colons).
fn parse_line(line: &str) -> Option<Pokemon> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.splitn(2, ':');
    let name = parts.next().unwrap_or("").to_string();
    if name.is_empty() {
        return None;
    }
    let nickname = parts.next().map(|s| s.to_string());
    Some(Pokemon { name, nickname })
}

/// Read every team `.txt` file in the current directory and return a map of
/// `stem → PokemonTeam` (e.g. `"team"`, `"team_pete"`).
pub fn read_team_files() -> Result<HashMap<String, PokemonTeam>, std::io::Error> {
    let files = get_team_files();
    let mut teams = HashMap::new();

    for file in files {
        let content = fs::read_to_string(&file)?;
        let slots: Vec<Pokemon> = content.lines().filter_map(parse_line).collect();

        let stem = file.split('.').next().unwrap_or(&file).to_string();

        teams.insert(stem, PokemonTeam::from_slots(slots));
    }

    Ok(teams)
}
