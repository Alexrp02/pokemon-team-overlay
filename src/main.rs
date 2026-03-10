mod savefile;
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
use std::{collections::HashMap, fs, path, sync::Arc};
use tokio::sync::broadcast;
use tower_http::{cors::CorsLayer, services::ServeDir};

use team::{read_team_files, PokemonTeam};

// ── Embedded static assets ────────────────────────────────────────────────────

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

// ── Constants ─────────────────────────────────────────────────────────────────

const TEAM_FILE: &str = "team.txt";
const SPRITES_DIR: &str = "sprites";
const STATIC_DIR: &str = "static";

/// Key used in the team map when the source is a save file.
const SAVE_FILE_TEAM_KEY: &str = "team";

// ── Team source ───────────────────────────────────────────────────────────────

/// Describes where the overlay should obtain its team data.
enum TeamSource {
    /// Read one or more `*team*.txt` files from the current directory.
    TextFiles,
    /// Parse a HeartGold / SoulSilver save file at the given path.
    SaveFile(String),
}

impl TeamSource {
    /// Read the current team data from whichever source is configured.
    fn read(&self) -> Result<HashMap<String, PokemonTeam>, String> {
        match self {
            TeamSource::TextFiles => read_team_files().map_err(|e| e.to_string()),
            TeamSource::SaveFile(path) => {
                let data = fs::read(path).map_err(|e| e.to_string())?;
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

// ── Application state ─────────────────────────────────────────────────────────

struct AppState {
    tx: broadcast::Sender<HashMap<String, PokemonTeam>>,
    source: TeamSource,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let source = parse_args();

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

    let (tx, _) = broadcast::channel::<HashMap<String, PokemonTeam>>(100);
    let state = Arc::new(AppState {
        tx: tx.clone(),
        source,
    });

    // Spawn the file watcher appropriate for the chosen source.
    let tx_watcher = tx.clone();
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

    let app = Router::new()
        .route("/ws", get(websocket_handler))
        .nest_service("/sprites", ServeDir::new(SPRITES_DIR))
        .route(
            "/",
            get(|| async { embedded_static(Path("".into())).await }),
        )
        .route("/*path", get(embedded_static))
        .layer(CorsLayer::permissive())
        .with_state(Arc::clone(&state));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
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

// ── Argument parsing ──────────────────────────────────────────────────────────

/// Parse command-line arguments and return the appropriate [`TeamSource`].
///
/// Usage:
/// ```
/// pokemon-team-display                        # text-file mode
/// pokemon-team-display --save-file <path>     # save-file mode
/// ```
fn parse_args() -> TeamSource {
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--save-file" {
            if let Some(path) = args.get(i + 1) {
                return TeamSource::SaveFile(path.clone());
            } else {
                eprintln!("Error: --save-file requires a path argument");
                std::process::exit(1);
            }
        }
        i += 1;
    }
    TeamSource::TextFiles
}

// ── Static asset handler ──────────────────────────────────────────────────────

async fn embedded_static(Path(path): Path<String>) -> Response<Body> {
    let path = if path.is_empty() {
        "index.html"
    } else {
        path.as_str()
    };

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

// ── WebSocket ─────────────────────────────────────────────────────────────────

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, _receiver) = socket.split();
    let mut rx = state.tx.subscribe();

    // Send the current team state immediately on connect.
    // (The broadcast channel drops past messages for new subscribers, so we
    //  re-read directly from the source here.)
    if let Ok(team) = state.source.read() {
        let json = serde_json::to_string(&team).unwrap();
        if sender.send(Message::Text(json)).await.is_err() {
            return;
        }
    }

    while let Ok(team) = rx.recv().await {
        let json = serde_json::to_string(&team).unwrap();
        if sender.send(Message::Text(json)).await.is_err() {
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
