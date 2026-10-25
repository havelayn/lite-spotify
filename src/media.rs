//! The media popup Windows shows over the volume indicator, and the media keys
//! on the keyboard (Windows' "System Media Transport Controls"). On other
//! systems this does nothing.

/// A media key or a button in Windows' media popup.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum MediaKey {
    Play,
    Pause,
    Next,
    Previous,
    Stop,
    /// The user dragged the position in the popup (milliseconds).
    SeekTo(u32),
}

/// What the popup shows about the song.
#[cfg_attr(not(windows), allow(dead_code))]
pub struct SongInfo<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    pub cover_url: Option<&'a str>,
    pub duration_ms: u32,
}

/// Playing, paused or stopped, for the popup's play/pause button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    Playing,
    Paused,
    Stopped,
}

pub use imp::Controls;

#[cfg(windows)]
mod imp {
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{Result, bail};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
    use windows::Media::{
        MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
        SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties,
    };
    use windows::Storage::Streams::RandomAccessStreamReference;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
    use windows::core::{HSTRING, Ref};

    use super::{MediaKey, SongInfo, State};

    pub struct Controls {
        controls: SystemMediaTransportControls,
        timeline: SystemMediaTransportControlsTimelineProperties,
    }

    impl Controls {
        /// Connects to Windows' media controls for `window`. `on_key` is called
        /// (on another thread) when a media key or popup button is pressed.
        pub fn new(
            window: &impl HasWindowHandle,
            on_key: impl Fn(MediaKey) + Send + Sync + 'static,
        ) -> Option<Self> {
            match Self::try_new(window, on_key) {
                Ok(c) => Some(c),
                Err(e) => {
                    log::warn!("Media keys are not available: {e:#}");
                    None
                }
            }
        }

        fn try_new(
            window: &impl HasWindowHandle,
            on_key: impl Fn(MediaKey) + Send + Sync + 'static,
        ) -> Result<Self> {
            let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
                bail!("not a Win32 window");
            };
            let hwnd = HWND(handle.hwnd.get() as *mut std::ffi::c_void);
            let interop = windows::core::factory::<
                SystemMediaTransportControls,
                ISystemMediaTransportControlsInterop,
            >()?;
            let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };
            controls.SetIsEnabled(true)?;
            controls.SetIsPlayEnabled(true)?;
            controls.SetIsPauseEnabled(true)?;
            controls.SetIsStopEnabled(true)?;
            controls.SetIsNextEnabled(true)?;
            controls.SetIsPreviousEnabled(true)?;
            controls
                .DisplayUpdater()?
                .SetType(MediaPlaybackType::Music)?;

            let on_key = Arc::new(on_key);
            let on_button = on_key.clone();
            controls.ButtonPressed(&TypedEventHandler::new(
                move |_, args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
                    let button = args.ok()?.Button()?;
                    let key = match button {
                        SystemMediaTransportControlsButton::Play => MediaKey::Play,
                        SystemMediaTransportControlsButton::Pause => MediaKey::Pause,
                        SystemMediaTransportControlsButton::Stop => MediaKey::Stop,
                        SystemMediaTransportControlsButton::Next => MediaKey::Next,
                        SystemMediaTransportControlsButton::Previous => MediaKey::Previous,
                        _ => return Ok(()),
                    };
                    on_button(key);
                    Ok(())
                },
            ))?;
            controls.PlaybackPositionChangeRequested(&TypedEventHandler::new(
                move |_, args: Ref<PlaybackPositionChangeRequestedEventArgs>| {
                    let position = Duration::from(args.ok()?.RequestedPlaybackPosition()?);
                    on_key(MediaKey::SeekTo(position.as_millis() as u32));
                    Ok(())
                },
            ))?;
            let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
            Ok(Self { controls, timeline })
        }

        pub fn set_song(&mut self, song: Option<SongInfo>) {
            if let Err(e) = self.try_set_song(song) {
                log::warn!("Could not update the media popup: {e}");
            }
        }

        fn try_set_song(&mut self, song: Option<SongInfo>) -> windows::core::Result<()> {
            let updater = self.controls.DisplayUpdater()?;
            let Some(song) = song else {
                updater.ClearAll()?;
                updater.SetType(MediaPlaybackType::Music)?;
                return updater.Update();
            };
            updater.SetType(MediaPlaybackType::Music)?;
            let music = updater.MusicProperties()?;
            music.SetTitle(&HSTRING::from(song.title))?;
            music.SetArtist(&HSTRING::from(song.artist))?;
            music.SetAlbumTitle(&HSTRING::from(song.album))?;
            match song.cover_url {
                Some(url) => {
                    let uri = Uri::CreateUri(&HSTRING::from(url))?;
                    updater.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&uri)?)?;
                }
                None => updater.SetThumbnail(None)?,
            }
            updater.Update()?;

            let end = TimeSpan::from(Duration::from_millis(song.duration_ms as u64));
            self.timeline.SetStartTime(TimeSpan::default())?;
            self.timeline.SetMinSeekTime(TimeSpan::default())?;
            self.timeline.SetEndTime(end)?;
            self.timeline.SetMaxSeekTime(end)?;
            self.controls.UpdateTimelineProperties(&self.timeline)
        }

        pub fn set_state(&mut self, state: State, position_ms: u32) {
            let status = match state {
                State::Playing => MediaPlaybackStatus::Playing,
                State::Paused => MediaPlaybackStatus::Paused,
                State::Stopped => MediaPlaybackStatus::Stopped,
            };
            let result = (|| {
                self.controls.SetPlaybackStatus(status)?;
                self.timeline
                    .SetPosition(Duration::from_millis(position_ms as u64).into())?;
                self.controls.UpdateTimelineProperties(&self.timeline)
            })();
            if let Err(e) = result {
                log::warn!("Could not update the media popup: {e}");
            }
        }
    }

    impl Drop for Controls {
        fn drop(&mut self) {
            let _ = self.controls.SetIsEnabled(false);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{MediaKey, SongInfo, State};

    pub struct Controls;

    impl Controls {
        pub fn new(
            _window: &impl raw_window_handle::HasWindowHandle,
            _on_key: impl Fn(MediaKey) + Send + Sync + 'static,
        ) -> Option<Self> {
            None
        }
        pub fn set_song(&mut self, _song: Option<SongInfo>) {}
        pub fn set_state(&mut self, _state: State, _position_ms: u32) {}
    }
}
