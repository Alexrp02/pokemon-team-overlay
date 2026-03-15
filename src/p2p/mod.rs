mod connection;
mod error;
mod protocol;
mod ticket;
mod ui;

pub use error::P2pError;
pub use ui::{build_connection_ui, TeamUrl, UiAction, UiBridge};

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::state::AppState;

pub async fn start(
    state: Arc<AppState>,
    ui: UiBridge,
    action_rx: mpsc::Receiver<UiAction>,
) -> Result<(), P2pError> {
    connection::run(state, ui, action_rx).await
}
