//! The screens: setup and sign-in, then the main window with its tabs, the
//! now-playing panel on the right and the top bar.

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Key, Layout, Margin, Rect, RichText, Sense, Stroke,
    Ui, Vec2, pos2, vec2,
};
use rand::Rng;

use super::{App, ListView, NOTICE_TIME, PlayerState, Screen, Tab};
use crate::api::TrackSource;
use crate::auth;
use crate::config::{self, Repeat};
use crate::theme::{self, BG, BORDER, ERROR, HOVER, MUTED, RAISED, SUBDUED, SURFACE, TEXT};
use crate::widgets::{self, ButtonStyle, Icon, format_time};

/// What the user did in a song list.
enum ListAction {
    Play(usize),
    Queue(usize),
    PlayAll,
    ShuffleAll,
}

#[derive(Clone, Copy, PartialEq)]
enum Which {
    Library,
    Podcast,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_messages(&ctx);

        match self.screen {
            Screen::Setup { .. } => self.setup_screen(ui),
            Screen::SignIn { .. } => self.sign_in_screen(ui),
            Screen::Main => {
                // Space toggles play/pause, unless the user is typing. The key is consumed
                // so a focused button does not also react to it.
                if !ctx.egui_wants_keyboard_input()
                    && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Space))
                {
                    self.toggle_play();
                }
                self.main_screen(ui);
                // Keep the seek bar moving; otherwise only redraw when something happens.
                if self.now.playing {
                    ctx.request_repaint_after(std::time::Duration::from_millis(500));
                }
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.config.save();
    }
}

impl App {
    // ---------------------------------------------------------- first start

    /// A card in the middle of the window, used by the setup and sign-in screens.
    fn centered_card(ui: &mut Ui, width: f32, add_contents: impl FnOnce(&mut Ui)) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let width = width.min(ui.available_width() - 32.0);
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.06).clamp(16.0, 80.0));
                        Frame::new()
                            .fill(SURFACE)
                            .stroke(Stroke::new(1.0, BORDER))
                            .corner_radius(14)
                            .inner_margin(32)
                            .show(ui, |ui| {
                                ui.set_width(width - 64.0);
                                ui.with_layout(Layout::top_down(Align::Min), add_contents);
                            });
                        ui.add_space(24.0);
                    });
                });
            });
    }

    fn setup_screen(&mut self, ui: &mut Ui) {
        let copied = self
            .copied_at
            .is_some_and(|t| t.elapsed().as_secs_f32() < 2.0);
        if copied {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        let Screen::Setup { input, error } = &mut self.screen else {
            return;
        };
        let mut submit = false;
        let mut copy = false;
        Self::centered_card(ui, 600.0, |ui| {
            ui.horizontal(|ui| {
                widgets::logo(ui, 40.0);
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Welcome to lite-spotify")
                        .font(theme::bold(26.0))
                        .color(TEXT),
                );
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "lite-spotify connects to Spotify through your own free developer app. \
                     This takes about three minutes and only has to be done once.",
                )
                .color(SUBDUED),
            );
            ui.add_space(18.0);

            step(ui, 1, |ui| {
                ui.label("Open");
                ui.hyperlink_to(
                    "developer.spotify.com/dashboard",
                    "https://developer.spotify.com/dashboard",
                );
                ui.label("and log in.");
            });
            step(ui, 2, |ui| {
                ui.label("Click \"Create app\". Any name and description will do.");
            });
            step(ui, 3, |ui| {
                ui.label("Under \"Redirect URIs\", add exactly this address:");
            });
            ui.horizontal(|ui| {
                ui.add_space(36.0);
                Frame::new()
                    .fill(RAISED)
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(auth::WEB_REDIRECT_URI)
                                .monospace()
                                .color(TEXT),
                        );
                    });
                let label = if copied { "Copied" } else { "Copy" };
                if widgets::button(ui, label, ButtonStyle::Ghost, 13.0).clicked() {
                    copy = true;
                }
            });
            ui.add_space(4.0);
            step(ui, 4, |ui| {
                ui.label("Tick \"Web API\", accept the terms and click Save.");
            });
            step(ui, 5, |ui| {
                ui.label("Open the app's Settings, copy the Client ID and paste it below.");
            });
            ui.add_space(12.0);

            let edit = ui.add(
                egui::TextEdit::singleline(input)
                    .hint_text("Client ID")
                    .font(theme::regular(16.0))
                    .margin(vec2(14.0, 12.0))
                    .desired_width(f32::INFINITY),
            );
            if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
            if let Some(e) = error {
                ui.add_space(4.0);
                ui.label(RichText::new(e.as_str()).color(ERROR));
            }
            ui.add_space(14.0);
            let width = ui.available_width();
            if widgets::button_sized(
                ui,
                "Save and sign in",
                ButtonStyle::Accent,
                16.0,
                Some(width),
            )
            .clicked()
            {
                submit = true;
            }
            ui.add_space(14.0);
            ui.label(
                RichText::new(format!(
                    "Settings are saved in {}",
                    config::app_dir().display()
                ))
                .color(MUTED)
                .small(),
            );
        });

        if copy {
            ui.ctx().copy_text(auth::WEB_REDIRECT_URI.to_string());
            self.copied_at = Some(std::time::Instant::now());
        }
        if submit {
            let id = input.trim().to_string();
            if id.len() == 32 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
                self.config.client_id = id;
                self.config.refresh_token = None;
                self.config.save();
                self.start_browser_sign_in();
            } else {
                *error = Some(
                    "That does not look like a Client ID. It should be 32 letters and numbers."
                        .into(),
                );
            }
        }
    }

    fn sign_in_screen(&mut self, ui: &mut Ui) {
        let Screen::SignIn { url, busy, error } = &self.screen else {
            return;
        };
        let (url, busy, error) = (url.clone(), *busy, error.clone());
        let mut sign_in = false;
        let mut change_id = false;
        let mut cancel = false;
        Self::centered_card(ui, 440.0, |ui| {
            ui.vertical_centered(|ui| {
                widgets::logo(ui, 56.0);
                ui.add_space(12.0);
                ui.label(
                    RichText::new("lite-spotify")
                        .font(theme::bold(30.0))
                        .color(TEXT),
                );
                ui.label(
                    RichText::new("A small player for your Spotify Premium account").color(SUBDUED),
                );
                ui.add_space(26.0);
                if busy {
                    ui.add(egui::Spinner::new().size(28.0).color(theme::accent()));
                    ui.add_space(12.0);
                    if let Some(url) = &url {
                        ui.label(RichText::new("Finish signing in in your browser,").color(TEXT));
                        ui.label(RichText::new("then come back here.").color(TEXT));
                        ui.add_space(12.0);
                        ui.hyperlink_to("Browser did not open? Click here.", url);
                        ui.add_space(18.0);
                        if widgets::button(ui, "Cancel", ButtonStyle::Ghost, 14.0).clicked() {
                            cancel = true;
                        }
                    } else {
                        ui.label(RichText::new("Signing in…").color(TEXT));
                    }
                } else {
                    if let Some(e) = &error {
                        ui.label(RichText::new(e).color(ERROR));
                        ui.add_space(16.0);
                    }
                    let width = ui.available_width();
                    if widgets::button_sized(
                        ui,
                        "Sign in with Spotify",
                        ButtonStyle::Accent,
                        16.0,
                        Some(width),
                    )
                    .clicked()
                    {
                        sign_in = true;
                    }
                    ui.add_space(14.0);
                    let other = ui.add(
                        egui::Button::new(
                            RichText::new("Use a different Client ID").color(SUBDUED),
                        )
                        .frame(false),
                    );
                    if other.clicked() {
                        change_id = true;
                    }
                }
            });
        });
        if sign_in {
            self.start_browser_sign_in();
        }
        if cancel {
            self.cancel_sign_in();
        }
        if change_id {
            self.screen = Screen::Setup {
                input: self.config.client_id.clone(),
                error: None,
            };
        }
    }

    // ---------------------------------------------------------- main window

    fn main_screen(&mut self, ui: &mut Ui) {
        egui::Panel::top("top")
            .exact_size(54.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .inner_margin(Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::right("now-playing")
            .resizable(true)
            .default_size(300.0)
            .size_range(230.0..=420.0)
            .frame(Frame::new().fill(SURFACE).inner_margin(Margin::same(16)))
            .show(ui, |ui| self.now_playing_panel(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG))
            .show(ui, |ui| {
                self.player_banner(ui);
                match self.tab {
                    Tab::Library => self.library_tab(ui),
                    Tab::Search => self.search_tab(ui),
                    Tab::Podcasts => self.podcasts_tab(ui),
                    Tab::Fun => self.fun_tab(ui),
                    Tab::Settings => self.settings_tab(ui),
                }
            });
        self.toast(ui.ctx());
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let bar = ui.max_rect();
        ui.painter()
            .hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, BORDER));
        ui.horizontal_centered(|ui| {
            widgets::logo(ui, 26.0);
            ui.label(
                RichText::new("lite-spotify")
                    .font(theme::bold(16.0))
                    .color(TEXT),
            );
            ui.add_space(18.0);
            let mut tabs = vec![
                (Tab::Library, "Library"),
                (Tab::Search, "Search"),
                (Tab::Podcasts, "Podcasts"),
            ];
            if self.fun.is_some() {
                tabs.push((Tab::Fun, "Fun"));
            }
            tabs.push((Tab::Settings, "Settings"));
            for (tab, label) in tabs {
                if tab_button(ui, label, self.tab == tab, bar.bottom()).clicked() {
                    self.tab = tab;
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let width = (ui.available_width() - 8.0).clamp(140.0, 300.0);
                let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
                if self.search_box(ui, rect) {
                    self.start_search();
                }
            });
        });
    }

    /// The search box. Returns true when the user presses Enter.
    fn search_box(&mut self, ui: &mut Ui, rect: Rect) -> bool {
        let id = egui::Id::unique("search");
        let focused = ui.memory(|m| m.has_focus(id));
        let background = ui.interact(rect, id.with("bg"), Sense::click());
        if background.clicked() {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let fill = if focused || background.hovered() {
            HOVER
        } else {
            RAISED
        };
        let stroke = if focused {
            Stroke::new(1.5, theme::accent())
        } else {
            Stroke::new(1.0, BORDER)
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(8),
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );
        let icon =
            Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(15.0));
        widgets::paint_icon(ui.painter(), icon, Icon::Search, SUBDUED);
        let text_rect = Rect::from_min_max(
            pos2(rect.left() + 34.0, rect.top()),
            pos2(rect.right() - 30.0, rect.bottom()),
        );
        let submitted = widgets::place_in(ui, text_rect, |ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.search_input)
                    .id(id)
                    .frame(Frame::NONE)
                    .hint_text("Search songs, albums, podcasts")
                    .desired_width(f32::INFINITY),
            );
            edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
        });
        if !self.search_input.is_empty() {
            let clear = Rect::from_center_size(
                pos2(rect.right() - 16.0, rect.center().y),
                Vec2::splat(22.0),
            );
            let clicked = widgets::place_in(ui, clear, |ui| {
                widgets::icon_button(ui, Icon::Close, 10.0, true, false)
                    .on_hover_text("Clear")
                    .clicked()
            });
            if clicked {
                self.search_input.clear();
            }
        }
        submitted
    }

    fn now_playing_panel(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let ready = self.player_ready();
        let has_track = self.now.track.is_some();
        let enabled = ready && has_track;

        // Cover: smaller in short windows, so "Up next" keeps some room.
        let size = width
            .min(340.0)
            .min((ui.available_height() - 330.0).max(120.0));
        let (cover_rect, _) = ui.allocate_exact_size(vec2(width, size), Sense::hover());
        let cover_rect = Rect::from_center_size(cover_rect.center(), Vec2::splat(size));
        let icon = if self.now.track.as_ref().is_some_and(|t| t.is_episode) {
            Icon::Mic
        } else {
            Icon::Note
        };
        let texture = self.cover.as_ref().filter(|_| has_track).map(|(_, t)| t);
        widgets::paint_cover(ui, cover_rect, texture, icon, 10);
        ui.add_space(12.0);

        // Title and artist.
        match &self.now.track {
            Some(t) => {
                ui.add(
                    egui::Label::new(RichText::new(&t.name).font(theme::bold(18.0)).color(TEXT))
                        .truncate(),
                );
                ui.add(egui::Label::new(RichText::new(&t.artists).color(SUBDUED)).truncate());
                ui.add(egui::Label::new(RichText::new(&t.album).small().color(MUTED)).truncate());
            }
            None => {
                ui.label(
                    RichText::new("Nothing playing")
                        .font(theme::bold(18.0))
                        .color(TEXT),
                );
                ui.label(RichText::new(self.player_status_text()).color(SUBDUED));
                ui.label(RichText::new(" ").small());
            }
        }
        ui.add_space(8.0);

        // Seek bar with times underneath.
        let duration_ms = self.now.track.as_ref().map(|t| t.duration_ms).unwrap_or(0);
        let shown_ms = self
            .seek_drag
            .map(|v| (v * duration_ms as f32) as u32)
            .unwrap_or_else(|| self.now.position_ms());
        let fraction = if duration_ms > 0 {
            shown_ms as f32 / duration_ms as f32
        } else {
            0.0
        };
        let (response, new_value) =
            widgets::thin_slider(ui, fraction, width, enabled && duration_ms > 0);
        if let Some(v) = new_value {
            self.seek_drag = Some(v);
        }
        // Seek once the user lets go of the bar.
        if let Some(v) = self.seek_drag
            && !response.is_pointer_button_down_on()
            && !response.dragged()
        {
            self.seek_drag = None;
            self.seek((v * duration_ms as f32) as u32);
        }
        let (times, _) = ui.allocate_exact_size(vec2(width, 14.0), Sense::hover());
        let font = theme::regular(12.0);
        ui.painter().text(
            times.left_center(),
            egui::Align2::LEFT_CENTER,
            format_time(shown_ms),
            font.clone(),
            SUBDUED,
        );
        ui.painter().text(
            times.right_center(),
            egui::Align2::RIGHT_CENTER,
            format_time(duration_ms),
            font,
            SUBDUED,
        );
        ui.add_space(6.0);

        // Buttons.
        let mut action = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let total = 4.0 * 28.0 + 48.0 + 4.0 * 10.0;
            ui.add_space(((width - total) / 2.0).max(0.0));
            if widgets::icon_button(ui, Icon::Shuffle, 16.0, true, self.config.shuffle)
                .on_hover_text("Shuffle")
                .clicked()
            {
                action = Some(0);
            }
            if widgets::icon_button(ui, Icon::Previous, 16.0, enabled, false)
                .on_hover_text("Previous")
                .clicked()
            {
                action = Some(1);
            }
            if widgets::play_button(ui, 48.0, self.now.playing, enabled)
                .on_hover_text("Play/pause (space)")
                .clicked()
            {
                action = Some(2);
            }
            if widgets::icon_button(ui, Icon::Next, 16.0, enabled, false)
                .on_hover_text("Next")
                .clicked()
            {
                action = Some(3);
            }
            let (icon, tip) = match self.config.repeat {
                Repeat::Off => (Icon::Repeat, "Repeat: off"),
                Repeat::All => (Icon::Repeat, "Repeat: the whole list"),
                Repeat::One => (Icon::RepeatOne, "Repeat: this song"),
            };
            if widgets::icon_button(ui, icon, 16.0, true, self.config.repeat != Repeat::Off)
                .on_hover_text(tip)
                .clicked()
            {
                action = Some(4);
            }
        });
        match action {
            Some(0) => self.set_shuffle(!self.config.shuffle),
            Some(1) => self.previous(),
            Some(2) => self.toggle_play(),
            Some(3) => self.next(),
            Some(4) => self.cycle_repeat(),
            _ => {}
        }
        ui.add_space(4.0);

        // Volume.
        ui.horizontal(|ui| {
            let icon = match self.config.volume {
                v if v <= 0.0 => Icon::VolumeOff,
                v if v < 0.5 => Icon::VolumeLow,
                _ => Icon::VolumeHigh,
            };
            if widgets::icon_button(ui, icon, 16.0, true, false)
                .on_hover_text("Mute")
                .clicked()
            {
                if self.config.volume > 0.0 {
                    self.volume_before_mute = self.config.volume;
                    self.set_volume(0.0);
                } else {
                    self.set_volume(self.volume_before_mute.max(0.1));
                }
                self.config.save();
            }
            let w = ui.available_width();
            let (response, new_volume) = widgets::thin_slider(ui, self.config.volume, w, true);
            if let Some(v) = new_volume {
                self.set_volume(v);
            }
            if response.drag_stopped() || response.clicked() {
                self.config.save();
            }
        });

        ui.add_space(10.0);
        ui.painter().hline(
            ui.max_rect().x_range(),
            ui.cursor().top(),
            Stroke::new(1.0, BORDER),
        );
        ui.add_space(10.0);
        self.up_next(ui);
    }

    fn up_next(&mut self, ui: &mut Ui) {
        let start = self.queue_pos + 1;
        let left = self.queue.len().saturating_sub(start);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("UP NEXT")
                    .font(theme::bold(12.0))
                    .color(MUTED),
            );
            if left > 0 {
                ui.label(RichText::new(left.to_string()).small().color(MUTED));
            }
        });
        if left == 0 {
            ui.label(RichText::new("Nothing queued.").color(MUTED));
            return;
        }
        let mut jump = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (offset, t) in self.queue[start..].iter().take(40).enumerate() {
                    let index = start + offset;
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(6), HOVER);
                    }
                    let queued = offset < self.user_queued;
                    let text = rect.shrink2(vec2(8.0, 3.0));
                    widgets::place(
                        ui,
                        egui::UiBuilder::new()
                            .max_rect(text)
                            .layout(Layout::top_down(Align::Min)),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let color = if t.playable { TEXT } else { MUTED };
                            ui.add(
                                egui::Label::new(RichText::new(&t.name).color(color)).truncate(),
                            );
                            let artist = if queued {
                                format!("Queued · {}", t.artists)
                            } else {
                                t.artists.clone()
                            };
                            let color = if queued { theme::accent() } else { SUBDUED };
                            ui.add(
                                egui::Label::new(RichText::new(artist).small().color(color))
                                    .truncate(),
                            );
                        },
                    );
                    if response.on_hover_text("Click to play").clicked() {
                        jump = Some(index);
                    }
                }
            });
        if let Some(index) = jump {
            self.jump_to(index);
        }
    }

    fn player_status_text(&self) -> String {
        match &self.player_state {
            PlayerState::Off => "".into(),
            PlayerState::Connecting => "Connecting the player…".into(),
            PlayerState::BrowserLogin(_) => "Approve playback in your browser".into(),
            PlayerState::Ready => "Double-click a song to play it".into(),
            PlayerState::Failed(_) => "The player is not connected".into(),
        }
    }

    /// Messages about the player that need the user's attention.
    fn player_banner(&mut self, ui: &mut Ui) {
        let mut retry = false;
        let (fill, text, link) = match &self.player_state {
            PlayerState::BrowserLogin(url) => (
                Color32::from_rgb(0x1d, 0x2f, 0x4f),
                "One more step: a browser tab asks you to allow playback. \
                 This is Spotify's own login for the audio player."
                    .to_string(),
                Some(url.clone()),
            ),
            PlayerState::Failed(e) => (
                Color32::from_rgb(0x4a, 0x1c, 0x26),
                format!("Player: {e}"),
                None,
            ),
            _ => return,
        };
        Frame::new()
            .fill(fill)
            .corner_radius(8)
            .inner_margin(Margin::symmetric(16, 10))
            .outer_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(text).color(TEXT));
                    if let Some(url) = &link {
                        ui.hyperlink_to("Open it again", url);
                    } else if widgets::button(ui, "Retry", ButtonStyle::Ghost, 13.0).clicked() {
                        retry = true;
                    }
                });
            });
        if retry {
            self.start_player();
        }
    }

    // ---------------------------------------------------------------- tabs

    fn side_list(ui: &mut Ui, id: &str, add_contents: impl FnOnce(&mut Ui)) {
        egui::Panel::left(id.to_string())
            .resizable(true)
            .default_size(230.0)
            .size_range(170.0..=380.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .inner_margin(Margin::symmetric(10, 12)),
            )
            .show(ui, |ui| {
                let rect = ui.max_rect().expand2(vec2(10.0, 12.0));
                ui.painter()
                    .vline(rect.right(), rect.y_range(), Stroke::new(1.0, BORDER));
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        add_contents(ui);
                    });
            });
    }

    fn library_tab(&mut self, ui: &mut Ui) {
        let mut open = None;
        let mut reload = false;
        Self::side_list(ui, "library-list", |ui| {
            let liked = TrackSource::LikedSongs;
            let selected = self.library.source.as_ref() == Some(&liked);
            let playing = self.playing_source.as_ref() == Some(&liked);
            if widgets::nav_row(ui, Icon::Heart, "Liked Songs", selected, playing).clicked() {
                open = Some(liked);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("PLAYLISTS")
                        .font(theme::bold(12.0))
                        .color(MUTED),
                );
                if self.playlists_loading {
                    ui.add(egui::Spinner::new().size(12.0).color(MUTED));
                } else if ui
                    .add(
                        egui::Button::new(RichText::new("Reload").small().color(MUTED))
                            .frame(false),
                    )
                    .clicked()
                {
                    reload = true;
                }
            });
            for p in &self.playlists {
                let selected = matches!(&self.library.source,
                    Some(TrackSource::Playlist { id, .. }) if *id == p.id);
                let playing = matches!(&self.playing_source,
                    Some(TrackSource::Playlist { id, .. }) if *id == p.id);
                let response = widgets::nav_row(ui, Icon::Note, &p.name, selected, playing)
                    .on_hover_text(format!("{} · by {} · {} songs", p.name, p.owner, p.total));
                if response.clicked() {
                    open = Some(TrackSource::Playlist {
                        id: p.id.clone(),
                        name: p.name.clone(),
                    });
                }
            }
        });
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(20, 14)),
            )
            .show(ui, |ui| {
                if let Some(action) = self.list_view(ui, Which::Library) {
                    self.apply_list_action(Which::Library, action);
                }
            });
        if reload {
            self.reload_playlists();
        }
        if let Some(source) = open {
            self.open_list(source);
        }
    }

    fn podcasts_tab(&mut self, ui: &mut Ui) {
        if !self.shows_loaded {
            self.load_shows();
        }
        let mut open = None;
        let mut reload = false;
        Self::side_list(ui, "podcast-list", |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("YOUR PODCASTS")
                        .font(theme::bold(12.0))
                        .color(MUTED),
                );
                if self.shows_loading {
                    ui.add(egui::Spinner::new().size(12.0).color(MUTED));
                } else if ui
                    .add(
                        egui::Button::new(RichText::new("Reload").small().color(MUTED))
                            .frame(false),
                    )
                    .clicked()
                {
                    reload = true;
                }
            });
            if self.shows.is_empty() && !self.shows_loading {
                ui.label(
                    RichText::new(
                        "No podcasts yet. Follow some in Spotify, or find them with Search.",
                    )
                    .small()
                    .color(MUTED),
                );
            }
            for s in &self.shows {
                let selected = matches!(&self.podcast.source,
                    Some(TrackSource::Show { id, .. }) if *id == s.id);
                let playing = matches!(&self.playing_source,
                    Some(TrackSource::Show { id, .. }) if *id == s.id);
                let mut response = widgets::nav_row(ui, Icon::Mic, &s.name, selected, playing);
                if !s.publisher.is_empty() {
                    response = response.on_hover_text(format!("{} · {}", s.name, s.publisher));
                }
                if response.clicked() {
                    open = Some(TrackSource::Show {
                        id: s.id.clone(),
                        name: s.name.clone(),
                    });
                }
            }
        });
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(20, 14)),
            )
            .show(ui, |ui| {
                if self.podcast.source.is_none() {
                    ui.add_space(30.0);
                    ui.label(
                        RichText::new("Podcasts")
                            .font(theme::bold(26.0))
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new("Pick a podcast on the left to see its episodes.")
                            .color(SUBDUED),
                    );
                    return;
                }
                if let Some(action) = self.list_view(ui, Which::Podcast) {
                    self.apply_list_action(Which::Podcast, action);
                }
            });
        if reload {
            self.load_shows();
        }
        if let Some(source) = open {
            self.open_list(source);
        }
    }

    fn view(&self, which: Which) -> &ListView {
        match which {
            Which::Library => &self.library,
            Which::Podcast => &self.podcast,
        }
    }

    /// A list's title, buttons and song table.
    fn list_view(&self, ui: &mut Ui, which: Which) -> Option<ListAction> {
        let view = self.view(which);
        let source = view.source.as_ref()?;
        let count = view.tracks.len();
        let episodes = matches!(source, TrackSource::Show { .. });
        let (kind, title) = match source {
            TrackSource::LikedSongs => ("Your Liked Songs", "Liked Songs".to_string()),
            TrackSource::Playlist { name, .. } => ("Playlist", name.clone()),
            TrackSource::Album { name, .. } => ("Album", name.clone()),
            TrackSource::Show { name, .. } => ("Podcast", name.clone()),
        };
        let owner = match source {
            TrackSource::Playlist { id, .. } => self
                .playlists
                .iter()
                .find(|p| p.id == *id)
                .map(|p| format!(" · by {}", p.owner))
                .unwrap_or_default(),
            _ => String::new(),
        };
        let noun = if episodes { "episodes" } else { "songs" };
        let mut action = None;

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(kind.to_uppercase())
                        .font(theme::bold(11.0))
                        .color(theme::accent()),
                );
                ui.add(
                    egui::Label::new(RichText::new(&title).font(theme::bold(26.0)).color(TEXT))
                        .truncate(),
                );
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{count} {noun}{owner}")).color(SUBDUED));
                    if view.loading {
                        ui.add(egui::Spinner::new().size(14.0).color(SUBDUED));
                    }
                });
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let enabled = count > 0;
            ui.add_enabled_ui(enabled, |ui| {
                if widgets::button(ui, "Play", ButtonStyle::Accent, 14.0).clicked() {
                    action = Some(ListAction::PlayAll);
                }
                if !episodes
                    && widgets::button(ui, "Shuffle play", ButtonStyle::Ghost, 14.0).clicked()
                {
                    action = Some(ListAction::ShuffleAll);
                }
            });
            ui.add(
                egui::Label::new(
                    RichText::new("Double-click to play · right-click to add to the queue")
                        .small()
                        .color(MUTED),
                )
                .truncate(),
            );
        });
        if let Some(e) = &view.error {
            ui.label(RichText::new(e).color(ERROR));
        }
        ui.add_space(8.0);

        let labels = if episodes {
            ["SHOW", "RELEASED"]
        } else {
            ["ARTIST", "ALBUM"]
        };
        let (header, _) = ui.allocate_exact_size(
            vec2(ui.available_width(), widgets::HEADER_HEIGHT),
            Sense::hover(),
        );
        widgets::table_header(ui, header, labels);
        ui.add_space(4.0);
        let current = self.now.track.as_ref().map(|t| t.uri.as_str());
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            widgets::ROW_HEIGHT,
            count,
            |ui, range| {
                for i in range {
                    let t = &view.tracks[i];
                    let (rect, _) = ui.allocate_exact_size(
                        vec2(ui.available_width(), widgets::ROW_HEIGHT),
                        Sense::hover(),
                    );
                    let is_current = current == Some(t.uri.as_str());
                    let row = widgets::table_row(
                        ui,
                        rect,
                        i + 1,
                        t,
                        is_current,
                        is_current && self.now.playing,
                    );
                    if row.play {
                        action = Some(ListAction::Play(i));
                    }
                    if row.queue {
                        action = Some(ListAction::Queue(i));
                    }
                }
            },
        );
        action
    }

    fn apply_list_action(&mut self, which: Which, action: ListAction) {
        let view = self.view(which);
        let tracks = view.tracks.clone();
        let source = view.source.clone();
        match action {
            ListAction::Play(i) => self.play_list(tracks, i, source),
            ListAction::Queue(i) => self.add_to_queue(vec![tracks[i].clone()]),
            ListAction::PlayAll => self.play_list(tracks, 0, source),
            ListAction::ShuffleAll => {
                if !self.config.shuffle {
                    self.config.shuffle = true;
                    self.config.save();
                }
                let start = rand::rng().random_range(0..tracks.len().max(1));
                self.play_list(tracks, start, source);
            }
        }
    }

    fn search_tab(&mut self, ui: &mut Ui) {
        let mut play = None;
        let mut queue = None;
        let mut open = None;
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(20, 14)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if self.search.query.is_empty() {
                            ui.add_space(30.0);
                            ui.label(RichText::new("Search").font(theme::bold(26.0)).color(TEXT));
                            ui.label(
                                RichText::new("Type in the box at the top right and press Enter.")
                                    .color(SUBDUED),
                            );
                            return;
                        }
                        ui.label(
                            RichText::new("RESULTS FOR")
                                .font(theme::bold(11.0))
                                .color(theme::accent()),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.search.query)
                                    .font(theme::bold(26.0))
                                    .color(TEXT),
                            )
                            .truncate(),
                        );
                        if self.search.loading {
                            ui.add(egui::Spinner::new().size(22.0).color(theme::accent()));
                        }
                        if let Some(e) = &self.search.error {
                            ui.label(RichText::new(e).color(ERROR));
                        }
                        let Some(results) = &self.search.results else {
                            return;
                        };

                        section_title(ui, "Songs");
                        if results.tracks.is_empty() {
                            ui.label(RichText::new("No songs found").color(SUBDUED));
                        } else {
                            let (header, _) = ui.allocate_exact_size(
                                vec2(ui.available_width(), widgets::HEADER_HEIGHT),
                                Sense::hover(),
                            );
                            widgets::table_header(ui, header, ["ARTIST", "ALBUM"]);
                            ui.add_space(4.0);
                            let current = self.now.track.as_ref().map(|t| t.uri.as_str());
                            ui.scope(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for (i, t) in results.tracks.iter().enumerate() {
                                    let (rect, _) = ui.allocate_exact_size(
                                        vec2(ui.available_width(), widgets::ROW_HEIGHT),
                                        Sense::hover(),
                                    );
                                    let is_current = current == Some(t.uri.as_str());
                                    let row = widgets::table_row(
                                        ui,
                                        rect,
                                        i + 1,
                                        t,
                                        is_current,
                                        is_current && self.now.playing,
                                    );
                                    if row.play {
                                        play = Some(i);
                                    }
                                    if row.queue {
                                        queue = Some(i);
                                    }
                                }
                            });
                        }

                        ui.add_space(18.0);
                        ui.columns(3, |cols| {
                            section_title(&mut cols[0], "Albums");
                            empty_hint(&mut cols[0], results.albums.is_empty(), "No albums found");
                            for a in &results.albums {
                                if result_row(&mut cols[0], Icon::Disc, &a.name, &a.artists)
                                    .clicked()
                                {
                                    open = Some(TrackSource::Album {
                                        id: a.id.clone(),
                                        name: a.name.clone(),
                                    });
                                }
                            }
                            section_title(&mut cols[1], "Playlists");
                            empty_hint(
                                &mut cols[1],
                                results.playlists.is_empty(),
                                "No playlists found",
                            );
                            for p in &results.playlists {
                                if result_row(
                                    &mut cols[1],
                                    Icon::Note,
                                    &p.name,
                                    &format!("by {}", p.owner),
                                )
                                .clicked()
                                {
                                    open = Some(TrackSource::Playlist {
                                        id: p.id.clone(),
                                        name: p.name.clone(),
                                    });
                                }
                            }
                            section_title(&mut cols[2], "Podcasts");
                            empty_hint(&mut cols[2], results.shows.is_empty(), "No podcasts found");
                            for s in &results.shows {
                                let subtitle = if s.publisher.is_empty() {
                                    "Podcast"
                                } else {
                                    &s.publisher
                                };
                                if result_row(&mut cols[2], Icon::Mic, &s.name, subtitle).clicked()
                                {
                                    open = Some(TrackSource::Show {
                                        id: s.id.clone(),
                                        name: s.name.clone(),
                                    });
                                }
                            }
                        });
                    });
            });
        let tracks = self
            .search
            .results
            .as_ref()
            .map(|r| r.tracks.clone())
            .unwrap_or_default();
        if let Some(i) = play {
            self.play_list(tracks.clone(), i, None);
        }
        if let Some(i) = queue {
            self.add_to_queue(vec![tracks[i].clone()]);
        }
        if let Some(source) = open {
            self.open_list(source);
        }
    }

    fn fun_tab(&mut self, ui: &mut Ui) {
        let Some(mut fun) = self.fun.take() else {
            self.tab = Tab::Library;
            return;
        };
        if fun.needs_pool() {
            let source = fun.pool_source().clone();
            self.fun = Some(fun);
            self.load_fun_pool(source);
            fun = self.fun.take().expect("just put back");
        }
        let mut actions = Vec::new();
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG))
            .show(ui, |ui| {
                actions = fun.ui(ui, &self.playlists, self.config.fun_autoplay);
            });
        self.fun = Some(fun);
        for action in actions {
            self.on_fun_action(action);
        }
    }

    fn settings_tab(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut fun_enabled = self.config.fun_enabled;
        let mut sign_out = false;
        let mut change_id = false;
        let mut reload = false;
        let mut retry = false;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(20, 14)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.set_max_width(640.0);
                    ui.label(RichText::new("Settings").font(theme::bold(26.0)).color(TEXT));
                    ui.add_space(10.0);

                    settings_card(ui, "Look", |ui| {
                        ui.label(RichText::new("Accent colour").color(SUBDUED));
                        ui.horizontal(|ui| {
                            for (name, color) in theme::ACCENTS {
                                let selected = self.config.accent == name;
                                let (rect, response) =
                                    ui.allocate_exact_size(vec2(64.0, 58.0), Sense::click());
                                let center = pos2(rect.center().x, rect.top() + 18.0);
                                if selected {
                                    ui.painter().circle_stroke(center, 18.0, Stroke::new(2.0, TEXT));
                                }
                                let r = if response.hovered() { 15.0 } else { 14.0 };
                                ui.painter().circle_filled(center, r, color);
                                ui.painter().text(
                                    pos2(rect.center().x, rect.bottom() - 8.0),
                                    egui::Align2::CENTER_CENTER,
                                    name,
                                    theme::regular(12.0),
                                    if selected { TEXT } else { SUBDUED },
                                );
                                if response.clicked() {
                                    self.config.accent = name.to_string();
                                    theme::set_accent(&ctx, name);
                                    self.config.save();
                                }
                            }
                        });
                    });

                    settings_card(ui, "Fun tab", |ui| {
                        ui.checkbox(&mut fun_enabled, "Show the Fun tab");
                        ui.label(
                            RichText::new(
                                "Song Roulette, the Claw Machine and the other pickers. They are \
                                 drawn by the app itself and use almost no memory; while the tab \
                                 is turned off they use none at all.",
                            )
                            .small()
                            .color(MUTED),
                        );
                        ui.add_space(6.0);
                        if ui
                            .checkbox(
                                &mut self.config.fun_autoplay,
                                "Play what a game picks straight away",
                            )
                            .changed()
                        {
                            self.config.save();
                        }
                    });

                    settings_card(ui, "Playback", |ui| {
                        let status = match &self.player_state {
                            PlayerState::Ready => "Connected".to_string(),
                            PlayerState::Connecting => "Connecting…".to_string(),
                            PlayerState::BrowserLogin(_) => "Waiting for you to approve it in the browser".to_string(),
                            PlayerState::Failed(e) => format!("Not connected: {e}"),
                            PlayerState::Off => "Off".to_string(),
                        };
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("Player:").color(SUBDUED));
                            ui.label(RichText::new(status).color(TEXT));
                            if matches!(self.player_state, PlayerState::Failed(_))
                                && widgets::button(ui, "Retry", ButtonStyle::Ghost, 13.0).clicked()
                            {
                                retry = true;
                            }
                        });
                        ui.label(
                            RichText::new("Music streams at 320 kbit/s, the best quality Spotify offers. Nothing is saved to disk.")
                                .small()
                                .color(MUTED),
                        );
                    });

                    settings_card(ui, "Account", |ui| {
                        let who = if self.user_name.is_empty() {
                            "Signed in".to_string()
                        } else {
                            format!("Signed in as {}", self.user_name)
                        };
                        ui.label(RichText::new(who).color(TEXT));
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if widgets::button(ui, "Reload playlists", ButtonStyle::Ghost, 13.0).clicked() {
                                reload = true;
                            }
                            if widgets::button(ui, "Sign out", ButtonStyle::Ghost, 13.0).clicked() {
                                sign_out = true;
                            }
                            if widgets::button(ui, "Use a different Client ID", ButtonStyle::Ghost, 13.0).clicked() {
                                change_id = true;
                            }
                        });
                    });

                    settings_card(ui, "About", |ui| {
                        ui.label(
                            RichText::new(concat!("lite-spotify ", env!("CARGO_PKG_VERSION")))
                                .color(TEXT),
                        );
                        ui.label(
                            RichText::new(format!("Settings and log file: {}", config::app_dir().display()))
                                .small()
                                .color(MUTED),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if widgets::button(ui, "Open settings folder", ButtonStyle::Ghost, 13.0).clicked() {
                                let _ = open::that_detached(config::app_dir());
                            }
                            if widgets::button(ui, "Open log file", ButtonStyle::Ghost, 13.0).clicked() {
                                let _ = open::that_detached(config::log_file());
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Uses the Inter font by Rasmus Andersson (SIL Open Font License).")
                                .small()
                                .color(MUTED),
                        );
                    });
                });
            });
        if fun_enabled != self.config.fun_enabled {
            self.set_fun_enabled(fun_enabled);
        }
        if retry {
            self.start_player();
        }
        if reload {
            self.reload_playlists();
            if self.shows_loaded {
                self.load_shows();
            }
        }
        if sign_out || change_id {
            self.sign_out();
        }
        if change_id {
            self.screen = Screen::Setup {
                input: self.config.client_id.clone(),
                error: None,
            };
        }
    }

    /// Short messages at the bottom of the window.
    fn toast(&mut self, ctx: &egui::Context) {
        let Some((text, since)) = &self.notice else {
            return;
        };
        if since.elapsed() > NOTICE_TIME {
            self.notice = None;
            return;
        }
        ctx.request_repaint_after(NOTICE_TIME.saturating_sub(since.elapsed()));
        let text = text.clone();
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(-150.0, -24.0))
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                let response = Frame::new()
                    .fill(RAISED)
                    .stroke(Stroke::new(1.0, theme::accent()))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).color(TEXT));
                    })
                    .response;
                if response.interact(Sense::click()).clicked() {
                    self.notice = None;
                }
            });
    }
}

fn tab_button(ui: &mut Ui, label: &str, selected: bool, bar_bottom: f32) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), theme::bold(14.0), Color32::PLACEHOLDER);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(20.0, 20.0), Sense::click());
    let color = if selected || response.hovered() {
        TEXT
    } else {
        SUBDUED
    };
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_max(
                pos2(rect.left() + 8.0, bar_bottom - 3.0),
                pos2(rect.right() - 8.0, bar_bottom),
            ),
            CornerRadius::same(2),
            theme::accent(),
        );
    }
    response
}

/// A numbered step on the setup screen.
fn step(ui: &mut Ui, number: u32, add_contents: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 12.0, theme::accent());
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            number.to_string(),
            theme::bold(13.0),
            theme::ON_ACCENT,
        );
        ui.add_space(4.0);
        add_contents(ui);
    });
    ui.add_space(6.0);
}

fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).font(theme::bold(18.0)).color(TEXT));
    ui.add_space(4.0);
}

fn empty_hint(ui: &mut Ui, empty: bool, text: &str) {
    if empty {
        ui.label(RichText::new(text).color(SUBDUED));
    }
}

/// An album, playlist or podcast in the search results.
fn result_row(ui: &mut Ui, icon: Icon, title: &str, subtitle: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), HOVER);
    }
    let tile = Rect::from_min_size(rect.min + vec2(6.0, 6.0), Vec2::splat(36.0));
    widgets::paint_cover(ui, tile, None, icon, 6);
    let text = Rect::from_min_max(
        pos2(tile.right() + 10.0, rect.top() + 6.0),
        pos2(rect.right() - 6.0, rect.bottom() - 6.0),
    );
    widgets::place(
        ui,
        egui::UiBuilder::new()
            .max_rect(text)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add(egui::Label::new(RichText::new(title).color(TEXT)).truncate());
            ui.add(egui::Label::new(RichText::new(subtitle).small().color(SUBDUED)).truncate());
        },
    );
    response
}

fn settings_card(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10)
        .inner_margin(Margin::same(16))
        .outer_margin(Margin {
            left: 0,
            right: 0,
            top: 0,
            bottom: 12,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(theme::bold(16.0)).color(TEXT));
            ui.add_space(6.0);
            add_contents(ui);
        });
}
