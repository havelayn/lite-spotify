//! Choosing files and folders with Windows' own dialog. On other systems
//! there is no dialog; files can be dragged onto the window instead.

use std::path::PathBuf;

/// Shows a "choose a folder" dialog and waits for the answer. Call it from a
/// thread of its own; the dialog keeps it busy until the user is done.
#[cfg(windows)]
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_folder()
}

#[cfg(not(windows))]
pub fn pick_folder(_title: &str) -> Option<PathBuf> {
    None
}

/// Whether [`pick_folder`] shows a dialog on this system.
pub const CAN_PICK: bool = cfg!(windows);
