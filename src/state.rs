use std::collections::HashMap;

use tokio::sync::{broadcast, RwLock};

use crate::{savefile, team::read_team_files, team::PokemonTeam};

/// Key used in the team map when the source is a save file.
pub const SAVE_FILE_TEAM_KEY: &str = "team";

/// Describes where the overlay should obtain its team data.
#[derive(Clone)]
pub enum TeamSource {
    /// Read one or more `*team*.txt` files from the current directory.
    TextFiles,
    /// Parse a HeartGold / SoulSilver save file at the given path.
    SaveFile(String),
}

impl TeamSource {
    /// Read the current team data from whichever source is configured.
    pub fn read(&self) -> Result<HashMap<String, PokemonTeam>, String> {
        match self {
            TeamSource::TextFiles => read_team_files().map_err(|e| e.to_string()),
            TeamSource::SaveFile(path) => {
                let data = std::fs::read(path).map_err(|e| e.to_string())?;
                let slots = savefile::read_party(&data)?;
                let mut map = HashMap::new();
                map.insert(
                    SAVE_FILE_TEAM_KEY.to_string(),
                    PokemonTeam::from_slots(slots),
                );
                Ok(map)
            }
        }
    }
}

pub struct AppState {
    pub tx: broadcast::Sender<HashMap<String, PokemonTeam>>,
    pub tx_remote: broadcast::Sender<HashMap<String, PokemonTeam>>,
    pub source: TeamSource,
    pub remote: RwLock<HashMap<String, PokemonTeam>>,
}

impl AppState {
    pub fn new(source: TeamSource) -> Self {
        let (tx, _) = broadcast::channel::<HashMap<String, PokemonTeam>>(100);
        let (tx_remote, _) = broadcast::channel::<HashMap<String, PokemonTeam>>(100);
        Self {
            tx,
            tx_remote,
            source,
            remote: RwLock::new(HashMap::new()),
        }
    }
}
