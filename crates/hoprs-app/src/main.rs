//! hoprs: desktop SSH + SFTP + FTP/FTPS client. Everything lives in the library.

// Release builds on Windows use the GUI subsystem: no console window behind the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use hoprs_app::cli::{self, Cli};

fn main() -> iced::Result {
    // Handled before `run()`: no instance lock, no logging, no window.
    match cli::parse_args(std::env::args().skip(1)) {
        Cli::Run => hoprs_app::run(),
        // The relay an MCP client starts: its stdin and stdout are the protocol, so the
        // standard handles the client gave are used as they are (no console attach, on
        // Windows nothing reopened), and nothing here prints.
        Cli::Mcp => std::process::exit(hoprs_app::run_mcp()),
        Cli::Version => {
            attach_parent_console();
            println!("{}", cli::version_line());
            Ok(())
        }
        Cli::Help => {
            attach_parent_console();
            println!("{}", cli::usage());
            Ok(())
        }
        Cli::Unknown(arg) => {
            attach_parent_console();
            eprintln!("hoprs: unknown argument '{arg}'\n{}", cli::usage());
            std::process::exit(2);
        }
    }
}

/// A GUI-subsystem process starts without a console, so `--version` would print nowhere.
/// Attach to the parent's console (the terminal that launched us) before printing.
/// Failure is ignored: when double-clicked there is no parent console and nothing to print to.
///
/// Started from an interactive cmd/PowerShell, the standard handles are null and attaching
/// alone does not fill them, so any null/invalid output handle is pointed at `CONOUT$`.
/// Redirected or piped handles are left untouched. Never called for `mcp`, whose
/// standard handles belong to the MCP client.
#[cfg(windows)]
fn attach_parent_console() {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };

    // SAFETY: plain Win32 calls; the only pointer is a NUL-terminated UTF-16 literal that
    // outlives the call. Every failure is harmless (nothing to print to).
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        let conout: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        for std_handle in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let current = GetStdHandle(std_handle);
            if !current.is_null() && current != INVALID_HANDLE_VALUE {
                continue;
            }
            let console = CreateFileW(
                conout.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if console != INVALID_HANDLE_VALUE {
                let _ = SetStdHandle(std_handle, console);
            }
        }
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}
