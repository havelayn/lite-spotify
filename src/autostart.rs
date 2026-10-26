//! "Start lite-spotify when Windows starts": an entry under
//! HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run, written
//! with Windows' own `reg` tool. Each version has its own .exe name, so the
//! entry is renewed at every start to point at the program that is running.

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "lite-spotify";

#[cfg(windows)]
pub fn set(enabled: bool) {
    use std::os::windows::process::CommandExt;
    // Don't flash a console window.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("reg");
    if enabled {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let data = format!("\"{}\"", exe.display());
        command.args([
            "add", RUN_KEY, "/v", VALUE, "/t", "REG_SZ", "/d", &data, "/f",
        ]);
    } else {
        command.args(["delete", RUN_KEY, "/v", VALUE, "/f"]);
    }
    match command.creation_flags(CREATE_NO_WINDOW).output() {
        Ok(out) if out.status.success() => {}
        // Deleting an entry that isn't there "fails"; that is fine.
        Ok(_) if !enabled => {}
        Ok(out) => log::warn!(
            "Could not change starting with Windows: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => log::warn!("Could not change starting with Windows: {e}"),
    }
}

#[cfg(not(windows))]
pub fn set(_enabled: bool) {
    let _ = (RUN_KEY, VALUE);
}

/// Whether this system can start the app at login.
pub const SUPPORTED: bool = cfg!(windows);
