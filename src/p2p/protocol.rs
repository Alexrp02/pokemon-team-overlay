use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::team::PokemonTeam;

pub const ALPN: &[u8] = b"pokemon-team-overlay/1";

#[derive(Debug, Serialize, Deserialize)]
pub struct TeamsMessage {
    pub teams: HashMap<String, PokemonTeam>,
}
