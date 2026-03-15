#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod p2p;
mod savefile;
mod source_picker;
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
use rust_embed::RustEmbed;
use std::{collections::HashMap, env, fs, path, sync::Arc};
use tokio::sync::broadcast;
use tower_http::{cors::CorsLayer, services::ServeDir};

use serde::Serialize;
use state::{AppState, TeamSource};
use team::PokemonTeam;

// ── Embedded static assets ────────────────────────────────────────────────────

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

// ── Constants ─────────────────────────────────────────────────────────────────

const TEAM_FILE: &str = "team.txt";
const SPRITES_DIR: &str = "sprites";
const STATIC_DIR: &str = "static";

// ── Application state ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct WsPayload {
    teams: HashMap<String, PokemonTeam>,
    remote: HashMap<String, PokemonTeam>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let source = choose_initial_team_source();

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

    // Build the iced UI on the main thread (required by winit / most OS window
    // systems).  The Tokio runtime runs on a background thread instead.
    let (ui_runner, ui_bridge, action_rx) = p2p::build_connection_ui();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to build Tokio runtime");

        rt.block_on(async move {
            let state = Arc::new(AppState::new(source));

            let source_watch_state = Arc::clone(&state);
            tokio::spawn(async move {
                watch_current_source(source_watch_state).await;
            });

            let p2p_state = Arc::clone(&state);
            let p2p_ui = ui_bridge;
            tokio::spawn(async move {
                if let Err(e) = p2p::start(p2p_state, p2p_ui, action_rx).await {
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

            let source = state.source.read().await.clone();
            match &source {
                TeamSource::TextFiles => {
                    println!("Server running on http://0.0.0.0:{}", port);
                    println!("Edit '{}' to update your Pokemon team", TEAM_FILE);
                    println!("  - Additional files whose name contains 'team' are also picked up.");
                    println!("  - Use '?team=<name>' in the URL to switch between teams.");
                }
                TeamSource::SaveFile(path) => {
                    println!("Server running on http://0.0.0.0:{}", port);
                    println!("Reading party from save file: {}", path);
                }
            }
            println!("Place Pokemon sprites in the '{}' directory", SPRITES_DIR);

            axum::serve(listener, app)
                .await
                .expect("Failed to start server");
        });
    });

    // Run the iced window on the main thread; blocks until the window is closed.
    let _ = ui_runner();

    // When the UI window is closed, exit the whole process.  The background
    // thread (Tokio runtime) will be torn down automatically.
    std::process::exit(0);
}

// ── Save-file selection ──────────────────────────────────────────────────────

fn choose_initial_team_source() -> TeamSource {
    if let Some(cached) = source_picker::load_cached_save_path().filter(|p| p.exists()) {
        if source_picker::validate_save_file(&cached).is_ok() {
            return TeamSource::SaveFile(cached.to_string_lossy().into_owned());
        }
        eprintln!(
            "Cached save file '{}' failed to load. Please choose a valid .sav file.",
            cached.display()
        );
        if let Some(selected) = source_picker::prompt_for_valid_save_file(Some(cached.as_path())) {
            return TeamSource::SaveFile(selected.to_string_lossy().into_owned());
        }
    }

    match source_picker::prompt_for_valid_save_file(None) {
        Some(path) => TeamSource::SaveFile(path.to_string_lossy().into_owned()),
        None => {
            eprintln!("No .sav file selected; falling back to text-file mode.");
            TeamSource::TextFiles
        }
    }
}

async fn watch_current_source(state: Arc<AppState>) {
    let mut last_sent: Option<HashMap<String, PokemonTeam>> = None;
    let mut prompted_for_save_error = false;

    loop {
        let source = state.source.read().await.clone();
        match source.read() {
            Ok(teams) => {
                prompted_for_save_error = false;
                if last_sent.as_ref() != Some(&teams) {
                    let _ = state.tx.send(teams.clone());
                    last_sent = Some(teams);
                }
            }
            Err(err) => {
                eprintln!("Team source read error: {}", err);
                if let TeamSource::SaveFile(path) = source {
                    if !prompted_for_save_error {
                        prompted_for_save_error = true;
                        if let Some(selected) = source_picker::prompt_for_valid_save_file(Some(
                            std::path::Path::new(&path),
                        )) {
                            let mut source = state.source.write().await;
                            *source = TeamSource::SaveFile(selected.to_string_lossy().into_owned());
                            continue;
                        }

                        eprintln!(
                            "No replacement save file selected. Switching to text-file mode."
                        );
                        let mut source = state.source.write().await;
                        *source = TeamSource::TextFiles;
                    }
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
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
    let local_source = state.source.read().await.clone();
    let mut current_local = match local_source.read() {
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
