use std::env;
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
    println!("Paste a friend's ticket and press Enter to connect.");
    println!();

    let active = Arc::new(Mutex::new(None::<Connection>));

    let accept_state = Arc::clone(&state);
    let accept_active = Arc::clone(&active);
    let accept_endpoint = endpoint.clone();
    tokio::spawn(async move {
        accept_loop(accept_endpoint, accept_state, accept_active).await;
    });

    let mut ticket_rx = spawn_ticket_reader();
    let connect_state = Arc::clone(&state);
    let connect_active = Arc::clone(&active);
    let connect_endpoint = endpoint.clone();
    tokio::spawn(async move {
        while let Some(ticket) = ticket_rx.recv().await {
            if let Err(err) =
                connect_with_ticket(&connect_endpoint, &ticket, connect_state.clone(), connect_active.clone())
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

async fn accept_loop(endpoint: Endpoint, state: Arc<AppState>, active: Arc<Mutex<Option<Connection>>>) {
    loop {
        let accepting = match endpoint.accept().await {
            Some(accepting) => accepting,
            None => break,
        };
        match accepting.await {
            Ok(conn) => {
                if let Err(err) = handle_connection(conn, state.clone(), active.clone()).await {
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
    active: Arc<Mutex<Option<Connection>>>,
) -> Result<(), P2pError> {
    let addr = parse_ticket(ticket)?;
    let conn = endpoint.connect(addr, ALPN).await?;
    handle_connection(conn, state, active).await
}

async fn handle_connection(
    conn: Connection,
    state: Arc<AppState>,
    active: Arc<Mutex<Option<Connection>>>,
) -> Result<(), P2pError> {
    {
        let mut guard = active.lock().await;
        if guard.is_some() {
            conn.close(0u8.into(), b"connection already active");
            eprintln!("P2P connection rejected: already connected to a peer");
            return Ok(());
        }
        *guard = Some(conn.clone());
    }

    if let Ok(local) = state.source.read() {
        if let Err(err) = send_teams(&conn, &local).await {
            eprintln!("P2P initial send error: {}", err);
        }
    }

    let recv_state = state.clone();
    let recv_active = active.clone();
    tokio::spawn(async move {
        if let Err(err) = receive_loop(conn, recv_state).await {
            eprintln!("P2P receive error: {}", err);
        }
        let mut guard = recv_active.lock().await;
        guard.take();
    });

    Ok(())
}

async fn receive_loop(conn: Connection, state: Arc<AppState>) -> Result<(), P2pError> {
    loop {
        let mut recv = conn.accept_uni().await.map_err(P2pError::AcceptUni)?;
        let data = recv.read_to_end(MAX_MESSAGE_BYTES).await?;
        let message: TeamsMessage = serde_json::from_slice(&data)?;
        update_remote(state.clone(), message.teams).await;
    }
}

async fn send_updates_loop(state: Arc<AppState>, active: Arc<Mutex<Option<Connection>>>) {
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

        let conn = { active.lock().await.clone() };
        if let Some(conn) = conn {
            if let Err(err) = send_teams(&conn, &teams).await {
                eprintln!("P2P send error: {}", err);
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
