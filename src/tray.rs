//! The app's icon, and on Windows the icon in the notification area (system
//! tray) with a small menu.

/// What the tray icon's menu asks for.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum TrayCmd {
    Show,
    PlayPause,
    Next,
    Previous,
    MiniPlayer,
    Quit,
}

/// The logo's normal colour.
pub const AMBER: [u8; 3] = [0xff, 0xb0, 0x2e];

/// The logo (a rounded square in `color` with equaliser bars) as RGBA pixels.
pub fn icon_rgba(size: u32, color: [u8; 3]) -> Vec<u8> {
    let accent = color;
    const BARS: [u8; 3] = [0x14, 0x16, 0x1d];
    let s = size as f32;
    let radius = s * 0.22;
    let heights = [0.35, 0.6, 0.45, 0.7];
    let bar_w = s * 0.11;
    let gap = s * 0.06;
    let total = 4.0 * bar_w + 3.0 * gap;
    let bottom = s - s * 0.2;
    // Inside the rounded square?
    let in_square = |x: f32, y: f32| {
        let cx = x.clamp(radius, s - radius);
        let cy = y.clamp(radius, s - radius);
        (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
    };
    let in_bar = |x: f32, y: f32| {
        heights.iter().enumerate().any(|(i, h)| {
            let left = (s - total) / 2.0 + i as f32 * (bar_w + gap);
            let top = bottom - s * h * 0.75;
            x >= left && x <= left + bar_w && y >= top && y <= bottom
        })
    };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            // 4 × 4 samples per pixel for smooth edges.
            let (mut square, mut bar) = (0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = px as f32 + (sx as f32 + 0.5) / 4.0;
                    let y = py as f32 + (sy as f32 + 0.5) / 4.0;
                    if in_square(x, y) {
                        square += 1;
                        if in_bar(x, y) {
                            bar += 1;
                        }
                    }
                }
            }
            let mix = |a: u8, b: u8| {
                (a as u32 * (square - bar) + b as u32 * bar)
                    .checked_div(square)
                    .unwrap_or(0) as u8
            };
            out.extend([
                mix(accent[0], BARS[0]),
                mix(accent[1], BARS[1]),
                mix(accent[2], BARS[2]),
                (square * 255 / 16) as u8,
            ]);
        }
    }
    out
}

pub use imp::Tray;

#[cfg(windows)]
mod imp {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::{AMBER, TrayCmd, icon_rgba};

    pub struct Tray {
        _icon: TrayIcon,
    }

    impl Tray {
        /// Adds the icon to the notification area. `on_cmd` is called from
        /// Windows' message loop when a menu item is clicked.
        pub fn new(on_cmd: impl Fn(TrayCmd) + Send + Sync + Clone + 'static) -> Option<Self> {
            match Self::try_new(on_cmd) {
                Ok(t) => Some(t),
                Err(e) => {
                    log::warn!("No tray icon: {e:#}");
                    None
                }
            }
        }

        fn try_new(
            on_cmd: impl Fn(TrayCmd) + Send + Sync + Clone + 'static,
        ) -> anyhow::Result<Self> {
            let items = [
                (
                    MenuItem::new("Play / pause", true, None),
                    TrayCmd::PlayPause,
                ),
                (MenuItem::new("Next", true, None), TrayCmd::Next),
                (MenuItem::new("Previous", true, None), TrayCmd::Previous),
                (
                    MenuItem::new("Show lite-spotify", true, None),
                    TrayCmd::Show,
                ),
                (
                    MenuItem::new("Mini player", true, None),
                    TrayCmd::MiniPlayer,
                ),
                (MenuItem::new("Quit", true, None), TrayCmd::Quit),
            ];
            let menu = Menu::new();
            for (i, (item, _)) in items.iter().enumerate() {
                if i == 3 || i == 5 {
                    menu.append(&PredefinedMenuItem::separator())?;
                }
                menu.append(item)?;
            }
            let ids: Vec<_> = items
                .iter()
                .map(|(item, cmd)| (item.id().clone(), *cmd))
                .collect();
            let on_menu = on_cmd.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some((_, cmd)) = ids.iter().find(|(id, _)| *id == event.id) {
                    on_menu(*cmd);
                }
            }));
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    on_cmd(TrayCmd::Show);
                }
            }));
            let icon = tray_icon::Icon::from_rgba(icon_rgba(32, AMBER), 32, 32)?;
            let tray = TrayIconBuilder::new()
                .with_tooltip("lite-spotify")
                .with_icon(icon)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()?;
            Ok(Self { _icon: tray })
        }

        /// Recolours the icon (it follows the accent colour).
        pub fn set_color(&self, color: [u8; 3]) {
            if let Ok(icon) = tray_icon::Icon::from_rgba(icon_rgba(32, color), 32, 32) {
                let _ = self._icon.set_icon(Some(icon));
            }
        }

        /// Shows the song in the icon's tooltip.
        pub fn set_tooltip(&self, text: &str) {
            // Windows cuts tooltips off at 127 characters.
            let text: String = text.chars().take(120).collect();
            let _ = self._icon.set_tooltip(Some(text));
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::TrayCmd;

    pub struct Tray;

    impl Tray {
        pub fn new(_on_cmd: impl Fn(TrayCmd) + Send + Sync + Clone + 'static) -> Option<Self> {
            None
        }
        pub fn set_tooltip(&self, _text: &str) {}
        pub fn set_color(&self, _color: [u8; 3]) {}
    }
}
