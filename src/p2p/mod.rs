mod connection;
mod error;
mod input;
mod protocol;
mod ticket;

pub use error::P2pError;

use std::sync::Arc;

use crate::state::AppState;

pub async fn start(state: Arc<AppState>) -> Result<(), P2pError> {
    connection::run(state).await
}
