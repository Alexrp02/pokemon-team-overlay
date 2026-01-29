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
use serde::{Deserialize, Serialize};
use std::{fs, path, sync::Arc};
use tokio::sync::broadcast;
use tower_http::{cors::CorsLayer, services::ServeDir};

// --------------------
// Pack static assets into the binary
#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;
// --------------------

const TEAM_FILE: &str = "team.txt";
const SPRITES_DIR: &str = "sprites";
const STATIC_DIR: &str = "static";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pokemon {
    name: String,
    nickname: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PokemonTeam {
    pokemon: Vec<Pokemon>,
}

struct AppState {
    tx: broadcast::Sender<PokemonTeam>,
}

#[tokio::main]
async fn main() {
    // Create directories if they don't exist
    fs::create_dir_all(SPRITES_DIR).expect("Failed to create sprites directory");
    fs::create_dir_all(STATIC_DIR).expect("Failed to create static directory");

    // Create team file if it doesn't exist
    if !path::Path::new(TEAM_FILE).exists() {
        let default_team = "pikachu\ncharizard\nblastoise\nvenusaur\nmewtwo\ndragonite\n";
        fs::write(TEAM_FILE, default_team).expect("Failed to create team file");
    }

    // Create broadcast channel for team updates
    let (tx, _) = broadcast::channel::<PokemonTeam>(100);
    let state = Arc::new(AppState { tx: tx.clone() });

    // Setup file watcher with event-based monitoring
    let tx_watcher = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = watch_team_file(tx_watcher).await {
            eprintln!("File watcher error: {}", e);
        }
    });

    // Build the router
    let app = Router::new()
        .route("/ws", get(websocket_handler))
        .nest_service("/sprites", ServeDir::new(SPRITES_DIR))
        .route("/", get(|| async { embedded_static(Path("".into())).await }))
        .route("/*path", get(embedded_static))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // Start the server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("Failed to bind to port 3000");

    println!("🚀 Server running on http://127.0.0.1:3000");
    println!("📝 Edit '{}' to update your Pokemon team", TEAM_FILE);
    println!(
        "🖼️  Place your Pokemon sprites in the '{}' directory",
        SPRITES_DIR
    );

    axum::serve(listener, app)
        .await
        .expect("Failed to start server");
}

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

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, _receiver) = socket.split();
    let mut rx = state.tx.subscribe();

    // Send initial team state
    if let Ok(team) = read_team_file() {
        let json = serde_json::to_string(&team).unwrap();
        if sender.send(Message::Text(json)).await.is_err() {
            return;
        }
    }

    // Listen for team updates and forward to websocket
    while let Ok(team) = rx.recv().await {
        let json = serde_json::to_string(&team).unwrap();
        if sender.send(Message::Text(json)).await.is_err() {
            break;
        }
    }
}

async fn watch_team_file(tx: broadcast::Sender<PokemonTeam>) -> notify::Result<()> {
    use notify::{Config, EventKind};

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel(100);

    // Create watcher with custom config
    let config = Config::default().with_poll_interval(std::time::Duration::from_secs(1));

    let mut watcher = notify::RecommendedWatcher::new(
        move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = notify_tx.blocking_send(event);
            }
        },
        config,
    )?;

    // Watch the parent directory to catch rename/replace operations
    let team_path = path::Path::new(TEAM_FILE);
    let watch_path = team_path.parent().unwrap_or(path::Path::new("."));

    watcher.watch(watch_path, RecursiveMode::NonRecursive)?;

    // Send initial state
    if let Ok(team) = read_team_file() {
        let _ = tx.send(team);
    }

    let mut last_content = String::new();
    if let Ok(content) = fs::read_to_string(TEAM_FILE) {
        last_content = content;
    }

    // Watch for file changes
    loop {
        match notify_rx.recv().await {
            Some(event) => {
                // Check if the event is related to our file
                let is_team_file = event
                    .paths
                    .iter()
                    .any(|p| p.file_name() == team_path.file_name());

                if !is_team_file {
                    continue;
                }

                match event.kind {
                    EventKind::Modify(_)
                    | EventKind::Create(_)
                    | EventKind::Remove(_)
                    | EventKind::Any => {
                        // Small delay to ensure file write is complete
                        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

                        // Check if content actually changed
                        if let Ok(new_content) = fs::read_to_string(TEAM_FILE) {
                            if new_content != last_content {
                                last_content = new_content;

                                if let Ok(team) = read_team_file() {
                                    let _ = tx.send(team);
                                }
                            }
                        }
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

    // Keep watcher alive
    drop(watcher);
    Ok(())
}

fn read_team_file() -> Result<PokemonTeam, std::io::Error> {
    let content = fs::read_to_string(TEAM_FILE)?;
    let pokemon: Vec<Pokemon> = content
        .lines()
        .map(|line| {
            let parts: Vec<&str> = line.trim().split(":").collect();
            let name = parts[0].to_string();
            let nickname = if parts.len() > 1 {
                Some(parts[1..].join(" "))
            } else {
                None
            };
            Pokemon { name, nickname }
        })
        .filter(|pokemon| !pokemon.name.is_empty())
        .take(6) // Only take first 6 Pokemon
        .collect();

    // Pad with empty strings if less than 6
    let mut pokemon_team = pokemon;
    while pokemon_team.len() < 6 {
        pokemon_team.push(Pokemon {
            name: String::new(),
            nickname: None,
        });
    }

    Ok(PokemonTeam {
        pokemon: pokemon_team,
    })
}
