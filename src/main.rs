mod p2p;
mod savefile;
mod state;
mod team;
mod utils;

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::{header, Response, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{SinkExt, StreamExt};
use notify::{Event, RecursiveMode, Watcher};
use rust_embed::RustEmbed;
use std::{
    collections::HashMap,
    env, fs,
    path::{self, PathBuf},
    sync::Arc,
};
use tokio::sync::broadcast;
use tower_http::{cors::CorsLayer, services::ServeDir};

use serde::Serialize;
use state::{AppState, TeamSource, SAVE_FILE_TEAM_KEY};
use team::{read_team_files, PokemonTeam};

// ── Embedded static assets ────────────────────────────────────────────────────

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

// ── Constants ─────────────────────────────────────────────────────────────────

const TEAM_FILE: &str = "team.txt";
const SPRITES_DIR: &str = "sprites";
const STATIC_DIR: &str = "static";
const SAVE_FILE_CACHE_DIR: &str = "pokemon-team-display";
const SAVE_FILE_CACHE_FILE: &str = "last-save-path.txt";

// ── Application state ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct WsPayload {
    teams: HashMap<String, PokemonTeam>,
    remote: HashMap<String, PokemonTeam>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let source = choose_team_source();

    // Ensure required directories exist.
    fs::create_dir_all(SPRITES_DIR).expect("Failed to create sprites directory");
    fs::create_dir_all(STATIC_DIR).expect("Failed to create static directory");

    // When using text files, seed a default team file if none is present.
    if let TeamSource::TextFiles = &source {
        if !path::Path::new(TEAM_FILE).exists() {
            let default_team = "pikachu\ncharizard\nblastoise\nvenusaur\nmewtwo\ndragonite\n";
            fs::write(TEAM_FILE, default_team).expect("Failed to create team file");
        }
    }

    let state = Arc::new(AppState::new(source));

    // Spawn the file watcher appropriate for the chosen source.
    let tx_watcher = state.tx.clone();
    match &state.source {
        TeamSource::TextFiles => {
            tokio::spawn(async move {
                if let Err(e) = watch_team_files(tx_watcher).await {
                    eprintln!("File watcher error: {}", e);
                }
            });
        }
        TeamSource::SaveFile(ref path) => {
            let path = path.clone();
            tokio::spawn(async move {
                if let Err(e) = watch_save_file(path, tx_watcher).await {
                    eprintln!("Save file watcher error: {}", e);
                }
            });
        }
    }

    let p2p_state = Arc::clone(&state);
    tokio::spawn(async move {
        if let Err(e) = p2p::start(p2p_state).await {
            eprintln!("P2P error: {}", e);
        }
    });

    let app = Router::new()
        .route("/ws", get(websocket_handler))
        .nest_service("/sprites", ServeDir::new(SPRITES_DIR))
        .route("/", get(embedded_index))
        .route("/remote", get(embedded_index))
        .route("/remote/*path", get(embedded_remote))
        .route("/*path", get(embedded_static))
        .layer(CorsLayer::permissive())
        .with_state(Arc::clone(&state));

    let port = env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .expect("Failed to bind to port 3000");

    match &state.source {
        TeamSource::TextFiles => {
            println!("Server running on http://0.0.0.0:3000");
            println!("Edit '{}' to update your Pokemon team", TEAM_FILE);
            println!("  - Additional files whose name contains 'team' are also picked up.");
            println!("  - Use '?team=<name>' in the URL to switch between teams.");
        }
        TeamSource::SaveFile(path) => {
            println!("Server running on http://0.0.0.0:3000");
            println!("Reading party from save file: {}", path);
        }
    }
    println!("Place Pokemon sprites in the '{}' directory", SPRITES_DIR);

    axum::serve(listener, app)
        .await
        .expect("Failed to start server");
}

// ── Save-file selection ──────────────────────────────────────────────────────

/// Choose the team source via native file picker with save-path caching.
///
/// If the user cancels the picker and no cached save file exists, we fall back
/// to text-file mode to preserve previous non-savefile behavior.
fn choose_team_source() -> TeamSource {
    match choose_save_file_with_cache() {
        Some(path) => TeamSource::SaveFile(path),
        None => {
            eprintln!("No .sav file selected; falling back to text-file mode.");
            TeamSource::TextFiles
        }
    }
}

fn choose_save_file_with_cache() -> Option<String> {
    let cached_path = read_cached_save_path().filter(|path| path.exists());

    let mut dialog = rfd::FileDialog::new().add_filter("Pokemon save file", &["sav"]);
    if let Some(path) = cached_path.as_ref() {
        if let Some(parent) = path.parent() {
            dialog = dialog.set_directory(parent);
        }
        if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
            dialog = dialog.set_file_name(file_name);
        }
    }

    let selected = dialog.pick_file().or(cached_path);
    let selected = selected?;
    persist_cached_save_path(&selected);
    Some(selected.to_string_lossy().into_owned())
}

fn read_cached_save_path() -> Option<PathBuf> {
    let cache_file = save_file_cache_path()?;
    let raw = fs::read_to_string(cache_file).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

fn persist_cached_save_path(path: &std::path::Path) {
    let Some(cache_file) = save_file_cache_path() else {
        return;
    };
    let Some(parent) = cache_file.parent() else {
        return;
    };
    if let Err(err) = fs::create_dir_all(parent) {
        eprintln!("Failed to create save-path cache directory: {}", err);
        return;
    }
    if let Err(err) = fs::write(cache_file, path.to_string_lossy().as_ref()) {
        eprintln!("Failed to persist save-path cache: {}", err);
    }
}

fn save_file_cache_path() -> Option<PathBuf> {
    let config_dir = dirs::config_dir()?;
    Some(
        config_dir
            .join(SAVE_FILE_CACHE_DIR)
            .join(SAVE_FILE_CACHE_FILE),
    )
}

// ── Static asset handler ──────────────────────────────────────────────────────

async fn embedded_index() -> Response<Body> {
    asset_response("index.html")
}

async fn embedded_static(Path(path): Path<String>) -> Response<Body> {
    let path = if path.is_empty() {
        "index.html"
    } else {
        path.as_str()
    };
    asset_response(path)
}

async fn embedded_remote(Path(path): Path<String>) -> Response<Body> {
    let path = if path.is_empty() {
        "index.html"
    } else {
        path.as_str()
    };
    asset_response(path)
}

fn asset_response(path: &str) -> Response<Body> {
    match Assets::get(path) {
        Some(file) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, utils::content_type(path))
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(file.data))
            .unwrap(),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404"))
            .unwrap(),
    }
}

async fn send_payload(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    teams: &HashMap<String, PokemonTeam>,
    remote: &HashMap<String, PokemonTeam>,
) -> Result<(), ()> {
    let payload = WsPayload {
        teams: teams.clone(),
        remote: remote.clone(),
    };
    match serde_json::to_string(&payload) {
        Ok(json) => {
            if sender.send(Message::Text(json)).await.is_err() {
                Err(())
            } else {
                Ok(())
            }
        }
        Err(err) => {
            eprintln!("Failed to serialize websocket payload: {}", err);
            Err(())
        }
    }
}

// ── WebSocket ─────────────────────────────────────────────────────────────────

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, _receiver) = socket.split();
    let mut rx_local = state.tx.subscribe();
    let mut rx_remote = state.tx_remote.subscribe();

    // Send the current team state immediately on connect.
    // (The broadcast channel drops past messages for new subscribers, so we
    //  re-read directly from the source here.)
    let mut current_local = match state.source.read() {
        Ok(team) => team,
        Err(err) => {
            eprintln!("Failed to read local teams: {}", err);
            HashMap::new()
        }
    };
    let mut current_remote = state.remote.read().await.clone();

    if send_payload(&mut sender, &current_local, &current_remote)
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            local_update = rx_local.recv() => {
                match local_update {
                    Ok(team) => current_local = team,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        eprintln!("Local team updates lagged; skipped {} updates", skipped);
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            remote_update = rx_remote.recv() => {
                match remote_update {
                    Ok(team) => current_remote = team,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        eprintln!("Remote team updates lagged; skipped {} updates", skipped);
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }

        if send_payload(&mut sender, &current_local, &current_remote)
            .await
            .is_err()
        {
            break;
        }
    }
}

// ── File watchers ─────────────────────────────────────────────────────────────

/// Watch all `*team*.txt` files in the current directory for changes and
/// broadcast updated team data on each change (with a 300 ms debounce).
async fn watch_team_files(
    tx: broadcast::Sender<HashMap<String, PokemonTeam>>,
) -> notify::Result<()> {
    use notify::{Config, EventKind};

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel(100);

    let config = Config::default().with_poll_interval(std::time::Duration::from_secs(1));
    let mut watcher = notify::RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = notify_tx.blocking_send(event);
            }
        },
        config,
    )?;

    watcher.watch(path::Path::new("."), RecursiveMode::NonRecursive)?;

    // Broadcast the initial state.
    if let Ok(team) = read_team_files() {
        let _ = tx.send(team);
    }

    let debounce_duration = std::time::Duration::from_millis(300);
    let mut debounce_deadline: Option<tokio::time::Instant> = None;

    loop {
        let timeout = match debounce_deadline {
            Some(deadline) => {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    debounce_deadline = None;
                    if let Ok(team) = read_team_files() {
                        let _ = tx.send(team);
                    }
                    continue;
                }
                deadline - now
            }
            None => std::time::Duration::from_secs(3600),
        };

        tokio::select! {
            event_result = notify_rx.recv() => {
                match event_result {
                    Some(event) => {
                        let is_team_file = event.paths.iter().any(|p| {
                            p.file_name().map_or(false, |name| {
                                let name = name.to_string_lossy();
                                name.contains("team")
                                    && name.ends_with(".txt")
                                    && !name.ends_with('~')
                                    && !name.ends_with(".swp")
                                    && !name.ends_with(".tmp")
                            })
                        });

                        if !is_team_file {
                            continue;
                        }

                        match event.kind {
                            EventKind::Modify(_)
                            | EventKind::Create(_)
                            | EventKind::Remove(_)
                            | EventKind::Any => {
                                debounce_deadline =
                                    Some(tokio::time::Instant::now() + debounce_duration);
                            }
                            _ => {}
                        }
                    }
                    None => {
                        eprintln!("File watcher channel closed");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(timeout) => {
                // Debounce timer expired; handled at the top of the next iteration.
                continue;
            }
        }
    }

    drop(watcher);
    Ok(())
}

/// Watch a single save file for changes and broadcast the party on each change
/// (with a 300 ms debounce).
async fn watch_save_file(
    save_path: String,
    tx: broadcast::Sender<HashMap<String, PokemonTeam>>,
) -> notify::Result<()> {
    use notify::{Config, EventKind};

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel(100);

    let config = Config::default().with_poll_interval(std::time::Duration::from_secs(1));
    let mut watcher = notify::RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = notify_tx.blocking_send(event);
            }
        },
        config,
    )?;

    // Watch the directory containing the save file so we catch atomic writes
    // (where the editor replaces the file rather than modifying it in-place).
    let watch_dir = path::Path::new(&save_path)
        .parent()
        .unwrap_or(path::Path::new("."));
    watcher.watch(watch_dir, RecursiveMode::NonRecursive)?;

    // Broadcast the initial party.
    broadcast_save(&save_path, &tx);

    let debounce_duration = std::time::Duration::from_millis(300);
    let mut debounce_deadline: Option<tokio::time::Instant> = None;
    let save_filename = path::Path::new(&save_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    loop {
        let timeout = match debounce_deadline {
            Some(deadline) => {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    debounce_deadline = None;
                    broadcast_save(&save_path, &tx);
                    continue;
                }
                deadline - now
            }
            None => std::time::Duration::from_secs(3600),
        };

        tokio::select! {
            event_result = notify_rx.recv() => {
                match event_result {
                    Some(event) => {
                        let is_save_file = event.paths.iter().any(|p| {
                            p.file_name()
                                .map(|n| n.to_string_lossy() == save_filename)
                                .unwrap_or(false)
                        });

                        if !is_save_file {
                            continue;
                        }

                        match event.kind {
                            EventKind::Modify(_)
                            | EventKind::Create(_)
                            | EventKind::Remove(_)
                            | EventKind::Any => {
                                debounce_deadline =
                                    Some(tokio::time::Instant::now() + debounce_duration);
                            }
                            _ => {}
                        }
                    }
                    None => {
                        eprintln!("File watcher channel closed");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(timeout) => {
                continue;
            }
        }
    }

    drop(watcher);
    Ok(())
}

/// Read the save file at `path`, parse the party, and broadcast it.
/// Errors are printed to stderr and silently ignored so the watcher keeps running.
fn broadcast_save(path: &str, tx: &broadcast::Sender<HashMap<String, PokemonTeam>>) {
    match fs::read(path) {
        Ok(data) => match savefile::read_party(&data) {
            Ok(slots) => {
                let mut map = HashMap::new();
                map.insert(
                    SAVE_FILE_TEAM_KEY.to_string(),
                    PokemonTeam::from_slots(slots),
                );
                let _ = tx.send(map);
            }
            Err(e) => eprintln!("Failed to parse save file: {}", e),
        },
        Err(e) => eprintln!("Failed to read save file '{}': {}", path, e),
    }
}
