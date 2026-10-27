//! The Now Playing screen: the cover big on its own blurred colours, with the
//! controls on a frosted card and the lyrics next to it when there is room.
//! Opens by clicking the cover in the player panel (or Ctrl+N); Esc closes it.

use eframe::egui::{
    self, Align, CornerRadius, Frame, Key, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2,
    pos2, vec2,
};

use super::App;
use crate::config::Repeat;
use crate::theme::{self, muted, subdued};
use crate::widgets::{self, Icon, format_time};

impl App {
    pub(super) fn now_view_screen(&mut self, ui: &mut Ui) {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.now_view = false;
            return;
        }
        let mut close = false;
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::bg())
                    .inner_margin(Margin::same(28)),
            )
            .show(ui, |ui| {
                let full = ui.max_rect();
                // Close button, top right.
                let close_rect =
                    Rect::from_min_size(pos2(full.right() - 34.0, full.top()), Vec2::splat(34.0));
                widgets::place_in(ui, close_rect, |ui| {
                    if widgets::icon_button(ui, Icon::Close, 14.0, true, false)
                        .on_hover_text("Close (Esc)")
                        .clicked()
                    {
                        close = true;
                    }
                });

                // Lyrics on the right when the window is wide enough.
                let with_lyrics = full.width() > 980.0;
                let left = if with_lyrics {
                    Rect::from_min_max(full.min, pos2(full.center().x - 10.0, full.bottom()))
                } else {
                    full
                };
                widgets::place(
                    ui,
                    egui::UiBuilder::new()
                        .max_rect(left)
                        .layout(Layout::top_down(Align::Center)),
                    |ui| self.now_view_player(ui),
                );
                if with_lyrics {
                    let right = Rect::from_min_max(
                        pos2(full.center().x + 10.0, full.top() + 40.0),
                        full.max,
                    );
                    widgets::place(
                        ui,
                        egui::UiBuilder::new()
                            .max_rect(right)
                            .layout(Layout::top_down(Align::Min)),
                        |ui| {
                            glass_card().show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                                ui.label(
                                    RichText::new("LYRICS")
                                        .font(theme::bold(11.0))
                                        .color(theme::accent()),
                                );
                                self.lyrics_lines(ui, false);
                            });
                        },
                    );
                }
            });
        self.toast(ui.ctx());
        if close {
            self.now_view = false;
        }
    }

    /// Cover, title, seek bar and buttons.
    fn now_view_player(&mut self, ui: &mut Ui) {
        let enabled = self.player_ready() && self.now.track.is_some();
        let avail = ui.available_size();
        let size = (avail.y - 250.0).min(avail.x - 40.0).clamp(160.0, 520.0);
        ui.add_space(((avail.y - size - 250.0) / 2.0).max(0.0));
        let (cover, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        // A soft shadow under the cover.
        ui.painter().rect_filled(
            cover.translate(vec2(0.0, 10.0)).expand(6.0),
            CornerRadius::same(22),
            egui::Color32::from_black_alpha(60),
        );
        let texture = self
            .cover
            .as_ref()
            .filter(|_| self.now.track.is_some())
            .map(|(_, t)| t);
        widgets::paint_cover(ui, cover, texture, Icon::Note, 16);
        ui.add_space(18.0);

        let width = size.max(320.0).min(avail.x);
        let (title, artist) = match &self.now.track {
            Some(t) => (t.name.clone(), t.artists.clone()),
            None => ("Nothing playing".to_string(), String::new()),
        };
        ui.scope(|ui| {
            ui.set_max_width(width);
            ui.add(
                egui::Label::new(
                    RichText::new(title)
                        .font(theme::bold(26.0))
                        .color(theme::text()),
                )
                .truncate(),
            );
            ui.add(egui::Label::new(RichText::new(artist).size(17.0).color(subdued())).truncate());
        });
        ui.add_space(12.0);

        // Seek bar and times.
        let duration = self.now.track.as_ref().map_or(0, |t| t.duration_ms);
        let shown = self
            .seek_drag
            .map(|v| (v * duration as f32) as u32)
            .unwrap_or_else(|| self.now.position_ms());
        let fraction = if duration > 0 {
            shown as f32 / duration as f32
        } else {
            0.0
        };
        let (response, value) = widgets::thin_slider(ui, fraction, width, enabled && duration > 0);
        if let Some(v) = value {
            self.seek_drag = Some(v);
        }
        if let Some(v) = self.seek_drag
            && !response.is_pointer_button_down_on()
            && !response.dragged()
        {
            self.seek_drag = None;
            self.seek((v * duration as f32) as u32);
        }
        let (times, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
        for (pos, align, ms) in [
            (times.left_center(), egui::Align2::LEFT_CENTER, shown),
            (times.right_center(), egui::Align2::RIGHT_CENTER, duration),
        ] {
            ui.painter()
                .text(pos, align, format_time(ms), theme::regular(12.0), muted());
        }
        ui.add_space(10.0);

        // Buttons.
        let mut action = None;
        ui.allocate_ui_with_layout(
            vec2(width, 72.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let total = 4.0 * 34.0 + 64.0 + 5.0 * 16.0 + 30.0;
                ui.add_space(((width - total) / 2.0).max(0.0));
                if widgets::heart_button(
                    ui,
                    20.0,
                    self.now
                        .track
                        .as_ref()
                        .is_some_and(|t| self.is_liked(&t.uri)),
                )
                .on_hover_text("Liked Songs (L)")
                .clicked()
                {
                    action = Some(5);
                }
                if widgets::icon_button(ui, Icon::Shuffle, 20.0, true, self.config.shuffle)
                    .clicked()
                {
                    action = Some(0);
                }
                if widgets::icon_button(ui, Icon::Previous, 22.0, enabled, false).clicked() {
                    action = Some(1);
                }
                if widgets::play_button(ui, 64.0, self.now.playing, enabled).clicked() {
                    action = Some(2);
                }
                if widgets::icon_button(ui, Icon::Next, 22.0, enabled, false).clicked() {
                    action = Some(3);
                }
                let icon = if self.config.repeat == Repeat::One {
                    Icon::RepeatOne
                } else {
                    Icon::Repeat
                };
                if widgets::icon_button(ui, icon, 20.0, true, self.config.repeat != Repeat::Off)
                    .clicked()
                {
                    action = Some(4);
                }
            },
        );
        match action {
            Some(0) => self.set_shuffle(!self.config.shuffle),
            Some(1) => self.previous(),
            Some(2) => self.toggle_play(),
            Some(3) => self.next(),
            Some(4) => self.cycle_repeat(),
            Some(5) => {
                if let Some(t) = self.now.track.clone() {
                    self.toggle_like(t);
                }
            }
            _ => {}
        }

        // Volume.
        ui.allocate_ui_with_layout(
            vec2(width.min(300.0), 24.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                widgets::paint_icon(
                    ui.painter(),
                    Rect::from_center_size(ui.cursor().min + vec2(9.0, 12.0), Vec2::splat(16.0)),
                    Icon::VolumeLow,
                    muted(),
                );
                ui.add_space(24.0);
                let w = ui.available_width();
                let (response, value) = widgets::thin_slider(ui, self.config.volume, w, true);
                if let Some(v) = value {
                    self.set_volume(v);
                }
                if let Some(v) = wheel_volume(ui, &response, self.config.volume) {
                    self.set_volume(v);
                }
                if response.drag_stopped() || response.clicked() {
                    self.config.save();
                }
            },
        );
        if self.now.playing {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

/// A frosted card (or a plain panel outside the glass look).
pub(super) fn glass_card() -> Frame {
    Frame::new()
        .fill(theme::surface())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(20)
        .inner_margin(Margin::same(20))
}

/// The mouse wheel over a volume bar changes the volume. Returns the new
/// volume when it changed.
pub(super) fn wheel_volume(ui: &Ui, response: &egui::Response, volume: f32) -> Option<f32> {
    if !response.hovered() {
        return None;
    }
    let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.smooth_scroll_delta.x);
    (scroll.abs() > 0.1).then(|| (volume + scroll / 600.0).clamp(0.0, 1.0))
}
