mod app;
mod model;

use eframe::egui;
use tokio::sync::{mpsc, watch};

pub use model::TeamUrl;
use model::UiSnapshot;

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

pub fn spawn_connection_ui() -> (UiBridge, mpsc::Receiver<String>) {
    let (ticket_input_tx, ticket_input_rx) = mpsc::channel(16);
    let (snapshot_tx, snapshot_rx) = watch::channel(UiSnapshot::default());
    let ui_bridge = UiBridge {
        snapshot_tx: snapshot_tx.clone(),
    };

    let ui_ticket_tx = ticket_input_tx.clone();
    std::thread::spawn(move || {
        let mut options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([720.0, 640.0]),
            ..Default::default()
        };

        #[cfg(target_os = "linux")]
        {
            options.event_loop_builder = Some(Box::new(|builder| {
                use winit::platform::wayland::EventLoopBuilderExtWayland;
                use winit::platform::x11::EventLoopBuilderExtX11;
                EventLoopBuilderExtWayland::with_any_thread(builder, true);
                EventLoopBuilderExtX11::with_any_thread(builder, true);
            }));
        }

        let result = eframe::run_native(
            "Pokemon Team Overlay - P2P",
            options,
            Box::new(move |_cc| {
                Ok(Box::new(app::ConnectionUiApp::new(
                    snapshot_rx,
                    ui_ticket_tx,
                )))
            }),
        );

        if let Err(err) = result {
            eprintln!("P2P UI error: {}", err);
        }
    });

    (ui_bridge, ticket_input_rx)
}
