use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{collections::HashMap, sync::Arc, time::Duration};

use iroh::endpoint::Connection;
use iroh::Endpoint;
use tokio::sync::{mpsc, Mutex};

use crate::source_picker;
use crate::state::{AppState, TeamSource};
use crate::team::PokemonTeam;

use super::error::P2pError;
use super::protocol::{TeamsMessage, ALPN};
use super::ticket::{create_ticket, parse_ticket};
use super::ui::{TeamUrl, UiAction, UiBridge};

const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

struct ActiveConnection {
    id: u64,
    conn: Connection,
}

pub async fn run(
    state: Arc<AppState>,
    ui: UiBridge,
    ticket_rx: mpsc::Receiver<UiAction>,
) -> Result<(), P2pError> {
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
    ui.set_local_ticket(ticket.clone());
    ui.set_status("Waiting for peer ticket or inbound connection...".to_string());
    ui.set_source_mode(source_mode_label(&state).await);
    refresh_ui_urls(&state, &ui).await;

    println!();
    println!("P2P ticket (share this with your friend):");
    println!("{}", ticket);
    println!();
    println!("Use the P2P window to paste a friend's ticket and connect.");
    println!();

    let active = Arc::new(Mutex::new(None::<ActiveConnection>));
    let next_id = Arc::new(AtomicU64::new(1));

    let accept_state = Arc::clone(&state);
    let accept_active = Arc::clone(&active);
    let accept_endpoint = endpoint.clone();
    let accept_next_id = Arc::clone(&next_id);
    let accept_ui = ui.clone();
    tokio::spawn(accept_loop(
        accept_endpoint,
        accept_state,
        accept_active,
        accept_next_id,
        accept_ui,
    ));

    let connect_state = Arc::clone(&state);
    let connect_active = Arc::clone(&active);
    let connect_endpoint = endpoint.clone();
    let connect_next_id = Arc::clone(&next_id);
    let connect_ui = ui.clone();
    tokio::spawn(connect_ticket_loop(
        connect_endpoint,
        connect_state,
        connect_active,
        connect_next_id,
        connect_ui,
        ticket_rx,
    ));

    let update_state = Arc::clone(&state);
    let update_active = Arc::clone(&active);
    let update_ui = ui;
    tokio::spawn(send_updates_loop(update_state, update_active, update_ui));

    std::future::pending::<()>().await;
    Ok(())
}

async fn accept_loop(
    endpoint: Endpoint,
    state: Arc<AppState>,
    active: Arc<Mutex<Option<ActiveConnection>>>,
    next_id: Arc<AtomicU64>,
    ui: UiBridge,
) {
    loop {
        let accepting = match endpoint.accept().await {
            Some(accepting) => accepting,
            None => break,
        };
        match accepting.await {
            Ok(conn) => {
                if let Err(err) = handle_connection(
                    conn,
                    state.clone(),
                    active.clone(),
                    next_id.clone(),
                    ui.clone(),
                )
                .await
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

async fn connect_ticket_loop(
    endpoint: Endpoint,
    state: Arc<AppState>,
    active: Arc<Mutex<Option<ActiveConnection>>>,
    next_id: Arc<AtomicU64>,
    ui: UiBridge,
    mut actions: mpsc::Receiver<UiAction>,
) {
    while let Some(action) = actions.recv().await {
        match action {
            UiAction::ConnectTicket(ticket) => {
                ui.set_status("Connecting to peer...".to_string());
                if let Err(err) = connect_with_ticket(
                    &endpoint,
                    &ticket,
                    state.clone(),
                    active.clone(),
                    next_id.clone(),
                    ui.clone(),
                )
                .await
                {
                    eprintln!("P2P connect error: {}", err);
                    ui.set_status(format!("Connect failed: {}", err));
                }
            }
            UiAction::ToggleSourceMode => {
                switch_source_mode(state.clone(), &ui).await;
                ui.set_source_mode(source_mode_label(&state).await);
                refresh_ui_urls(&state, &ui).await;
            }
            UiAction::SelectSaveFile => {
                select_save_file(state.clone(), &ui).await;
                ui.set_source_mode(source_mode_label(&state).await);
                refresh_ui_urls(&state, &ui).await;
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
    ui: UiBridge,
) -> Result<(), P2pError> {
    let addr = parse_ticket(ticket)?;
    let conn = endpoint.connect(addr, ALPN).await?;
    handle_connection(conn, state, active, next_id, ui).await
}

async fn handle_connection(
    conn: Connection,
    state: Arc<AppState>,
    active: Arc<Mutex<Option<ActiveConnection>>>,
    next_id: Arc<AtomicU64>,
    ui: UiBridge,
) -> Result<(), P2pError> {
    let connection_id = next_id.fetch_add(1, Ordering::Relaxed);
    let replaced = replace_active_connection(&active, connection_id, &conn).await;

    if let Some(existing) = replaced {
        existing
            .conn
            .close(0u8.into(), b"replaced by new connection");
        clear_remote(state.clone()).await;
        refresh_ui_urls(&state, &ui).await;
        println!("P2P connection replaced. Waiting for new teams...");
    }

    let local_source = state.source.read().await.clone();
    if let Ok(local) = local_source.read() {
        if let Err(err) = send_teams(&conn, &local).await {
            eprintln!("P2P initial send error: {}", err);
        }
    }

    let recv_state = state.clone();
    let recv_active = active.clone();
    let recv_conn = conn.clone();
    let recv_id = connection_id;
    let recv_ui = ui.clone();
    tokio::spawn(async move {
        if let Err(err) = receive_loop(recv_conn, recv_state.clone(), recv_ui.clone()).await {
            eprintln!("P2P receive error: {}", err);
        }
        reset_connection(
            recv_state,
            recv_active,
            recv_ui,
            "receive loop ended",
            recv_id,
        )
        .await;
    });

    let closed_state = state.clone();
    let closed_active = active.clone();
    let closed_conn = conn;
    let closed_id = connection_id;
    let closed_ui = ui.clone();
    tokio::spawn(async move {
        closed_conn.closed().await;
        reset_connection(
            closed_state,
            closed_active,
            closed_ui,
            "connection closed",
            closed_id,
        )
        .await;
    });

    ui.set_status("Connected. Waiting for team updates...".to_string());
    refresh_ui_urls(&state, &ui).await;

    Ok(())
}

async fn receive_loop(
    conn: Connection,
    state: Arc<AppState>,
    ui: UiBridge,
) -> Result<(), P2pError> {
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
        update_remote(state.clone(), message.teams, ui.clone()).await;
    }
    Ok(())
}

async fn send_updates_loop(
    state: Arc<AppState>,
    active: Arc<Mutex<Option<ActiveConnection>>>,
    ui: UiBridge,
) {
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

        refresh_ui_urls(&state, &ui).await;
        if let Some((conn_id, conn)) = active_connection_snapshot(&active).await {
            if let Err(err) = send_teams(&conn, &teams).await {
                eprintln!("P2P send error: {}", err);
                reset_connection(
                    state.clone(),
                    active.clone(),
                    ui.clone(),
                    "send failed",
                    conn_id,
                )
                .await;
            }
        }
    }
}

async fn send_teams(
    conn: &Connection,
    teams: &HashMap<String, PokemonTeam>,
) -> Result<(), P2pError> {
    let message = TeamsMessage {
        teams: teams.clone(),
    };
    let payload = serde_json::to_vec(&message)?;
    let mut send = conn.open_uni().await.map_err(P2pError::OpenUni)?;
    send.write_all(&payload).await.map_err(P2pError::Write)?;
    send.finish().map_err(P2pError::Finish)?;
    Ok(())
}

async fn update_remote(state: Arc<AppState>, teams: HashMap<String, PokemonTeam>, ui: UiBridge) {
    let should_log;
    {
        let mut remote = state.remote.write().await;
        let previous: std::collections::HashSet<String> = remote.keys().cloned().collect();
        let next: std::collections::HashSet<String> = teams.keys().cloned().collect();
        should_log = previous != next;
        *remote = teams;
    }
    let teams_snapshot = {
        let remote = state.remote.read().await;
        remote.clone()
    };
    let _ = state.tx_remote.send(teams_snapshot.clone());
    refresh_ui_urls(&state, &ui).await;
    if should_log {
        let port = current_port();
        log_remote_urls(&teams_snapshot, &port);
        let local_source = state.source.read().await.clone();
        if let Ok(teams) = local_source.read() {
            log_local_urls(&teams, &port)
        }
    }
}

async fn reset_connection(
    state: Arc<AppState>,
    active: Arc<Mutex<Option<ActiveConnection>>>,
    ui: UiBridge,
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

    clear_remote(state.clone()).await;
    refresh_ui_urls(&state, &ui).await;
    ui.set_status("Disconnected. Waiting for peer ticket or inbound connection...".to_string());
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

async fn active_connection_snapshot(
    active: &Arc<Mutex<Option<ActiveConnection>>>,
) -> Option<(u64, Connection)> {
    let guard = active.lock().await;
    guard
        .as_ref()
        .map(|active| (active.id, active.conn.clone()))
}

async fn replace_active_connection(
    active: &Arc<Mutex<Option<ActiveConnection>>>,
    connection_id: u64,
    conn: &Connection,
) -> Option<ActiveConnection> {
    let mut guard = active.lock().await;
    let existing = guard.take();
    *guard = Some(ActiveConnection {
        id: connection_id,
        conn: conn.clone(),
    });
    existing
}

fn current_port() -> String {
    env::var("PORT").unwrap_or_else(|_| "3000".to_string())
}

fn log_remote_urls(teams: &HashMap<String, PokemonTeam>, port: &str) {
    if teams.is_empty() {
        return;
    }
    log_team_urls("Remote teams available:", "remote", port, teams);
}

fn log_local_urls(teams: &HashMap<String, PokemonTeam>, port: &str) {
    if teams.is_empty() {
        return;
    }
    log_team_urls("Local teams available:", "", port, teams);
}

fn log_team_urls(title: &str, path: &str, port: &str, teams: &HashMap<String, PokemonTeam>) {
    println!("{}", title);
    for row in team_url_rows(path, port, teams) {
        println!("  - {}", row.url);
    }
    println!();
}

async fn refresh_ui_urls(state: &Arc<AppState>, ui: &UiBridge) {
    let local_source = state.source.read().await.clone();
    let local = match local_source.read() {
        Ok(teams) => teams,
        Err(err) => {
            eprintln!("Failed to read local teams for UI: {}", err);
            HashMap::new()
        }
    };
    let remote = state.remote.read().await.clone();
    let port = current_port();

    ui.set_local_urls(team_url_rows("", &port, &local));
    ui.set_remote_urls(team_url_rows("remote", &port, &remote));
    ui.set_source_mode(source_mode_label(state).await);
}

async fn source_mode_label(state: &Arc<AppState>) -> String {
    match state.source.read().await.clone() {
        TeamSource::TextFiles => "Team files (*.txt)".to_string(),
        TeamSource::SaveFile(path) => format!(".sav file ({})", path),
    }
}

async fn switch_source_mode(state: Arc<AppState>, ui: &UiBridge) {
    let current = state.source.read().await.clone();
    match current {
        TeamSource::TextFiles => {
            let cached = source_picker::load_cached_save_path().filter(|p| p.exists());
            let selected = if let Some(cached) = cached {
                if source_picker::validate_save_file(&cached).is_ok() {
                    Some(cached)
                } else {
                    source_picker::prompt_for_valid_save_file(Some(cached.as_path()))
                }
            } else {
                source_picker::prompt_for_valid_save_file(None)
            };

            if let Some(path) = selected {
                let mut source = state.source.write().await;
                *source = TeamSource::SaveFile(path.to_string_lossy().into_owned());
                ui.set_status("Switched to save-file mode.".to_string());
            } else {
                ui.set_status("Save-file selection cancelled. Staying in team-file mode.".to_string());
            }
        }
        TeamSource::SaveFile(_) => {
            let mut source = state.source.write().await;
            *source = TeamSource::TextFiles;
            ui.set_status("Switched to team-file mode.".to_string());
        }
    }
}

async fn select_save_file(state: Arc<AppState>, ui: &UiBridge) {
    let default = match state.source.read().await.clone() {
        TeamSource::SaveFile(path) => Some(std::path::PathBuf::from(path)),
        TeamSource::TextFiles => source_picker::load_cached_save_path(),
    };

    match source_picker::prompt_for_valid_save_file(default.as_deref()) {
        Some(path) => {
            let mut source = state.source.write().await;
            *source = TeamSource::SaveFile(path.to_string_lossy().into_owned());
            ui.set_status("Selected a new save file.".to_string());
        }
        None => {
            ui.set_status("Save-file selection cancelled.".to_string());
        }
    }
}

fn team_url_rows(path: &str, port: &str, teams: &HashMap<String, PokemonTeam>) -> Vec<TeamUrl> {
    let mut names: Vec<String> = teams.keys().cloned().collect();
    names.sort();

    let base = if path.is_empty() {
        format!("http://localhost:{}", port)
    } else {
        format!("http://localhost:{}/{}", port, path)
    };

    names
        .into_iter()
        .map(|team_name| TeamUrl {
            url: format!("{}?team={}", base, team_name),
            team_name,
        })
        .collect()
}
