//! The screens: setup and sign-in, then the main window with its tabs, the
//! now-playing panel on the right and the top bar.

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Key, Layout, Margin, Rect, RichText, Sense, Stroke,
    Ui, Vec2, pos2, vec2,
};
use rand::Rng;

use std::time::{Duration, Instant};

use super::{App, Dialog, ListView, NOTICE_TIME, PlayerState, Screen, Sleep, Tab};
use crate::api::{Track, TrackSource};
use crate::auth;
use crate::config::{self, Repeat};
use crate::stats;
use crate::theme::{self, bg, border, hover, muted, raised, subdued, surface};
use crate::widgets::{self, ButtonStyle, Icon, RowAction, RowMenu, RowState, format_time};

/// What the user did in a song list.
enum ListAction {
    Play(usize),
    Queue(usize),
    Like(usize),
    PlayAll,
    ShuffleAll,
    AddTo(usize, String),
    NewPlaylistWith(usize),
    Remove(usize),
    Rename,
}

/// Quick searches on the Search tab, for when you don't know what to look for.
const SEARCH_IDEAS: [&str; 14] = [
    "Chill",
    "Workout",
    "Focus",
    "Party",
    "Sleep",
    "Lo-fi",
    "Rock",
    "Hip hop",
    "Jazz",
    "Acoustic",
    "80s",
    "90s",
    "Summer",
    "Road trip",
];

#[derive(Clone, Copy, PartialEq)]
enum Which {
    Library,
    Podcast,
}

/// The keyboard shortcuts, as listed in Settings.
const SHORTCUTS: [(&str, &str); 10] = [
    ("Space", "Play / pause"),
    ("Ctrl + \u{2192} / \u{2190}", "Next / previous song"),
    ("\u{2192} / \u{2190}", "Forward / back 10 seconds"),
    ("+ / \u{2212}", "Volume up / down"),
    ("M", "Mute"),
    ("L", "Save the song to Liked Songs (or remove it)"),
    ("S", "Shuffle on / off"),
    ("R", "Repeat: off, list, song"),
    ("Ctrl + F", "Search"),
    ("Ctrl + 1 \u{2026} 6", "Switch tabs"),
];

impl eframe::App for App {
    /// Runs before every frame, and also while the window is minimised, so the
    /// next song still starts and the sleep timer still works.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_messages(ctx);
        self.track_listening();
        if self.tab != Tab::Stats {
            self.stats = None;
        }
        self.update_lyrics();
        self.tick_sleep(ctx);
        if self.tab != Tab::Fun || self.lyrics_open {
            // Leaving the Fun tab ends Guess the Song.
            self.end_quiz();
        }
        self.tick_quiz(ctx);
        self.update_media_popup();
        self.update_tray();
        self.update_discord();
        self.handle_close(ctx);
        if self.now.playing {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        match self.screen {
            Screen::Setup { .. } => self.setup_screen(ui),
            Screen::SignIn { .. } => self.sign_in_screen(ui),
            Screen::Main => {
                self.keyboard_shortcuts(&ctx);
                self.main_screen(ui);
                // Keep the seek bar moving; otherwise only redraw when something happens.
                if self.now.playing {
                    ctx.request_repaint_after(Duration::from_millis(500));
                }
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(p) = &self.player {
            p.stop_connect();
        }
        self.config.save();
        self.save_session();
        self.finish_listen();
    }
}

impl App {
    /// See [`SHORTCUTS`]. Keys are only used while the user is not typing, and
    /// are consumed so a focused button does not also react to them.
    fn keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.dialog.is_some() {
            return;
        }
        use egui::Modifiers as M;
        let pressed = |modifiers: M, key: Key| ctx.input_mut(|i| i.consume_key(modifiers, key));
        if pressed(M::COMMAND, Key::F) {
            self.tab = Tab::Search;
            self.focus_search = true;
        }
        let tabs = [
            (Key::Num1, Tab::Library),
            (Key::Num2, Tab::Search),
            (Key::Num3, Tab::Podcasts),
            (Key::Num4, Tab::Fun),
            (Key::Num5, Tab::Stats),
            (Key::Num6, Tab::Settings),
        ];
        for (key, tab) in tabs {
            if pressed(M::COMMAND, key) && (tab != Tab::Fun || self.fun.is_some()) {
                self.tab = tab;
            }
        }
        if pressed(M::NONE, Key::Space) {
            self.toggle_play();
        }
        let has_track = self.now.track.is_some();
        if pressed(M::COMMAND, Key::ArrowRight) && has_track {
            self.next();
        }
        if pressed(M::COMMAND, Key::ArrowLeft) && has_track {
            self.previous();
        }
        // The claw machine on the Fun tab uses the arrow keys itself.
        if self.tab != Tab::Fun {
            if pressed(M::NONE, Key::ArrowRight) {
                self.seek_by(10);
            }
            if pressed(M::NONE, Key::ArrowLeft) {
                self.seek_by(-10);
            }
        }
        let mut volume = None;
        if pressed(M::NONE, Key::Plus) || pressed(M::NONE, Key::Equals) {
            volume = Some((self.config.volume + 0.05).min(1.0));
        }
        if pressed(M::NONE, Key::Minus) {
            volume = Some((self.config.volume - 0.05).max(0.0));
        }
        if pressed(M::NONE, Key::M) {
            volume = Some(self.toggled_mute());
        }
        if let Some(v) = volume {
            self.set_volume(v);
            self.config.save();
        }
        if pressed(M::NONE, Key::L)
            && let Some(track) = self.now.track.clone()
        {
            self.toggle_like(track);
        }
        if pressed(M::NONE, Key::S) {
            self.set_shuffle(!self.config.shuffle);
        }
        if pressed(M::NONE, Key::R) {
            self.cycle_repeat();
        }
    }

    /// The volume after pressing mute: 0, or what it was before muting.
    fn toggled_mute(&mut self) -> f32 {
        if self.config.volume > 0.0 {
            self.volume_before_mute = self.config.volume;
            0.0
        } else {
            self.volume_before_mute.max(0.1)
        }
    }

    // ---------------------------------------------------------- first start

    /// A card in the middle of the window, used by the setup and sign-in screens.
    fn centered_card(ui: &mut Ui, width: f32, add_contents: impl FnOnce(&mut Ui)) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(bg()))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let width = width.min(ui.available_width() - 32.0);
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.06).clamp(16.0, 80.0));
                        Frame::new()
                            .fill(surface())
                            .stroke(Stroke::new(1.0, border()))
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
                        .color(theme::text()),
                );
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "lite-spotify connects to Spotify through your own free developer app. \
                     This takes about three minutes and only has to be done once.",
                )
                .color(subdued()),
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
                    .fill(raised())
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(auth::WEB_REDIRECT_URI)
                                .monospace()
                                .color(theme::text()),
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
                ui.label(RichText::new(e.as_str()).color(theme::error()));
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
                .color(muted())
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
                        .color(theme::text()),
                );
                ui.label(
                    RichText::new("A small player for your Spotify Premium account")
                        .color(subdued()),
                );
                ui.add_space(26.0);
                if busy {
                    ui.add(egui::Spinner::new().size(28.0).color(theme::accent()));
                    ui.add_space(12.0);
                    if let Some(url) = &url {
                        ui.label(
                            RichText::new("Finish signing in in your browser,")
                                .color(theme::text()),
                        );
                        ui.label(RichText::new("then come back here.").color(theme::text()));
                        ui.add_space(12.0);
                        ui.hyperlink_to("Browser did not open? Click here.", url);
                        ui.add_space(18.0);
                        if widgets::button(ui, "Cancel", ButtonStyle::Ghost, 14.0).clicked() {
                            cancel = true;
                        }
                    } else {
                        ui.label(RichText::new("Signing in…").color(theme::text()));
                    }
                } else {
                    if let Some(e) = &error {
                        ui.label(RichText::new(e).color(theme::error()));
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
                            RichText::new("Use a different Client ID").color(subdued()),
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
        if self.mini.is_some() {
            self.mini_player(ui);
            return;
        }
        if self.karaoke {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                self.set_karaoke(&ui.ctx().clone(), false);
            }
            self.karaoke_screen(ui);
            self.toast(ui.ctx());
            return;
        }
        egui::Panel::top("top")
            .exact_size(54.0)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(surface())
                    .inner_margin(Margin::symmetric(14, 0)),
            )
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::right("now-playing")
            .resizable(true)
            .default_size(300.0)
            .size_range(230.0..=420.0)
            .frame(Frame::new().fill(surface()).inner_margin(Margin::same(16)))
            .show(ui, |ui| self.now_playing_panel(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(bg()))
            .show(ui, |ui| {
                self.update_banner(ui);
                self.player_banner(ui);
                if self.lyrics_open {
                    self.lyrics_view(ui, false);
                    return;
                }
                match self.tab {
                    Tab::Library => self.library_tab(ui),
                    Tab::Search => self.search_tab(ui),
                    Tab::Podcasts => self.podcasts_tab(ui),
                    Tab::Fun => self.fun_tab(ui),
                    Tab::Stats => self.stats_tab(ui),
                    Tab::Settings => self.settings_tab(ui),
                }
            });
        self.toast(ui.ctx());
        self.dialog_ui(ui.ctx());
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let bar = ui.max_rect();
        ui.painter().hline(
            bar.x_range(),
            bar.bottom() - 0.5,
            Stroke::new(1.0, border()),
        );
        ui.horizontal_centered(|ui| {
            widgets::logo(ui, 26.0);
            ui.label(
                RichText::new("lite-spotify")
                    .font(theme::bold(16.0))
                    .color(theme::text()),
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
            tabs.push((Tab::Stats, "Stats"));
            tabs.push((Tab::Settings, "Settings"));
            for (tab, label) in tabs {
                let selected = self.tab == tab && !self.lyrics_open;
                if tab_button(ui, label, selected, bar.bottom()).clicked() {
                    self.tab = tab;
                    self.lyrics_open = false;
                    if tab == Tab::Search {
                        self.focus_search = true;
                    }
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::MiniPlayer, 18.0, true, false)
                    .on_hover_text("Mini player: a small window that stays on top")
                    .clicked()
                {
                    let ctx = ui.ctx().clone();
                    self.set_mini(&ctx, true);
                }
                // The Search tab has its own big search box.
                if self.tab == Tab::Search {
                    return;
                }
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
            hover()
        } else {
            raised()
        };
        let stroke = if focused {
            Stroke::new(1.5, theme::accent())
        } else {
            Stroke::new(1.0, border())
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
        widgets::paint_icon(ui.painter(), icon, Icon::Search, subdued());
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
            .min((ui.available_height() - 365.0).max(110.0));
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
        let mut add_to: Option<Option<String>> = None;
        let mut like = false;
        match &self.now.track {
            Some(t) => {
                ui.add(
                    egui::Label::new(
                        RichText::new(&t.name)
                            .font(theme::bold(18.0))
                            .color(theme::text()),
                    )
                    .truncate(),
                );
                ui.add(egui::Label::new(RichText::new(&t.artists).color(subdued())).truncate());
                ui.add(egui::Label::new(RichText::new(&t.album).small().color(muted())).truncate());
                ui.add_space(4.0);
                let liked = self.is_liked(&t.uri);
                let mut button = None;
                ui.horizontal(|ui| {
                    let tip = if liked {
                        "Remove from Liked Songs (L)"
                    } else {
                        "Save to Liked Songs (L)"
                    };
                    if widgets::heart_button(ui, 18.0, liked)
                        .on_hover_text(tip)
                        .clicked()
                    {
                        like = true;
                    }
                    button = Some(widgets::button(
                        ui,
                        "+ Add to playlist",
                        ButtonStyle::Ghost,
                        12.0,
                    ));
                });
                let button = button.expect("set above");
                let playlists = self.own_playlists();
                egui::Popup::menu(&button).show(|ui| {
                    ui.set_min_width(200.0);
                    if ui.button("New playlist…").clicked() {
                        add_to = Some(None);
                    }
                    if !playlists.is_empty() {
                        ui.separator();
                    }
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for (id, name) in &playlists {
                                if ui.button(name.as_str()).clicked() {
                                    add_to = Some(Some(id.clone()));
                                }
                            }
                        });
                });
            }
            None => {
                ui.label(
                    RichText::new("Nothing playing")
                        .font(theme::bold(18.0))
                        .color(theme::text()),
                );
                ui.label(RichText::new(self.player_status_text()).color(subdued()));
                ui.label(RichText::new(" ").small());
            }
        }
        if like && let Some(track) = self.now.track.clone() {
            self.toggle_like(track);
        }
        if let (Some(target), Some(track)) = (add_to, self.now.track.clone()) {
            match target {
                Some(id) => self.add_to_playlist(id, vec![track]),
                None => {
                    if self.check_can_edit() {
                        self.dialog = Some(Dialog::NewPlaylist {
                            name: String::new(),
                            tracks: vec![track],
                        });
                    }
                }
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
            subdued(),
        );
        ui.painter().text(
            times.right_center(),
            egui::Align2::RIGHT_CENTER,
            format_time(duration_ms),
            font,
            subdued(),
        );
        ui.add_space(6.0);

        // Buttons.
        let mut action = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let total = 4.0 * 28.0 + 48.0 + 4.0 * 10.0;
            ui.add_space(((width - total) / 2.0).max(0.0));
            if widgets::icon_button(ui, Icon::Shuffle, 16.0, true, self.config.shuffle)
                .on_hover_text("Shuffle (S)")
                .clicked()
            {
                action = Some(0);
            }
            if widgets::icon_button(ui, Icon::Previous, 16.0, enabled, false)
                .on_hover_text("Previous (Ctrl + \u{2190})")
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
                .on_hover_text("Next (Ctrl + \u{2192})")
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
                .on_hover_text("Mute (M)")
                .clicked()
            {
                let v = self.toggled_mute();
                self.set_volume(v);
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

        ui.add_space(6.0);
        self.sleep_timer_button(ui);
        ui.add_space(8.0);
        ui.painter().hline(
            ui.max_rect().x_range(),
            ui.cursor().top(),
            Stroke::new(1.0, border()),
        );
        ui.add_space(10.0);
        self.up_next(ui);
    }

    /// "Lyrics" and "Sleep timer" under the volume.
    fn sleep_timer_button(&mut self, ui: &mut Ui) {
        let lyrics_label = if self.lyrics_open {
            "Hide lyrics"
        } else {
            "Lyrics"
        };
        let label = match self.sleep {
            None => "Sleep timer".to_string(),
            Some(Sleep::EndOfSong) => "Sleep: after this song".to_string(),
            Some(Sleep::At(t)) => {
                let left = t.saturating_duration_since(Instant::now());
                format!("Sleep in {}", format_time(left.as_millis() as u32 + 999))
            }
        };
        let mut chosen: Option<Option<Sleep>> = None;
        ui.horizontal(|ui| {
            if widgets::button(ui, lyrics_label, ButtonStyle::Ghost, 12.0)
                .on_hover_text("The words of the song, line by line")
                .clicked()
            {
                self.lyrics_open = !self.lyrics_open;
            }
            let button = widgets::button(ui, &label, ButtonStyle::Ghost, 12.0)
                .on_hover_text("Pause the music after a while");
            egui::Popup::menu(&button).show(|ui| {
                ui.set_min_width(170.0);
                for minutes in [15, 30, 45, 60, 90] {
                    if ui.button(format!("In {minutes} minutes")).clicked() {
                        let at = Instant::now() + Duration::from_secs(minutes * 60);
                        chosen = Some(Some(Sleep::At(at)));
                    }
                }
                if ui.button("After this song").clicked() {
                    chosen = Some(Some(Sleep::EndOfSong));
                }
                if self.sleep.is_some() {
                    ui.separator();
                    if ui.button("Turn off").clicked() {
                        chosen = Some(None);
                    }
                }
            });
            if self.sleep.is_some()
                && widgets::icon_button(ui, Icon::Close, 10.0, true, false)
                    .on_hover_text("Turn the sleep timer off")
                    .clicked()
            {
                chosen = Some(None);
            }
        });
        if let Some(sleep) = chosen {
            self.set_sleep(sleep);
            match sleep {
                Some(Sleep::At(_)) => {
                    self.notify("Sleep timer is on. The music fades out at the end.")
                }
                Some(Sleep::EndOfSong) => self.notify("The music stops after this song."),
                None => self.notify("Sleep timer is off."),
            }
        }
        if matches!(self.sleep, Some(Sleep::At(_))) {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
    }

    fn up_next(&mut self, ui: &mut Ui) {
        if self.remote {
            ui.label(
                RichText::new("SPOTIFY CONNECT")
                    .font(theme::bold(12.0))
                    .color(muted()),
            );
            ui.label(
                RichText::new(
                    "Another device is playing music here. The buttons above control it; \
                     change the queue on that device. Play any song here to take over.",
                )
                .color(subdued()),
            );
            return;
        }
        let start = self.queue_pos + 1;
        let left = self.queue.len().saturating_sub(start);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("UP NEXT")
                    .font(theme::bold(12.0))
                    .color(muted()),
            );
            if left > 0 {
                ui.label(RichText::new(left.to_string()).small().color(muted()));
            }
        });
        if left == 0 {
            ui.label(RichText::new("Nothing queued.").color(muted()));
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
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(6), hover());
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
                            let color = if t.playable { theme::text() } else { muted() };
                            ui.add(
                                egui::Label::new(RichText::new(&t.name).color(color)).truncate(),
                            );
                            let artist = if queued {
                                format!("Queued · {}", t.artists)
                            } else {
                                t.artists.clone()
                            };
                            let color = if queued { theme::accent() } else { subdued() };
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

    /// The lyrics of the current song. The line being sung is highlighted and
    /// kept in the middle; click a line to jump to it.
    fn lyrics_view(&mut self, ui: &mut Ui, karaoke: bool) {
        let ctx = ui.ctx().clone();
        let (title, artist) = match &self.now.track {
            Some(t) => (t.name.clone(), t.artists.clone()),
            None => ("Nothing playing".to_string(), String::new()),
        };
        let mut close = false;
        let mut toggle_karaoke = false;
        let margin = if karaoke { 40 } else { 20 };
        Frame::new()
            .inner_margin(Margin::symmetric(margin, 14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("LYRICS")
                                .font(theme::bold(11.0))
                                .color(theme::accent()),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(&title)
                                    .font(theme::bold(22.0))
                                    .color(theme::text()),
                            )
                            .truncate(),
                        );
                        ui.label(RichText::new(&artist).color(subdued()));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if widgets::icon_button(ui, Icon::Close, 12.0, true, false)
                            .on_hover_text(if karaoke { "Close (Esc)" } else { "Close" })
                            .clicked()
                        {
                            close = true;
                        }
                        let label = if karaoke {
                            "Leave full screen"
                        } else {
                            "Full screen"
                        };
                        if widgets::button(ui, label, ButtonStyle::Ghost, 13.0)
                            .on_hover_text("Karaoke: big lyrics on the whole screen")
                            .clicked()
                        {
                            toggle_karaoke = true;
                        }
                    });
                });
                ui.add_space(8.0);
                self.lyrics_lines(ui, karaoke);
            });
        if toggle_karaoke {
            self.set_karaoke(&ctx, !karaoke);
        }
        if close {
            if karaoke {
                self.set_karaoke(&ctx, false);
            }
            self.lyrics_open = false;
        }
    }

    fn lyrics_lines(&mut self, ui: &mut Ui, karaoke: bool) {
        let center = |ui: &mut Ui, text: RichText| {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| ui.label(text));
        };
        let lyrics = match &self.lyrics {
            _ if self.now.track.is_none() => {
                center(
                    ui,
                    RichText::new("Play a song to see its lyrics.").color(subdued()),
                );
                return;
            }
            None | Some((_, None)) => {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.add(egui::Spinner::new().size(28.0).color(theme::accent()))
                });
                return;
            }
            Some((_, Some(Err(_)))) => {
                center(
                    ui,
                    RichText::new("No lyrics for this song.")
                        .font(theme::bold(18.0))
                        .color(subdued()),
                );
                return;
            }
            Some((_, Some(Ok(lyrics)))) => lyrics,
        };
        let position = self.now.position_ms();
        let current = if lyrics.synced {
            lyrics.lines.iter().rposition(|l| l.start_ms <= position)
        } else {
            None
        };
        let size = if karaoke { 34.0 } else { 22.0 };
        // In karaoke, Spotify's own colours for these lyrics.
        let (text, sung, upcoming) = match lyrics.colors {
            Some([_, t, h]) if karaoke => {
                let t = Color32::from_rgb(t[0], t[1], t[2]);
                let h = Color32::from_rgb(h[0], h[1], h[2]);
                (h, h.gamma_multiply(0.55), t)
            }
            _ => (theme::text(), muted(), subdued()),
        };
        let mut seek_to = None;
        let mut scrolled = self.lyrics_scrolled_to;
        let height = ui.available_height();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = if karaoke { 18.0 } else { 10.0 };
                    ui.add_space(height * 0.25);
                    for (i, line) in lyrics.lines.iter().enumerate() {
                        let color = match current {
                            Some(c) if c == i => text,
                            Some(c) if i < c => sung,
                            _ if lyrics.synced => upcoming,
                            _ => text,
                        };
                        let words = if line.words.trim().is_empty() {
                            "\u{266a}"
                        } else {
                            line.words.as_str()
                        };
                        let font = if current == Some(i) {
                            theme::bold(size)
                        } else {
                            theme::regular(size)
                        };
                        let mut label =
                            egui::Label::new(RichText::new(words).font(font).color(color));
                        if lyrics.synced {
                            label = label.sense(Sense::click());
                        }
                        let response = ui.add(label);
                        if current == Some(i) && scrolled != current {
                            response.scroll_to_me(Some(Align::Center));
                            scrolled = current;
                        }
                        if lyrics.synced
                            && response
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                        {
                            seek_to = Some(line.start_ms);
                        }
                    }
                    ui.add_space(24.0);
                    if !lyrics.synced {
                        ui.label(
                            RichText::new("These lyrics are not timed, so no line is highlighted.")
                                .small()
                                .color(muted()),
                        );
                    }
                    if !lyrics.provider.is_empty() {
                        ui.label(
                            RichText::new(format!("Lyrics provided by {}", lyrics.provider))
                                .small()
                                .color(muted()),
                        );
                    }
                    ui.add_space(height * 0.35);
                });
            });
        self.lyrics_scrolled_to = scrolled;
        if let Some(ms) = seek_to {
            self.seek(ms);
            self.lyrics_scrolled_to = None;
        }
        if self.now.playing {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }

    /// Full-screen karaoke: the lyrics on the song's own colour, with small
    /// play controls at the bottom.
    fn karaoke_screen(&mut self, ui: &mut Ui) {
        let background = match &self.lyrics {
            Some((_, Some(Ok(l)))) => l.colors.map(|c| c[0]),
            _ => None,
        }
        .map(|[r, g, b]| Color32::from_rgb(r, g, b))
        .unwrap_or(bg());
        let enabled = self.player_ready() && self.now.track.is_some();
        let mut action = None;
        egui::Panel::bottom("karaoke-controls")
            .exact_size(76.0)
            .show_separator_line(false)
            .frame(Frame::new().fill(background))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let total = 2.0 * 26.0 + 48.0 + 2.0 * 10.0;
                    ui.add_space((ui.available_width() - total) / 2.0);
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if widgets::icon_button(ui, Icon::Previous, 16.0, enabled, false).clicked() {
                        action = Some(1);
                    }
                    if widgets::play_button(ui, 48.0, self.now.playing, enabled).clicked() {
                        action = Some(2);
                    }
                    if widgets::icon_button(ui, Icon::Next, 16.0, enabled, false).clicked() {
                        action = Some(3);
                    }
                });
            });
        egui::CentralPanel::default()
            .frame(Frame::new().fill(background))
            .show(ui, |ui| self.lyrics_view(ui, true));
        match action {
            Some(1) => self.previous(),
            Some(2) => self.toggle_play(),
            Some(3) => self.next(),
            _ => {}
        }
    }

    /// The mini player: cover, song and buttons in a small window on top.
    fn mini_player(&mut self, ui: &mut Ui) {
        let enabled = self.player_ready() && self.now.track.is_some();
        let mut action = None;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(surface()).inner_margin(Margin::same(10)))
            .show(ui, |ui| {
                let full = ui.max_rect();
                let cover_size = full.height().min(96.0);
                let cover = Rect::from_min_size(full.min, Vec2::splat(cover_size));
                let texture = self
                    .cover
                    .as_ref()
                    .filter(|_| self.now.track.is_some())
                    .map(|(_, t)| t);
                widgets::paint_cover(ui, cover, texture, Icon::Note, 8);
                let right = Rect::from_min_max(pos2(cover.right() + 12.0, full.top()), full.max);
                widgets::place(
                    ui,
                    egui::UiBuilder::new()
                        .max_rect(right)
                        .layout(Layout::top_down(Align::Min)),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let (title, artist) = match &self.now.track {
                            Some(t) => (t.name.as_str(), t.artists.as_str()),
                            None => ("Nothing playing", ""),
                        };
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                if widgets::icon_button(ui, Icon::Expand, 13.0, true, false)
                                    .on_hover_text("Back to the full window")
                                    .clicked()
                                {
                                    action = Some(0);
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(title)
                                            .font(theme::bold(15.0))
                                            .color(theme::text()),
                                    )
                                    .truncate(),
                                );
                            });
                        });
                        ui.add(egui::Label::new(RichText::new(artist).color(subdued())).truncate());
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if widgets::icon_button(ui, Icon::Previous, 14.0, enabled, false)
                                .clicked()
                            {
                                action = Some(1);
                            }
                            if widgets::play_button(ui, 36.0, self.now.playing, enabled).clicked() {
                                action = Some(2);
                            }
                            if widgets::icon_button(ui, Icon::Next, 14.0, enabled, false).clicked()
                            {
                                action = Some(3);
                            }
                            if let Some(t) = &self.now.track {
                                let liked = self.is_liked(&t.uri);
                                if widgets::heart_button(ui, 14.0, liked).clicked() {
                                    action = Some(4);
                                }
                            }
                        });
                        // A thin progress line.
                        let duration = self.now.track.as_ref().map_or(0, |t| t.duration_ms);
                        if duration > 0 {
                            let fraction = self.now.position_ms() as f32 / duration as f32;
                            let line = Rect::from_min_size(
                                pos2(right.left(), right.bottom() - 3.0),
                                vec2(right.width(), 3.0),
                            );
                            ui.painter()
                                .rect_filled(line, CornerRadius::same(2), border());
                            let mut done = line;
                            done.set_right(line.left() + line.width() * fraction.clamp(0.0, 1.0));
                            ui.painter()
                                .rect_filled(done, CornerRadius::same(2), theme::accent());
                        }
                    },
                );
            });
        self.toast(ui.ctx());
        match action {
            Some(0) => {
                let ctx = ui.ctx().clone();
                self.set_mini(&ctx, false);
            }
            Some(1) => self.previous(),
            Some(2) => self.toggle_play(),
            Some(3) => self.next(),
            Some(4) => {
                if let Some(t) = self.now.track.clone() {
                    self.toggle_like(t);
                }
            }
            _ => {}
        }
    }

    /// "A new version is out", with a download button.
    fn update_banner(&mut self, ui: &mut Ui) {
        let Some(release) = &self.update else { return };
        let mut close = false;
        Frame::new()
            .fill(theme::accent_soft())
            .stroke(Stroke::new(1.0, theme::accent()))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(16, 8))
            .outer_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("lite-spotify {} is out!", release.version))
                            .font(theme::bold(14.0))
                            .color(theme::text()),
                    );
                    ui.label(
                        RichText::new(concat!("You have ", env!("CARGO_PKG_VERSION"), "."))
                            .color(subdued()),
                    );
                    if widgets::button(ui, "Download", ButtonStyle::Accent, 13.0)
                        .on_hover_text("Downloads the new .exe in your browser. Close this one and start the new one; your settings and login stay.")
                        .clicked()
                    {
                        let _ = open::that_detached(&release.download_url);
                    }
                    if widgets::button(ui, "What's new", ButtonStyle::Ghost, 13.0).clicked() {
                        let _ = open::that_detached(&release.page_url);
                    }
                    if widgets::icon_button(ui, Icon::Close, 10.0, true, false)
                        .on_hover_text("Hide until next start")
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        if close {
            self.update = None;
        }
    }

    /// Messages about the player that need the user's attention.
    fn player_banner(&mut self, ui: &mut Ui) {
        let mut retry = false;
        let (fill, text, link) = match &self.player_state {
            PlayerState::BrowserLogin(url) => (
                theme::selected(),
                "One more step: a browser tab asks you to allow playback. \
                 This is Spotify's own login for the audio player."
                    .to_string(),
                Some(url.clone()),
            ),
            PlayerState::Failed(e) => (
                theme::error().gamma_multiply(0.25),
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
                    ui.label(RichText::new(text).color(theme::text()));
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
                    .fill(surface())
                    .inner_margin(Margin::symmetric(10, 12)),
            )
            .show(ui, |ui| {
                let rect = ui.max_rect().expand2(vec2(10.0, 12.0));
                ui.painter()
                    .vline(rect.right(), rect.y_range(), Stroke::new(1.0, border()));
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
        let mut new_playlist = false;
        Self::side_list(ui, "library-list", |ui| {
            let liked = TrackSource::LikedSongs;
            let selected = self.library.source.as_ref() == Some(&liked);
            let playing = self.playing_source.as_ref() == Some(&liked);
            if widgets::nav_row(ui, Icon::Heart, "Liked Songs", selected, playing).clicked() {
                open = Some(liked);
            }
            if widgets::nav_row(ui, Icon::Plus, "New playlist", false, false).clicked() {
                new_playlist = true;
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("PLAYLISTS")
                        .font(theme::bold(12.0))
                        .color(muted()),
                );
                if self.playlists_loading {
                    ui.add(egui::Spinner::new().size(12.0).color(muted()));
                } else if ui
                    .add(
                        egui::Button::new(RichText::new("Reload").small().color(muted()))
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
                    .fill(bg())
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
        if new_playlist && self.check_can_edit() {
            self.dialog = Some(Dialog::NewPlaylist {
                name: String::new(),
                tracks: Vec::new(),
            });
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
                        .color(muted()),
                );
                if self.shows_loading {
                    ui.add(egui::Spinner::new().size(12.0).color(muted()));
                } else if ui
                    .add(
                        egui::Button::new(RichText::new("Reload").small().color(muted()))
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
                    .color(muted()),
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
                    .fill(bg())
                    .inner_margin(Margin::symmetric(20, 14)),
            )
            .show(ui, |ui| {
                if self.podcast.source.is_none() {
                    ui.add_space(30.0);
                    ui.label(
                        RichText::new("Podcasts")
                            .font(theme::bold(26.0))
                            .color(theme::text()),
                    );
                    ui.label(
                        RichText::new("Pick a podcast on the left to see its episodes.")
                            .color(subdued()),
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

    /// The menu for song rows: the user's playlists, and "Remove" when `source`
    /// is a playlist of their own.
    fn row_menu(&self, source: Option<&TrackSource>) -> RowMenu {
        let playlists = self.own_playlists();
        let can_remove = matches!(source, Some(TrackSource::Playlist { id, .. })
            if playlists.iter().any(|(own, _)| own == id));
        RowMenu {
            playlists,
            can_remove,
        }
    }

    /// Turns what happened in a song row into a list action.
    fn row_to_action(row: RowAction, i: usize) -> Option<ListAction> {
        if row.play {
            Some(ListAction::Play(i))
        } else if row.queue {
            Some(ListAction::Queue(i))
        } else if row.like {
            Some(ListAction::Like(i))
        } else if let Some(id) = row.add_to {
            Some(ListAction::AddTo(i, id))
        } else if row.new_playlist {
            Some(ListAction::NewPlaylistWith(i))
        } else if row.remove {
            Some(ListAction::Remove(i))
        } else {
            None
        }
    }

    fn view_mut(&mut self, which: Which) -> &mut ListView {
        match which {
            Which::Library => &mut self.library,
            Which::Podcast => &mut self.podcast,
        }
    }

    /// A list's title, buttons, filter box and song table.
    fn list_view(&mut self, ui: &mut Ui, which: Which) -> Option<ListAction> {
        self.view_mut(which).update_shown();
        let view = self.view(which);
        let source = view.source.as_ref()?;
        let count = view.tracks.len();
        let shown = view.shown.len();
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
        let menu = self.row_menu(Some(source));
        let noun = if episodes { "episodes" } else { "songs" };
        let mut action = None;
        let mut filter = view.filter.clone();
        let mut new_sort = None;

        ui.label(
            RichText::new(kind.to_uppercase())
                .font(theme::bold(11.0))
                .color(theme::accent()),
        );
        ui.add(
            egui::Label::new(
                RichText::new(&title)
                    .font(theme::bold(26.0))
                    .color(theme::text()),
            )
            .truncate(),
        );
        ui.horizontal(|ui| {
            let counted = if shown == count {
                format!("{count} {noun}{owner}")
            } else {
                format!("{shown} of {count} {noun}{owner}")
            };
            ui.label(RichText::new(counted).color(subdued()));
            if view.loading {
                ui.add(egui::Spinner::new().size(14.0).color(subdued()));
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(shown > 0, |ui| {
                if widgets::button(ui, "Play", ButtonStyle::Accent, 14.0).clicked() {
                    action = Some(ListAction::PlayAll);
                }
                if !episodes
                    && widgets::button(ui, "Shuffle play", ButtonStyle::Ghost, 14.0).clicked()
                {
                    action = Some(ListAction::ShuffleAll);
                }
            });
            if menu.can_remove && widgets::button(ui, "Rename", ButtonStyle::Ghost, 14.0).clicked()
            {
                action = Some(ListAction::Rename);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                filter_box(
                    ui,
                    &mut filter,
                    (ui.available_width() * 0.5).clamp(120.0, 240.0),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new("Double-click to play · right-click for more")
                            .small()
                            .color(muted()),
                    )
                    .truncate(),
                );
            });
        });
        if let Some(e) = &view.error {
            ui.label(RichText::new(e).color(theme::error()));
        }
        if menu.can_remove && count == 0 && !view.loading {
            ui.add_space(8.0);
            ui.label(
                RichText::new("This playlist is empty. Right-click any song in the app and choose \"Add to playlist\".")
                    .color(subdued()),
            );
        }
        if count > 0 && shown == 0 {
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("No {noun} match \u{201c}{}\u{201d}.", view.filter))
                    .color(subdued()),
            );
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
        if let Some(sort) = widgets::sortable_header(ui, header, labels, view.sort) {
            new_sort = Some(sort);
        }
        ui.add_space(4.0);
        let current = self.now.track.as_ref().map(|t| t.uri.as_str());
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            widgets::ROW_HEIGHT,
            shown,
            |ui, range| {
                for row_index in range {
                    let i = view.shown[row_index];
                    let t = &view.tracks[i];
                    let (rect, _) = ui.allocate_exact_size(
                        vec2(ui.available_width(), widgets::ROW_HEIGHT),
                        Sense::hover(),
                    );
                    let is_current = current == Some(t.uri.as_str());
                    let state = RowState {
                        is_current,
                        is_playing: is_current && self.now.playing,
                        liked: self.is_liked(&t.uri),
                    };
                    let row = widgets::table_row(ui, rect, i + 1, t, state, &menu);
                    if let Some(a) = Self::row_to_action(row, row_index) {
                        action = Some(a);
                    }
                }
            },
        );
        let view = self.view_mut(which);
        if filter != view.filter {
            view.filter = filter;
        }
        if let Some(sort) = new_sort {
            view.sort = sort;
        }
        action
    }

    /// Carries out `action` on `tracks` (the songs of the list it came from).
    fn apply_action(
        &mut self,
        tracks: Vec<Track>,
        source: Option<TrackSource>,
        action: ListAction,
    ) {
        match action {
            ListAction::Play(i) => self.play_list(tracks, i, source),
            ListAction::Queue(i) => self.add_to_queue(vec![tracks[i].clone()]),
            ListAction::Like(i) => self.toggle_like(tracks[i].clone()),
            ListAction::PlayAll => self.play_list(tracks, 0, source),
            ListAction::ShuffleAll => {
                if !self.config.shuffle {
                    self.config.shuffle = true;
                    self.config.save();
                }
                let start = rand::rng().random_range(0..tracks.len().max(1));
                self.play_list(tracks, start, source);
            }
            ListAction::AddTo(i, id) => self.add_to_playlist(id, vec![tracks[i].clone()]),
            ListAction::NewPlaylistWith(i) => {
                if self.check_can_edit() {
                    self.dialog = Some(Dialog::NewPlaylist {
                        name: String::new(),
                        tracks: vec![tracks[i].clone()],
                    });
                }
            }
            ListAction::Remove(i) => {
                if let Some(TrackSource::Playlist { id, .. }) = source {
                    self.remove_from_playlist(id, tracks[i].clone());
                }
            }
            ListAction::Rename => {
                if let Some(TrackSource::Playlist { id, name }) = source
                    && self.check_can_edit()
                {
                    self.dialog = Some(Dialog::Rename { id, name });
                }
            }
        }
    }

    fn apply_list_action(&mut self, which: Which, action: ListAction) {
        let view = self.view(which);
        // Play in the order shown, and only the songs that match the filter.
        let tracks = view.shown_tracks();
        let source = view.source.clone();
        self.apply_action(tracks, source, action);
    }

    /// The Search tab: a big search box, then recent searches and ideas, or the results.
    fn search_tab(&mut self, ui: &mut Ui) {
        let mut action = None;
        let mut open = None;
        let mut search_for: Option<String> = None;
        let menu = self.row_menu(None);
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(bg())
                    .inner_margin(Margin::symmetric(20, 14)),
            )
            .show(ui, |ui| {
                // The big search box.
                let width = ui.available_width().min(640.0);
                let (rect, _) = ui.allocate_exact_size(vec2(width, 48.0), Sense::hover());
                if self.big_search_box(ui, rect) {
                    search_for = Some(self.search_input.clone());
                }
                ui.add_space(14.0);

                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if self.search.query.is_empty() {
                            if !self.config.recent_searches.is_empty() {
                                section_title(ui, "Recent searches");
                                ui.horizontal_wrapped(|ui| {
                                    for q in &self.config.recent_searches {
                                        if widgets::button(ui, q, ButtonStyle::Ghost, 13.0)
                                            .clicked()
                                        {
                                            search_for = Some(q.clone());
                                        }
                                    }
                                });
                                ui.add_space(8.0);
                            }
                            section_title(ui, "Not sure? Try one of these");
                            ui.horizontal_wrapped(|ui| {
                                for idea in SEARCH_IDEAS {
                                    if mood_chip(ui, idea).clicked() {
                                        search_for = Some(idea.to_string());
                                    }
                                }
                            });
                            return;
                        }

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "Results for \u{201c}{}\u{201d}",
                                    self.search.query
                                ))
                                .font(theme::bold(20.0))
                                .color(theme::text()),
                            );
                            if self.search.loading {
                                ui.add(egui::Spinner::new().size(18.0).color(theme::accent()));
                            }
                        });
                        if let Some(e) = &self.search.error {
                            ui.label(RichText::new(e).color(theme::error()));
                        }
                        let Some(results) = &self.search.results else {
                            return;
                        };

                        section_title(ui, "Songs");
                        if results.tracks.is_empty() {
                            ui.label(RichText::new("No songs found").color(subdued()));
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
                                    let state = RowState {
                                        is_current,
                                        is_playing: is_current && self.now.playing,
                                        liked: self.is_liked(&t.uri),
                                    };
                                    let row = widgets::table_row(ui, rect, i + 1, t, state, &menu);
                                    if let Some(a) = Self::row_to_action(row, i) {
                                        action = Some(a);
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
        if let Some(query) = search_for {
            self.search_input = query;
            self.start_search();
        }
        if let Some(action) = action {
            let tracks = self
                .search
                .results
                .as_ref()
                .map(|r| r.tracks.clone())
                .unwrap_or_default();
            self.apply_action(tracks, None, action);
        }
        if let Some(source) = open {
            self.open_list(source);
        }
    }

    /// The large search box on the Search tab. Returns true when the user presses Enter.
    fn big_search_box(&mut self, ui: &mut Ui, rect: Rect) -> bool {
        let id = egui::Id::unique("big-search");
        if std::mem::take(&mut self.focus_search) {
            ui.memory_mut(|m| m.request_focus(id));
        }
        let focused = ui.memory(|m| m.has_focus(id));
        let stroke = if focused {
            Stroke::new(2.0, theme::accent())
        } else {
            Stroke::new(1.0, border())
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(10),
            raised(),
            stroke,
            egui::StrokeKind::Inside,
        );
        let icon =
            Rect::from_center_size(pos2(rect.left() + 26.0, rect.center().y), Vec2::splat(20.0));
        widgets::paint_icon(
            ui.painter(),
            icon,
            Icon::Search,
            if focused { theme::text() } else { subdued() },
        );
        let text_rect = Rect::from_min_max(
            pos2(rect.left() + 48.0, rect.top()),
            pos2(rect.right() - 110.0, rect.bottom()),
        );
        let mut submitted = widgets::place_in(ui, text_rect, |ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.search_input)
                    .id(id)
                    .frame(Frame::NONE)
                    .font(theme::regular(17.0))
                    .hint_text("Songs, artists, albums, playlists, podcasts…")
                    .desired_width(f32::INFINITY),
            );
            edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
        });
        let button = Rect::from_min_max(
            pos2(rect.right() - 100.0, rect.top() + 8.0),
            pos2(rect.right() - 8.0, rect.bottom() - 8.0),
        );
        let clicked = widgets::place_in(ui, button, |ui| {
            widgets::button_sized(
                ui,
                "Search",
                ButtonStyle::Accent,
                14.0,
                Some(button.width()),
            )
            .clicked()
        });
        submitted |= clicked;
        submitted && !self.search_input.trim().is_empty()
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
            .frame(Frame::new().fill(bg()))
            .show(ui, |ui| {
                actions = fun.ui(ui, &self.playlists, self.config.fun_autoplay);
            });
        self.fun = Some(fun);
        for action in actions {
            self.on_fun_action(action);
        }
    }

    /// Your stats: listening time, top songs and artists, and when you listen.
    fn stats_tab(&mut self, ui: &mut Ui) {
        if self.stats.is_none() {
            self.show_stats(stats::Period::Month);
        }
        let Some(view) = &self.stats else { return };
        let mut period = view.period;
        let mut clear = false;
        let sum = &view.summary;
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(bg())
                    .inner_margin(Margin::symmetric(20, 14)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Your stats")
                                .font(theme::bold(26.0))
                                .color(theme::text()),
                        );
                        ui.horizontal_wrapped(|ui| {
                            for p in stats::Period::ALL {
                                let style = if p == period {
                                    ButtonStyle::Accent
                                } else {
                                    ButtonStyle::Ghost
                                };
                                if widgets::button(ui, p.name(), style, 13.0).clicked() {
                                    period = p;
                                }
                            }
                        });
                        ui.add_space(12.0);
                        if view.plays.is_empty() || sum.plays == 0 {
                            ui.label(
                            RichText::new(
                                "Nothing here yet. Play some music and come back: a song counts \
                                 once you have listened to it for 30 seconds.",
                            )
                            .color(subdued()),
                        );
                            return;
                        }

                        // Big numbers.
                        ui.horizontal_wrapped(|ui| {
                            let tiles = [
                                (stats::format_duration(sum.total_ms), "listening"),
                                (sum.plays.to_string(), "songs played"),
                                (sum.songs.to_string(), "different songs"),
                                (sum.artists.to_string(), "different artists"),
                            ];
                            for (number, label) in tiles {
                                stat_tile(ui, &number, label);
                            }
                        });
                        ui.add_space(14.0);

                        ui.columns(2, |cols| {
                            section_title(&mut cols[0], "Top songs");
                            ranked_list(&mut cols[0], &sum.top_songs);
                            section_title(&mut cols[1], "Top artists");
                            ranked_list(&mut cols[1], &sum.top_artists);
                        });
                        ui.add_space(10.0);

                        section_title(ui, "When you listen");
                        ui.label(RichText::new("By hour of the day").small().color(muted()));
                        let hours: Vec<(String, u64)> = sum
                            .by_hour
                            .iter()
                            .enumerate()
                            .map(|(h, ms)| {
                                (
                                    if h % 3 == 0 {
                                        h.to_string()
                                    } else {
                                        String::new()
                                    },
                                    *ms,
                                )
                            })
                            .collect();
                        bar_chart(ui, &hours, 110.0);
                        ui.add_space(10.0);
                        let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
                        let weekdays: Vec<(String, u64)> = days
                            .iter()
                            .zip(sum.by_weekday)
                            .map(|(d, ms)| (d.to_string(), ms))
                            .collect();
                        bar_chart(ui, &weekdays, 90.0);
                        ui.add_space(16.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                            RichText::new(
                                "Your history is kept only on this computer, in history.jsonl in \
                                 the settings folder.",
                            )
                            .small()
                            .color(muted()),
                        );
                            if widgets::button(ui, "Clear history", ButtonStyle::Ghost, 12.0)
                                .clicked()
                            {
                                clear = true;
                            }
                        });
                    });
            });
        if clear {
            stats::clear();
            self.stats = None;
            self.show_stats(period);
            self.notify("Your listening history is cleared.");
        } else if period != view.period {
            self.show_stats(period);
        }
    }

    fn settings_tab(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut fun_enabled = self.config.fun_enabled;
        let mut sign_out = false;
        let mut change_id = false;
        let mut reload = false;
        let mut retry = false;
        let mut relogin = false;
        let mut restart_player = false;
        let mut discord_changed = false;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(bg()).inner_margin(Margin::symmetric(20, 14)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.set_max_width(640.0);
                    ui.label(RichText::new("Settings").font(theme::bold(26.0)).color(theme::text()));
                    ui.add_space(10.0);

                    settings_card(ui, "Look", |ui| {
                        ui.horizontal(|ui| {
                            for (light, name) in [(false, "Dark"), (true, "Light")] {
                                let style = if self.config.light_theme == light {
                                    ButtonStyle::Accent
                                } else {
                                    ButtonStyle::Ghost
                                };
                                if widgets::button(ui, name, style, 13.0).clicked() {
                                    self.config.light_theme = light;
                                    theme::set_light(&ctx, light);
                                    self.config.save();
                                }
                            }
                        });
                        ui.add_space(8.0);
                        ui.label(RichText::new("Accent colour").color(subdued()));
                        ui.horizontal_wrapped(|ui| {
                            let swatches = theme::ACCENTS
                                .iter()
                                .map(|(n, c)| (*n, Some(*c)))
                                .chain([(theme::ALBUM_ACCENT, None)]);
                            for (name, color) in swatches {
                                let selected = self.config.accent == name;
                                let (rect, response) =
                                    ui.allocate_exact_size(vec2(64.0, 58.0), Sense::click());
                                let center = pos2(rect.center().x, rect.top() + 18.0);
                                if selected {
                                    ui.painter().circle_stroke(center, 18.0, Stroke::new(2.0, theme::text()));
                                }
                                let r = if response.hovered() { 15.0 } else { 14.0 };
                                match color {
                                    Some(color) => {
                                        ui.painter().circle_filled(center, r, color);
                                    }
                                    None => {
                                        // A little colour wheel for "the cover's colour".
                                        for (i, (_, c)) in theme::ACCENTS.iter().enumerate() {
                                            let a0 = i as f32 / 5.0 * std::f32::consts::TAU;
                                            let a1 = (i + 1) as f32 / 5.0 * std::f32::consts::TAU;
                                            let points = std::iter::once(center)
                                                .chain((0..=6).map(|k| {
                                                    let a = a0 + (a1 - a0) * k as f32 / 6.0;
                                                    center + r * vec2(a.cos(), a.sin())
                                                }))
                                                .collect();
                                            ui.painter().add(egui::Shape::convex_polygon(points, *c, Stroke::NONE));
                                        }
                                    }
                                }
                                ui.painter().text(
                                    pos2(rect.center().x, rect.bottom() - 8.0),
                                    egui::Align2::CENTER_CENTER,
                                    name,
                                    theme::regular(12.0),
                                    if selected { theme::text() } else { subdued() },
                                );
                                let response = if color.is_none() {
                                    response.on_hover_text("Takes its colour from the cover of the song that is playing")
                                } else {
                                    response
                                };
                                if response.clicked() {
                                    self.config.accent = name.to_string();
                                    theme::set_accent(&ctx, name);
                                    self.apply_album_accent(&ctx);
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
                            .color(muted()),
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
                            ui.label(RichText::new("Player:").color(subdued()));
                            ui.label(RichText::new(status).color(theme::text()));
                            if matches!(self.player_state, PlayerState::Failed(_))
                                && widgets::button(ui, "Retry", ButtonStyle::Ghost, 13.0).clicked()
                            {
                                retry = true;
                            }
                        });
                        ui.label(
                            RichText::new("Music streams at 320 kbit/s, the best quality Spotify offers. Nothing is saved to disk.")
                                .small()
                                .color(muted()),
                        );
                        ui.add_space(6.0);
                        if ui
                            .checkbox(
                                &mut self.config.autoplay_similar,
                                "Keep playing similar songs when the music runs out",
                            )
                            .changed()
                        {
                            self.config.save();
                        }
                        ui.label(
                            RichText::new(
                                "After a single song, or at the end of a list, lite-spotify adds songs \
                                 in the same style from Spotify's radio and shuffles them.",
                            )
                            .small()
                            .color(muted()),
                        );
                        ui.add_space(6.0);
                        if ui
                            .checkbox(
                                &mut self.config.resume_session,
                                "Continue where I left off when the app starts",
                            )
                            .changed()
                        {
                            self.config.save();
                        }
                        ui.label(
                            RichText::new(
                                "The song you were listening to and the queue come back, paused. \
                                 Press play to carry on.",
                            )
                            .small()
                            .color(muted()),
                        );
                        ui.add_space(6.0);
                        if ui
                            .checkbox(
                                &mut self.config.spotify_connect,
                                "Spotify Connect: play music here from the Spotify app on your phone",
                            )
                            .changed()
                        {
                            self.config.save();
                            restart_player = true;
                        }
                        let connect_note = match (&self.remote_status, self.config.spotify_connect) {
                            (_, false) => "lite-spotify then shows up as a device called \"lite-spotify\" \
                                in the Spotify app. It uses a second connection and player, so a bit \
                                more memory; that's why it is off by default."
                                .to_string(),
                            (Some(Ok(())), true) => "On: pick \"lite-spotify\" under devices in the Spotify app.".to_string(),
                            (Some(Err(e)), true) => format!("Spotify Connect could not start: {e}"),
                            (None, true) => "Starting…".to_string(),
                        };
                        ui.label(RichText::new(connect_note).small().color(muted()));
                        if self.tray.is_some() {
                            ui.add_space(6.0);
                            if ui
                                .checkbox(
                                    &mut self.config.close_to_tray,
                                    "Keep playing in the notification area when I close the window",
                                )
                                .changed()
                            {
                                self.config.save();
                            }
                            ui.label(
                                RichText::new(
                                    "Click the lite-spotify icon next to the clock to bring the window \
                                     back; right-click it for play/pause, next and Quit.",
                                )
                                .small()
                                .color(muted()),
                            );
                        }
                    });

                    settings_card(ui, "Discord status", |ui| {
                        if ui
                            .checkbox(&mut self.config.discord_enabled, "Show the song I'm listening to in Discord")
                            .changed()
                        {
                            self.config.save();
                            discord_changed = true;
                        }
                        ui.label(
                            RichText::new(
                                "Discord needs an \"application\" for this, which you make yourself (free, two minutes): \
                                 open discord.com/developers/applications, click New Application, name it \
                                 lite-spotify (Discord shows this name) and copy its Application ID here.",
                            )
                            .small()
                            .color(muted()),
                        );
                        ui.horizontal(|ui| {
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut self.config.discord_app_id)
                                    .hint_text("Application ID (only numbers)")
                                    .desired_width(240.0),
                            );
                            if edit.lost_focus() {
                                self.config.save();
                                discord_changed = true;
                            }
                            ui.hyperlink_to("Open the Discord developer page", "https://discord.com/developers/applications");
                        });
                        let id = self.config.discord_app_id.trim();
                        if self.config.discord_enabled
                            && (id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()))
                        {
                            ui.label(RichText::new("Paste the Application ID to turn it on.").color(theme::error()));
                        }
                    });

                    settings_card(ui, "Keyboard shortcuts", |ui| {
                        egui::Grid::new("shortcuts").num_columns(2).spacing(vec2(24.0, 6.0)).show(ui, |ui| {
                            for (keys, what) in SHORTCUTS {
                                ui.label(RichText::new(keys).font(theme::bold(13.0)).color(theme::text()));
                                ui.label(RichText::new(what).color(subdued()));
                                ui.end_row();
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "The play, pause, next and previous keys on your keyboard work too, \
                                 and Windows shows the song in its media popup next to the volume.",
                            )
                            .small()
                            .color(muted()),
                        );
                    });

                    settings_card(ui, "Account", |ui| {
                        let who = if self.user_name.is_empty() {
                            "Signed in".to_string()
                        } else {
                            format!("Signed in as {}", self.user_name)
                        };
                        ui.label(RichText::new(who).color(theme::text()));
                        if !self.can_edit_playlists && !self.user_id.is_empty() {
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new("To make playlists and save songs to Liked Songs, sign in once more.")
                                        .color(subdued()),
                                );
                                if widgets::button(ui, "Sign in again", ButtonStyle::Accent, 13.0).clicked() {
                                    relogin = true;
                                }
                            });
                        }
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
                                .color(theme::text()),
                        );
                        if ui
                            .checkbox(
                                &mut self.config.check_updates,
                                "Tell me when a new version is out",
                            )
                            .changed()
                        {
                            self.config.save();
                        }
                        ui.label(
                            RichText::new("Looks on GitHub once when the app starts. Nothing about you is sent.")
                                .small()
                                .color(muted()),
                        );
                        ui.label(
                            RichText::new(format!("Settings and log file: {}", config::app_dir().display()))
                                .small()
                                .color(muted()),
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
                                .color(muted()),
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
        if discord_changed {
            self.set_discord(self.config.discord_enabled);
        }
        if restart_player && self.player.is_some() {
            self.restart_player();
        }
        if relogin {
            self.start_browser_sign_in();
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

    /// The dialog on top of the window: name a new playlist, rename one, or
    /// ask to sign in again.
    fn dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let (title, subtitle) = match dialog {
            Dialog::NewPlaylist { tracks, .. } => (
                "New playlist",
                match tracks.as_slice() {
                    [] => None,
                    [one] => Some(format!("With \"{}\"", one.name)),
                    many => Some(format!("With {} songs", many.len())),
                },
            ),
            Dialog::Rename { .. } => ("Rename playlist", None),
            Dialog::NeedsLogin => ("One more permission", None),
        };
        let renaming = matches!(dialog, Dialog::Rename { .. });
        let mut submit: Option<String> = None;
        let mut sign_in = false;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("dialog"))
            .frame(
                Frame::new()
                    .fill(surface())
                    .stroke(Stroke::new(1.0, border()))
                    .corner_radius(12)
                    .inner_margin(Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new(title).font(theme::bold(20.0)).color(theme::text()));
                if let Some(subtitle) = subtitle {
                    ui.add(egui::Label::new(RichText::new(subtitle).color(subdued())).truncate());
                }
                ui.add_space(10.0);
                match dialog {
                    Dialog::NewPlaylist { name, .. } | Dialog::Rename { name, .. } => {
                        let edit = ui.add(
                            egui::TextEdit::singleline(name)
                                .hint_text("Playlist name")
                                .font(theme::regular(16.0))
                                .margin(vec2(12.0, 10.0))
                                .desired_width(f32::INFINITY),
                        );
                        if !edit.has_focus() && !edit.lost_focus() {
                            edit.request_focus();
                        }
                        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.add_space(14.0);
                        let ok = !name.trim().is_empty();
                        ui.horizontal(|ui| {
                            let label = if renaming { "Rename" } else { "Create" };
                            let clicked = ui
                                .add_enabled_ui(ok, |ui| {
                                    widgets::button(ui, label, ButtonStyle::Accent, 14.0)
                                })
                                .inner
                                .clicked();
                            if (clicked || enter) && ok {
                                submit = Some(name.trim().to_string());
                            }
                            if widgets::button(ui, "Cancel", ButtonStyle::Ghost, 14.0).clicked() {
                                close = true;
                            }
                        });
                    }
                    Dialog::NeedsLogin => {
                        ui.label(
                            RichText::new(
                                "To make playlists and save songs to Liked Songs, lite-spotify needs \
                                 one more permission from Spotify. Sign in again; it only takes a moment.",
                            )
                            .color(subdued()),
                        );
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            if widgets::button(ui, "Sign in again", ButtonStyle::Accent, 14.0).clicked() {
                                sign_in = true;
                            }
                            if widgets::button(ui, "Not now", ButtonStyle::Ghost, 14.0).clicked() {
                                close = true;
                            }
                        });
                    }
                }
            });
        if modal.should_close() {
            close = true;
        }
        if let Some(name) = submit {
            match self.dialog.take() {
                Some(Dialog::NewPlaylist { tracks, .. }) => self.create_playlist(name, tracks),
                Some(Dialog::Rename { id, .. }) => self.rename_playlist(id, name),
                _ => {}
            }
        } else if sign_in {
            self.dialog = None;
            self.start_browser_sign_in();
        } else if close {
            self.dialog = None;
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
                    .fill(raised())
                    .stroke(Stroke::new(1.0, theme::accent()))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).color(theme::text()));
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
        theme::text()
    } else {
        subdued()
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
            theme::on_accent(),
        );
        ui.add_space(4.0);
        add_contents(ui);
    });
    ui.add_space(6.0);
}

/// A coloured tile for a quick search.
fn mood_chip(ui: &mut Ui, text: &str) -> egui::Response {
    const COLORS: [Color32; 7] = [
        Color32::from_rgb(0x8c, 0x3b, 0x5e),
        Color32::from_rgb(0x2d, 0x5d, 0x8a),
        Color32::from_rgb(0x2f, 0x6e, 0x55),
        Color32::from_rgb(0x8a, 0x5a, 0x2b),
        Color32::from_rgb(0x5b, 0x46, 0x8f),
        Color32::from_rgb(0x8a, 0x35, 0x35),
        Color32::from_rgb(0x2e, 0x6f, 0x77),
    ];
    let hash = text
        .bytes()
        .fold(3usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize));
    let color = COLORS[hash % COLORS.len()];
    let (rect, response) = ui.allocate_exact_size(vec2(150.0, 64.0), Sense::click());
    let fill = if response.hovered() {
        color.gamma_multiply(1.25)
    } else {
        color
    };
    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    ui.painter().text(
        rect.left_top() + vec2(14.0, 12.0),
        egui::Align2::LEFT_TOP,
        text,
        theme::bold(16.0),
        Color32::WHITE,
    );
    response
}

/// A small text box that filters a song list as you type.
fn filter_box(ui: &mut Ui, filter: &mut String, width: f32) {
    if !filter.is_empty()
        && widgets::icon_button(ui, Icon::Close, 9.0, true, false)
            .on_hover_text("Clear the filter")
            .clicked()
    {
        filter.clear();
    }
    ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("Filter this list")
            .desired_width(width),
    );
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    widgets::paint_icon(ui.painter(), rect, Icon::Search, subdued());
}

/// A big number with a label under it, on the Stats tab.
fn stat_tile(ui: &mut Ui, number: &str, label: &str) {
    Frame::new()
        .fill(surface())
        .stroke(Stroke::new(1.0, border()))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(18, 12))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_min_width(130.0);
                ui.label(
                    RichText::new(number)
                        .font(theme::bold(24.0))
                        .color(theme::accent()),
                );
                ui.label(RichText::new(label).color(subdued()));
            });
        });
}

/// Numbered top-10 list with a bar for how often each was played.
fn ranked_list(ui: &mut Ui, list: &[crate::stats::Ranked]) {
    let most = list.first().map_or(1, |r| r.plays.max(1));
    for (i, item) in list.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        let bar_w = (rect.width() - 40.0) * item.plays as f32 / most as f32;
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min + vec2(36.0, 3.0), vec2(bar_w, rect.height() - 6.0)),
            CornerRadius::same(6),
            theme::accent_soft(),
        );
        ui.painter().text(
            pos2(rect.left() + 16.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            (i + 1).to_string(),
            theme::bold(14.0),
            if i < 3 { theme::accent() } else { muted() },
        );
        let text = Rect::from_min_max(
            pos2(rect.left() + 44.0, rect.top() + 3.0),
            pos2(rect.right() - 70.0, rect.bottom() - 3.0),
        );
        widgets::place(
            ui,
            egui::UiBuilder::new()
                .max_rect(text)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(egui::Label::new(RichText::new(&item.name).color(theme::text())).truncate());
                if !item.detail.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(&item.detail).small().color(subdued()))
                            .truncate(),
                    );
                }
            },
        );
        let times = if item.plays == 1 {
            "1 play".to_string()
        } else {
            format!("{} plays", item.plays)
        };
        ui.painter().text(
            pos2(rect.right() - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            times,
            theme::regular(12.0),
            subdued(),
        );
    }
}

/// Simple bars with labels underneath; the values are listening times.
fn bar_chart(ui: &mut Ui, bars: &[(String, u64)], height: f32) {
    let width = ui.available_width().min(760.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height + 18.0), Sense::hover());
    let most = bars.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1);
    let slot = rect.width() / bars.len() as f32;
    let hover = ui.ctx().pointer_hover_pos();
    for (i, (label, value)) in bars.iter().enumerate() {
        let x = rect.left() + slot * i as f32;
        let h = (height - 4.0) * *value as f32 / most as f32;
        let bar = Rect::from_min_max(
            pos2(x + slot * 0.15, rect.top() + height - h.max(2.0)),
            pos2(x + slot * 0.85, rect.top() + height),
        );
        let column = Rect::from_min_max(pos2(x, rect.top()), pos2(x + slot, rect.top() + height));
        let hovered = hover.is_some_and(|p| column.contains(p));
        let color = if hovered {
            theme::accent()
        } else {
            theme::accent().gamma_multiply(0.7)
        };
        ui.painter().rect_filled(bar, CornerRadius::same(3), color);
        ui.painter().text(
            pos2(x + slot / 2.0, rect.top() + height + 9.0),
            egui::Align2::CENTER_CENTER,
            label,
            theme::regular(11.0),
            muted(),
        );
        if hovered && *value > 0 {
            ui.painter().text(
                pos2(x + slot / 2.0, bar.top() - 8.0),
                egui::Align2::CENTER_CENTER,
                crate::stats::format_duration(*value),
                theme::bold(11.0),
                theme::text(),
            );
        }
    }
}

fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(text)
            .font(theme::bold(18.0))
            .color(theme::text()),
    );
    ui.add_space(4.0);
}

fn empty_hint(ui: &mut Ui, empty: bool, text: &str) {
    if empty {
        ui.label(RichText::new(text).color(subdued()));
    }
}

/// An album, playlist or podcast in the search results.
fn result_row(ui: &mut Ui, icon: Icon, title: &str, subtitle: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), hover());
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
            ui.add(egui::Label::new(RichText::new(title).color(theme::text())).truncate());
            ui.add(egui::Label::new(RichText::new(subtitle).small().color(subdued())).truncate());
        },
    );
    response
}

fn settings_card(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(surface())
        .stroke(Stroke::new(1.0, border()))
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
            ui.label(
                RichText::new(title)
                    .font(theme::bold(16.0))
                    .color(theme::text()),
            );
            ui.add_space(6.0);
            add_contents(ui);
        });
}
