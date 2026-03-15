mod app;
mod model;

use iced::window;
use iced::{Element, Size, Task};
use tokio::sync::{mpsc, watch};

pub use model::TeamUrl;
use model::{Message, UiSnapshot};

fn update(app: &mut app::ConnectionUiApp, message: Message) -> Task<Message> {
    app.update(message)
}

fn view(app: &app::ConnectionUiApp) -> Element<'_, Message> {
    app.view()
}

pub enum UiAction {
    ConnectTicket(String),
    ToggleSourceMode,
    SelectSaveFile,
}

#[derive(Clone)]
pub struct UiBridge {
    snapshot_tx: watch::Sender<UiSnapshot>,
}

impl UiBridge {
    pub fn set_local_ticket(&self, ticket: String) {
        self.snapshot_tx.send_modify(|snapshot| {
            snapshot.local_ticket = ticket;
        });
    }

    pub fn set_status(&self, status: String) {
        self.snapshot_tx.send_modify(|snapshot| {
            snapshot.status = status;
        });
    }

    pub fn set_source_mode(&self, mode: String) {
        self.snapshot_tx.send_modify(|snapshot| {
            snapshot.source_mode = mode;
        });
    }

    pub fn set_local_urls(&self, urls: Vec<TeamUrl>) {
        self.snapshot_tx.send_modify(|snapshot| {
            snapshot.local_urls = urls;
        });
    }

    pub fn set_remote_urls(&self, urls: Vec<TeamUrl>) {
        self.snapshot_tx.send_modify(|snapshot| {
            snapshot.remote_urls = urls;
        });
    }
}

/// Prepares the iced application and returns it ready to run, along with the
/// bridge used to push state updates into it and the channel for receiving
/// actions from it.
///
/// The caller is responsible for calling `.run()` on the returned application
/// **on the main thread**, as required by every major windowing system.
pub fn build_connection_ui() -> (
    impl FnOnce() -> iced::Result,
    UiBridge,
    mpsc::Receiver<UiAction>,
) {
    let (action_tx, action_rx) = mpsc::channel(16);
    let (snapshot_tx, snapshot_rx) = watch::channel(UiSnapshot::default());

    let ui_bridge = UiBridge { snapshot_tx };
    let ui_action_tx = action_tx;

    // iced requires `Fn` for boot, but these values are only consumed once.
    // Wrap in Mutex<Option<_>> so the closure satisfies the `Fn` bound.
    let boot_state = std::sync::Mutex::new(Some((snapshot_rx, ui_action_tx)));

    let runner = move || {
        iced::application(
            move || {
                let (rx, tx) = boot_state
                    .lock()
                    .unwrap()
                    .take()
                    .expect("boot called more than once");
                app::ConnectionUiApp::new(rx, tx)
            },
            update,
            view,
        )
        .title("Pokemon Team Overlay - P2P")
        .subscription(|app: &app::ConnectionUiApp| app.subscription())
        .theme(|app: &app::ConnectionUiApp| app.theme())
        .window(window::Settings {
            size: Size::new(720.0, 640.0),
            min_size: Some(Size::new(500.0, 400.0)),
            ..Default::default()
        })
        .run()
    };

    (Box::new(runner), ui_bridge, action_rx)
}
