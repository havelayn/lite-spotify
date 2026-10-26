//! Small album covers for song lists, when turned on in Settings. They are
//! downloaded only for rows on screen, kept at 64 × 64 pixels, and the oldest
//! are dropped after a few hundred, so they cost a few megabytes at most.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use eframe::egui::{self, TextureHandle};

/// At most this many thumbnails are kept (about 16 KB each).
const MAX_KEPT: usize = 400;
/// At most this many downloads at once.
const MAX_LOADING: usize = 6;

#[derive(Default)]
pub struct Thumbs {
    inner: RefCell<Inner>,
}

#[derive(Default)]
struct Inner {
    enabled: bool,
    /// `None` while loading, or when the download failed.
    map: HashMap<String, Option<TextureHandle>>,
    /// Oldest first, for dropping old ones.
    order: VecDeque<String>,
    /// Wanted but not requested yet, newest last.
    waiting: Vec<String>,
    loading: usize,
}

impl Thumbs {
    pub fn set_enabled(&self, on: bool) {
        let mut inner = self.inner.borrow_mut();
        if !on {
            *inner = Inner::default();
        }
        inner.enabled = on;
    }

    /// The thumbnail for `url`, or `None` while it loads (or when thumbnails
    /// are off). Asking for one that is not there yet starts loading it.
    pub fn get(&self, url: Option<&str>) -> Option<TextureHandle> {
        let url = url?;
        let mut inner = self.inner.borrow_mut();
        if !inner.enabled {
            return None;
        }
        match inner.map.get(url) {
            Some(thumb) => thumb.clone(),
            None => {
                inner.map.insert(url.to_string(), None);
                inner.waiting.push(url.to_string());
                None
            }
        }
    }

    /// The next addresses to download. Rows drawn last (still on screen) go first.
    pub fn take_requests(&self) -> Vec<String> {
        let mut inner = self.inner.borrow_mut();
        let free = MAX_LOADING.saturating_sub(inner.loading);
        let take = free.min(inner.waiting.len());
        let start = inner.waiting.len() - take;
        let urls: Vec<String> = inner.waiting.drain(start..).collect();
        inner.loading += urls.len();
        // Rows that scrolled away long ago: forget them, they load again when needed.
        if inner.waiting.len() > 60 {
            let stale = std::mem::take(&mut inner.waiting);
            for url in stale {
                inner.map.remove(&url);
            }
        }
        urls
    }

    pub fn loaded(&self, ctx: &egui::Context, url: String, image: Option<egui::ColorImage>) {
        let mut inner = self.inner.borrow_mut();
        inner.loading = inner.loading.saturating_sub(1);
        if !inner.enabled {
            return;
        }
        let texture = image.map(|i| ctx.load_texture("thumb", i, egui::TextureOptions::LINEAR));
        inner.map.insert(url.clone(), texture);
        inner.order.push_back(url);
        while inner.order.len() > MAX_KEPT {
            if let Some(old) = inner.order.pop_front() {
                inner.map.remove(&old);
            }
        }
    }
}
