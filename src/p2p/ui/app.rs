use std::time::Duration;

use iced::border::Radius;
use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, row, rule, scrollable, text, text_input, Column};
use iced::{color, Element, Font, Length, Subscription, Task, Theme};
use tokio::sync::{mpsc, watch};

use super::model::{Message, TeamUrl, UiSnapshot};
use super::UiAction;

pub struct ConnectionUiApp {
    snapshot_rx: watch::Receiver<UiSnapshot>,
    snapshot: UiSnapshot,
    action_tx: mpsc::Sender<UiAction>,
    ticket_input: String,
    local_status: Option<String>,
}

impl ConnectionUiApp {
    pub fn new(
        snapshot_rx: watch::Receiver<UiSnapshot>,
        action_tx: mpsc::Sender<UiAction>,
    ) -> (Self, Task<Message>) {
        let snapshot = snapshot_rx.borrow().clone();
        (
            Self {
                snapshot_rx,
                snapshot,
                action_tx,
                ticket_input: String::new(),
                local_status: None,
            },
            Task::none(),
        )
    }

    fn pull_updates(&mut self) {
        while self.snapshot_rx.has_changed().unwrap_or(false) {
            self.snapshot = self.snapshot_rx.borrow_and_update().clone();
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TicketInputChanged(value) => {
                self.ticket_input = value;
                Task::none()
            }
            Message::CopyTicket => {
                if !self.snapshot.local_ticket.is_empty() {
                    let ticket = self.snapshot.local_ticket.clone();
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(&ticket);
                    }
                    self.local_status = Some("Ticket copied to clipboard.".to_string());
                }
                Task::none()
            }
            Message::SubmitTicket => {
                let trimmed = self.ticket_input.trim().to_string();
                if trimmed.is_empty() {
                    self.local_status = Some("Please enter a ticket.".to_string());
                } else if self
                    .action_tx
                    .try_send(UiAction::ConnectTicket(trimmed))
                    .is_err()
                {
                    self.local_status = Some("Unable to submit ticket right now.".to_string());
                } else {
                    self.ticket_input.clear();
                    self.local_status = Some("Ticket submitted. Connecting...".to_string());
                }
                Task::none()
            }
            Message::ToggleSourceMode => {
                if self.action_tx.try_send(UiAction::ToggleSourceMode).is_err() {
                    self.local_status = Some("Unable to switch mode right now.".to_string());
                }
                Task::none()
            }
            Message::SelectSaveFile => {
                if self.action_tx.try_send(UiAction::SelectSaveFile).is_err() {
                    self.local_status =
                        Some("Unable to open save-file picker right now.".to_string());
                }
                Task::none()
            }
            Message::CopyUrl(url) => {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(&url);
                }
                self.local_status = Some("URL copied to clipboard.".to_string());
                Task::none()
            }
            Message::Tick => {
                self.pull_updates();
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_millis(150)).map(|_| Message::Tick)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let content = column![
            self.view_connection_section(),
            rule::horizontal(1),
            self.view_status_section(),
            rule::horizontal(1),
            self.view_teams_section(),
        ]
        .spacing(16)
        .padding(24);

        let scrolled = scrollable(content).height(Length::Fill);

        container(scrolled)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_connection_section(&self) -> Element<'_, Message> {
        let heading = text("Connection").size(22).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        });

        // Your ticket section
        let your_ticket_label = text("Your ticket:").size(14);
        let copy_btn =
            styled_button("Copy Ticket", ButtonStyle::Primary).on_press(Message::CopyTicket);

        let ticket_display = container(
            text(&self.snapshot.local_ticket)
                .size(12)
                .font(Font::MONOSPACE)
                .width(Length::Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(|_theme: &Theme| text::Style {
                    color: Some(color!(0xcdd6f4)),
                    ..Default::default()
                }),
        )
        .padding(10)
        .width(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(iced::Background::Color(color!(0x1a1a2e))),
            border: iced::Border {
                color: color!(0x3a3a5c),
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        });

        let your_ticket = column![
            row![your_ticket_label, copy_btn]
                .spacing(12)
                .align_y(iced::Alignment::Center),
            ticket_display,
        ]
        .spacing(8);

        // Peer ticket section
        let peer_ticket_label = text("Peer ticket:").size(14);
        let peer_input = text_input("Paste peer ticket here...", &self.ticket_input)
            .on_input(Message::TicketInputChanged)
            .on_submit(Message::SubmitTicket)
            .padding(10)
            .size(13);

        let connect_btn =
            styled_button("Connect", ButtonStyle::Accent).on_press(Message::SubmitTicket);

        let peer_section = column![peer_ticket_label, peer_input, connect_btn].spacing(8);

        // Source mode section
        let source_label = row![
            text("Source mode: ").size(13),
            text(&self.snapshot.source_mode).size(13).font(Font {
                weight: iced::font::Weight::Bold,
                ..Font::DEFAULT
            }),
        ]
        .align_y(iced::Alignment::Center);

        let switch_btn = styled_button("Switch Mode", ButtonStyle::Secondary)
            .on_press(Message::ToggleSourceMode);
        let sav_btn = styled_button("Change .sav File", ButtonStyle::Secondary)
            .on_press(Message::SelectSaveFile);

        let mode_section = column![source_label, row![switch_btn, sav_btn].spacing(8),].spacing(8);

        column![
            heading,
            your_ticket,
            rule::horizontal(1),
            peer_section,
            rule::horizontal(1),
            mode_section,
        ]
        .spacing(12)
        .into()
    }

    fn view_status_section(&self) -> Element<'_, Message> {
        let status_text = text(format!("Status: {}", self.snapshot.status)).size(14);

        let mut col = column![status_text].spacing(4);

        if let Some(local_status) = &self.local_status {
            col = col.push(
                container(text(local_status).size(13).color(color!(0x64d2ff)))
                    .padding([4, 8])
                    .style(|_theme: &Theme| container::Style {
                        background: Some(iced::Background::Color(color!(0x0a3d5c, 0.5))),
                        border: iced::Border {
                            radius: 4.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }

        col.into()
    }

    fn view_teams_section(&self) -> Element<'_, Message> {
        let local_grid = team_grid("Local Teams", &self.snapshot.local_urls);
        let remote_grid = team_grid("Remote Teams", &self.snapshot.remote_urls);

        column![local_grid, rule::horizontal(1), remote_grid]
            .spacing(16)
            .into()
    }

    pub fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }
}

#[derive(Clone, Copy)]
enum ButtonStyle {
    Primary,
    Secondary,
    Accent,
}

fn styled_button(label: &str, style: ButtonStyle) -> iced::widget::Button<'_, Message> {
    let label_widget = text(label.to_string()).size(13);

    let btn = button(
        container(label_widget)
            .padding([6, 14])
            .center_x(Length::Shrink),
    );

    match style {
        ButtonStyle::Primary => btn.style(|theme: &Theme, status| {
            let palette = theme.palette();
            let base = button::Style {
                background: Some(iced::Background::Color(palette.primary)),
                text_color: palette.background,
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..button::Style::default()
            };
            match status {
                button::Status::Hovered => button::Style {
                    background: Some(iced::Background::Color(lighten(palette.primary, 0.1))),
                    ..base
                },
                button::Status::Pressed => button::Style {
                    background: Some(iced::Background::Color(darken(palette.primary, 0.1))),
                    ..base
                },
                _ => base,
            }
        }),
        ButtonStyle::Secondary => btn.style(|theme: &Theme, status| {
            let palette = theme.palette();
            let bg = color!(0x313244);
            let base = button::Style {
                background: Some(iced::Background::Color(bg)),
                text_color: palette.text,
                border: iced::Border {
                    color: color!(0x45475a),
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..button::Style::default()
            };
            match status {
                button::Status::Hovered => button::Style {
                    background: Some(iced::Background::Color(color!(0x45475a))),
                    ..base
                },
                button::Status::Pressed => button::Style {
                    background: Some(iced::Background::Color(color!(0x585b70))),
                    ..base
                },
                _ => base,
            }
        }),
        ButtonStyle::Accent => btn.style(|theme: &Theme, status| {
            let palette = theme.palette();
            let accent = color!(0xa6e3a1);
            let base = button::Style {
                background: Some(iced::Background::Color(accent)),
                text_color: palette.background,
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..button::Style::default()
            };
            match status {
                button::Status::Hovered => button::Style {
                    background: Some(iced::Background::Color(lighten(accent, 0.1))),
                    ..base
                },
                button::Status::Pressed => button::Style {
                    background: Some(iced::Background::Color(darken(accent, 0.1))),
                    ..base
                },
                _ => base,
            }
        }),
    }
}

fn team_grid<'a>(title: &str, rows: &[TeamUrl]) -> Element<'a, Message> {
    let heading = text(title.to_string()).size(18).font(Font {
        weight: iced::font::Weight::Bold,
        ..Font::DEFAULT
    });

    if rows.is_empty() {
        return column![heading, text("No teams available.").size(13)]
            .spacing(8)
            .into();
    }

    // Header row
    let header_radius = Radius {
        top_left: 6.0,
        top_right: 6.0,
        bottom_right: 0.0,
        bottom_left: 0.0,
    };

    let header = container(
        row![
            text("Team")
                .size(12)
                .font(Font {
                    weight: iced::font::Weight::Bold,
                    ..Font::DEFAULT
                })
                .width(Length::FillPortion(2)),
            text("URL")
                .size(12)
                .font(Font {
                    weight: iced::font::Weight::Bold,
                    ..Font::DEFAULT
                })
                .width(Length::FillPortion(5)),
            text("Action")
                .size(12)
                .font(Font {
                    weight: iced::font::Weight::Bold,
                    ..Font::DEFAULT
                })
                .width(Length::FillPortion(2)),
        ]
        .spacing(8)
        .padding([6, 10]),
    )
    .style(move |_theme: &Theme| container::Style {
        background: Some(iced::Background::Color(color!(0x313244))),
        border: iced::Border {
            radius: header_radius,
            ..Default::default()
        },
        ..Default::default()
    })
    .width(Length::Fill);

    let mut grid_col = Column::new().push(header);

    for (i, team_row) in rows.iter().enumerate() {
        let bg = if i % 2 == 0 {
            color!(0x1e1e2e)
        } else {
            color!(0x24243a)
        };

        let url_text = team_row.url.clone();
        let row_widget = container(
            row![
                text(team_row.team_name.clone())
                    .size(13)
                    .font(Font {
                        weight: iced::font::Weight::Bold,
                        ..Font::DEFAULT
                    })
                    .width(Length::FillPortion(2)),
                text(team_row.url.clone())
                    .size(12)
                    .font(Font::MONOSPACE)
                    .width(Length::FillPortion(5)),
                button(container(text("Copy URL").size(11)).padding([3, 8]),)
                    .on_press(Message::CopyUrl(url_text))
                    .style(|_theme: &Theme, status| {
                        let base = button::Style {
                            background: Some(iced::Background::Color(color!(0x45475a))),
                            text_color: color!(0xcdd6f4),
                            border: iced::Border {
                                radius: 4.0.into(),
                                ..Default::default()
                            },
                            ..button::Style::default()
                        };
                        match status {
                            button::Status::Hovered => button::Style {
                                background: Some(iced::Background::Color(color!(0x585b70))),
                                ..base
                            },
                            _ => base,
                        }
                    })
                    .width(Length::FillPortion(2)),
            ]
            .spacing(8)
            .padding([8, 10])
            .align_y(iced::Alignment::Center),
        )
        .style(move |_theme: &Theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            ..Default::default()
        })
        .width(Length::Fill);

        grid_col = grid_col.push(row_widget);
    }

    let bordered_grid = container(grid_col)
        .style(|_theme: &Theme| container::Style {
            border: iced::Border {
                color: color!(0x45475a),
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        })
        .width(Length::Fill);

    column![heading, bordered_grid].spacing(8).into()
}

fn lighten(color: iced::Color, amount: f32) -> iced::Color {
    iced::Color {
        r: (color.r + amount).min(1.0),
        g: (color.g + amount).min(1.0),
        b: (color.b + amount).min(1.0),
        a: color.a,
    }
}

fn darken(color: iced::Color, amount: f32) -> iced::Color {
    iced::Color {
        r: (color.r - amount).max(0.0),
        g: (color.g - amount).max(0.0),
        b: (color.b - amount).max(0.0),
        a: color.a,
    }
}
