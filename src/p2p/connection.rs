use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{collections::HashMap, sync::Arc, time::Duration};

use iroh::endpoint::Connection;
use iroh::Endpoint;
use tokio::sync::Mutex;

use crate::state::AppState;
use crate::team::PokemonTeam;

use super::error::P2pError;
use super::input::spawn_ticket_reader;
use super::protocol::{TeamsMessage, ALPN};
use super::ticket::{create_ticket, parse_ticket};

const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

struct ActiveConnection {
    id: u64,
    conn: Connection,
}

pub async fn run(state: Arc<AppState>) -> Result<(), P2pError> {
    let endpoint = Endpoint::builder()
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;

    let online_timeout = Duration::from_secs(iroh::NET_REPORT_TIMEOUT);
    if tokio::time::timeout(online_timeout, endpoint.online())
        .await
        .is_err()
    {
        eprintln!(
            "P2P warning: {}",
            P2pError::EndpointOnlineTimeout(online_timeout)
        );
    }

    let ticket = create_ticket(endpoint.addr());
    println!();
    println!("P2P ticket (share this with your friend):");
    println!("{}", ticket);
    println!();
    println!("Paste a friend's ticket and press Enter to connect.");
    println!();

    let active = Arc::new(Mutex::new(None::<ActiveConnection>));
    let next_id = Arc::new(AtomicU64::new(1));

    let accept_state = Arc::clone(&state);
    let accept_active = Arc::clone(&active);
    let accept_endpoint = endpoint.clone();
    let accept_next_id = Arc::clone(&next_id);
    tokio::spawn(async move {
        accept_loop(accept_endpoint, accept_state, accept_active, accept_next_id).await;
    });

    let mut ticket_rx = spawn_ticket_reader();
    let connect_state = Arc::clone(&state);
    let connect_active = Arc::clone(&active);
    let connect_endpoint = endpoint.clone();
    let connect_next_id = Arc::clone(&next_id);
    tokio::spawn(async move {
        while let Some(ticket) = ticket_rx.recv().await {
            if let Err(err) =
                connect_with_ticket(
                    &connect_endpoint,
                    &ticket,
                    connect_state.clone(),
                    connect_active.clone(),
                    connect_next_id.clone(),
                )
                    .await
            {
                eprintln!("P2P connect error: {}", err);
            }
        }
    });

    let update_state = Arc::clone(&state);
    let update_active = Arc::clone(&active);
    tokio::spawn(async move {
        send_updates_loop(update_state, update_active).await;
    });

    std::future::pending::<()>().await;
    Ok(())
}

async fn accept_loop(
    endpoint: Endpoint,
    state: Arc<AppState>,
    active: Arc<Mutex<Option<ActiveConnection>>>,
    next_id: Arc<AtomicU64>,
) {
    loop {
        let accepting = match endpoint.accept().await {
            Some(accepting) => accepting,
            None => break,
        };
        match accepting.await {
            Ok(conn) => {
                if let Err(err) =
                    handle_connection(conn, state.clone(), active.clone(), next_id.clone()).await
                {
                    eprintln!("P2P connection error: {}", err);
                }
            }
            Err(err) => {
                eprintln!("P2P accept handshake error: {}", err);
            }
        }
    }
}

async fn connect_with_ticket(
    endpoint: &Endpoint,
    ticket: &str,
    state: Arc<AppState>,
    active: Arc<Mutex<Option<ActiveConnection>>>,
    next_id: Arc<AtomicU64>,
) -> Result<(), P2pError> {
    let addr = parse_ticket(ticket)?;
    let conn = endpoint.connect(addr, ALPN).await?;
    handle_connection(conn, state, active, next_id).await
}

async fn handle_connection(
    conn: Connection,
    state: Arc<AppState>,
    active: Arc<Mutex<Option<ActiveConnection>>>,
    next_id: Arc<AtomicU64>,
) -> Result<(), P2pError> {
    let connection_id = next_id.fetch_add(1, Ordering::Relaxed);
    let replaced = {
        let mut guard = active.lock().await;
        let existing = guard.take();
        *guard = Some(ActiveConnection {
            id: connection_id,
            conn: conn.clone(),
        });
        existing
    };

    if let Some(existing) = replaced {
        existing.conn.close(0u8.into(), b"replaced by new connection");
        clear_remote(state.clone()).await;
        println!("P2P connection replaced. Waiting for new teams...");
    }

    if let Ok(local) = state.source.read() {
        if let Err(err) = send_teams(&conn, &local).await {
            eprintln!("P2P initial send error: {}", err);
        }
    }

    let recv_state = state.clone();
    let recv_active = active.clone();
    let closed_state = state.clone();
    let closed_active = active.clone();
    let closed_conn = conn.clone();
    let recv_id = connection_id;
    tokio::spawn(async move {
        if let Err(err) = receive_loop(conn, recv_state.clone()).await {
            eprintln!("P2P receive error: {}", err);
        }
        reset_connection(recv_state, recv_active, "receive loop ended", recv_id).await;
    });

    let closed_id = connection_id;
    tokio::spawn(async move {
        closed_conn.closed().await;
        reset_connection(closed_state, closed_active, "connection closed", closed_id).await;
    });

    Ok(())
}

async fn receive_loop(conn: Connection, state: Arc<AppState>) -> Result<(), P2pError> {
    loop {
        let mut recv = match conn.accept_uni().await {
            Ok(recv) => recv,
            Err(err) => {
                eprintln!("P2P connection closed: {}", err);
                break;
            }
        };
        let data = recv.read_to_end(MAX_MESSAGE_BYTES).await?;
        let message: TeamsMessage = serde_json::from_slice(&data)?;
        update_remote(state.clone(), message.teams).await;
    }
    Ok(())
}

async fn send_updates_loop(state: Arc<AppState>, active: Arc<Mutex<Option<ActiveConnection>>>) {
    let mut rx = state.tx.subscribe();
    loop {
        let teams = match rx.recv().await {
            Ok(teams) => teams,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                eprintln!("Local team updates lagged; skipped {} updates", skipped);
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };

        let current = {
            let guard = active.lock().await;
            guard.as_ref().map(|active| (active.id, active.conn.clone()))
        };
        if let Some((conn_id, conn)) = current {
            if let Err(err) = send_teams(&conn, &teams).await {
                eprintln!("P2P send error: {}", err);
                reset_connection(state.clone(), active.clone(), "send failed", conn_id).await;
            }
        }
    }
}

async fn send_teams(conn: &Connection, teams: &HashMap<String, PokemonTeam>) -> Result<(), P2pError> {
    let message = TeamsMessage {
        teams: teams.clone(),
    };
    let payload = serde_json::to_vec(&message)?;
    let mut send = conn.open_uni().await.map_err(P2pError::OpenUni)?;
    send.write_all(&payload).await.map_err(P2pError::Write)?;
    send.finish().map_err(P2pError::Finish)?;
    Ok(())
}

async fn update_remote(state: Arc<AppState>, teams: HashMap<String, PokemonTeam>) {
    let should_log;
    {
        let mut remote = state.remote.write().await;
        let previous: std::collections::HashSet<String> = remote.keys().cloned().collect();
        let next: std::collections::HashSet<String> = teams.keys().cloned().collect();
        should_log = previous != next;
        *remote = teams.clone();
    }
    let _ = state.tx_remote.send(teams.clone());
    if should_log {
        log_remote_urls(&teams);
        if let Ok(teams) = &state.source.read() {
            log_local_urls(teams)
        }
    }
}

async fn reset_connection(
    state: Arc<AppState>,
    active: Arc<Mutex<Option<ActiveConnection>>>,
    reason: &str,
    connection_id: u64,
) {
    let was_active = {
        let mut guard = active.lock().await;
        match guard.as_ref() {
            Some(active_conn) if active_conn.id == connection_id => {
                guard.take();
                true
            }
            _ => false,
        }
    };
    if !was_active {
        return;
    }

    clear_remote(state).await;
    println!(
        "P2P connection ended ({}). Waiting for a new ticket or inbound connection...",
        reason
    );
}

async fn clear_remote(state: Arc<AppState>) {
    let mut remote = state.remote.write().await;
    if remote.is_empty() {
        return;
    }
    remote.clear();
    let _ = state.tx_remote.send(HashMap::new());
}

fn log_remote_urls(teams: &HashMap<String, PokemonTeam>) {
    if teams.is_empty() {
        return;
    }
    println!("Remote teams available:");
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    for name in teams.keys() {
        println!("  - http://localhost:{}/remote?team={}", port, name);
    }
    println!();
}

fn log_local_urls(teams: &HashMap<String, PokemonTeam>) {
    if teams.is_empty() {
        return;
    }
    println!("Local teams available:");
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    for name in teams.keys() {
        println!("  - http://localhost:{}?team={}", port, name);
    }
    println!();
}
