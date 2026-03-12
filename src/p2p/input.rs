use tokio::io::{self, AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use super::error::P2pError;

pub fn spawn_ticket_reader() -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(10);
    tokio::spawn(async move {
        let mut lines = BufReader::new(io::stdin()).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if tx.send(trimmed.to_string()).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    eprintln!("P2P input error: {}", P2pError::Input(err));
                    break;
                }
            }
        }
    });
    rx
}
