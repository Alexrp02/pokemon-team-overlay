use std::time::Duration;

use eframe::egui;
use tokio::sync::{mpsc, watch};

use super::model::{TeamUrl, UiSnapshot};

pub struct ConnectionUiApp {
    snapshot_rx: watch::Receiver<UiSnapshot>,
    snapshot: UiSnapshot,
    ticket_input_tx: mpsc::Sender<String>,
    ticket_input: String,
    local_status: Option<String>,
}

impl ConnectionUiApp {
    pub fn new(
        snapshot_rx: watch::Receiver<UiSnapshot>,
        ticket_input_tx: mpsc::Sender<String>,
    ) -> Self {
        let snapshot = snapshot_rx.borrow().clone();
        Self {
            snapshot_rx,
            snapshot,
            ticket_input_tx,
            ticket_input: String::new(),
            local_status: None,
        }
    }

    fn pull_updates(&mut self) {
        while self.snapshot_rx.has_changed().unwrap_or(false) {
            self.snapshot = self.snapshot_rx.borrow_and_update().clone();
        }
    }

    fn ticket_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connection");

        ui.horizontal(|ui| {
            ui.label("Your ticket:");
            if ui.button("Copy ticket").clicked() && !self.snapshot.local_ticket.is_empty() {
                ui.ctx().copy_text(self.snapshot.local_ticket.clone());
                self.local_status = Some("Ticket copied to clipboard.".to_string());
            }
        });
        ui.add_enabled(
            false,
            egui::TextEdit::multiline(&mut self.snapshot.local_ticket)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );

        ui.separator();
        ui.label("Peer ticket:");
        let enter_pressed = ui
            .add(
                egui::TextEdit::multiline(&mut self.ticket_input)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            )
            .lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl);

        let submit_clicked = ui.button("Connect").clicked();
        if submit_clicked || enter_pressed {
            let trimmed = self.ticket_input.trim();
            if trimmed.is_empty() {
                self.local_status = Some("Please enter a ticket.".to_string());
            } else if self.ticket_input_tx.try_send(trimmed.to_string()).is_err() {
                self.local_status = Some("Unable to submit ticket right now.".to_string());
            } else {
                self.ticket_input.clear();
                self.local_status = Some("Ticket submitted. Connecting...".to_string());
            }
        }
    }
}

fn team_grid(ui: &mut egui::Ui, title: &str, rows: &[TeamUrl]) {
    ui.heading(title);
    if rows.is_empty() {
        ui.label("No teams available.");
        return;
    }

    egui::Grid::new(title)
        .num_columns(3)
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Team");
            ui.strong("URL");
            ui.strong("Action");
            ui.end_row();

            for row in rows {
                ui.label(&row.team_name);
                if ui.selectable_label(false, &row.url).clicked() {
                    ui.ctx().copy_text(row.url.clone());
                }
                if ui.button("Copy URL").clicked() {
                    ui.ctx().copy_text(row.url.clone());
                }
                ui.end_row();
            }
        });
}

impl eframe::App for ConnectionUiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pull_updates();

        egui::CentralPanel::default().show(ctx, |ui| {
            self.ticket_section(ui);
            ui.separator();

            ui.label(format!("Status: {}", self.snapshot.status));
            if let Some(local_status) = &self.local_status {
                ui.label(local_status);
            }

            ui.separator();
            team_grid(ui, "Local Teams", &self.snapshot.local_urls);
            ui.separator();
            team_grid(ui, "Remote Teams", &self.snapshot.remote_urls);
        });

        ctx.request_repaint_after(Duration::from_millis(150));
    }
}
