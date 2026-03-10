mod files;

pub use files::read_team_files;

use serde::{Deserialize, Serialize};

/// A single Pokemon slot in a team. `name` is the species name (e.g. `"grotle"`);
/// `nickname` is `None` when the slot is empty or has no nickname.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pokemon {
    pub name: String,
    pub nickname: Option<String>,
}

/// A full six-slot party. Slots with no Pokemon have an empty `name`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PokemonTeam {
    pub pokemon: Vec<Pokemon>,
}

impl PokemonTeam {
    /// Build a team from a raw slot list, padding to exactly 6 entries.
    pub fn from_slots(mut slots: Vec<Pokemon>) -> Self {
        slots.truncate(6);
        while slots.len() < 6 {
            slots.push(Pokemon {
                name: String::new(),
                nickname: None,
            });
        }
        Self { pokemon: slots }
    }
}
