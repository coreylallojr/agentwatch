//! OS glue: paths, rules loading, toasts, opening the browser, process liveness and detached spawning.
use crate::parse::{compile_rules, Rule};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const DEFAULT_PORT: u16 = 4778;
pub const BUNDLED_RULES: &str = include_str!("../../rules.json");

pub fn user_home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// `AGENTWATCH_HOME` or ~/.agentwatch (pidfile and optional rules.json).
pub fn home_dir() -> PathBuf {
    std::env::var_os("AGENTWATCH_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| user_home().join(".agentwatch"))
}

/// Root that holds one directory per Claude Code project (encoded cwd).
pub fn default_projects_root() -> PathBuf {
    std::env::var_os("CLAUDE_PROJECTS_DIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| user_home().join(".claude").join("projects"))
}

fn compile_text(t: &str) -> Option<Vec<Rule>> {
    let v = crate::js::parse_line(t)?;
    compile_rules(&v).ok()
}

/// User rules (~/.agentwatch/rules.json) override the bundled defaults; a broken file falls through to the next.
pub fn load_rules(file: Option<&std::path::Path>) -> Vec<Rule> {
    if let Some(f) = file { return std::fs::read_to_string(f).ok().and_then(|t| compile_text(&t)).unwrap_or_default(); }
    if let Some(r) = std::fs::read_to_string(home_dir().join("rules.json")).ok().and_then(|t| compile_text(&t)) { return r; }
    compile_text(BUNDLED_RULES).unwrap_or_default()
}

#[cfg(windows)]
mod win {
    pub const DETACHED_PROCESS: u32 = 0x0000_0008;
    pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
}

fn detach(c: &mut Command) -> &mut Command {
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(win::DETACHED_PROCESS | win::CREATE_NEW_PROCESS_GROUP | win::CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    c
}

/// Windows balloon notification via PowerShell (same script as the Node version). No-op elsewhere.
pub fn show_toast(title: &str, text: &str) {
    if !cfg!(windows) { return; }
    let script = [
        "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing;",
        "$n = New-Object System.Windows.Forms.NotifyIcon; $n.Icon = [System.Drawing.SystemIcons]::Information; $n.Visible = $true;",
        "$n.ShowBalloonTip(8000, $env:AW_TITLE, $env:AW_TEXT, 'Info'); Start-Sleep -Seconds 9; $n.Dispose()",
    ].join(" ");
    let _ = detach(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .env("AW_TITLE", crate::js::head16(title, 60)).env("AW_TEXT", crate::js::head16(text, 200))).spawn();
}

pub fn open_browser(url: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = detach(Command::new("cmd.exe").raw_arg(format!("/d /s /c start \"\" \"{url}\""))).spawn();
    }
    #[cfg(not(windows))]
    { let _ = detach(&mut Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(url)).spawn(); }
}

/// Start `agentwatch serve ...` in the background.
pub fn spawn_server(args: &[String]) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    detach(Command::new(exe).arg("serve").args(args)).spawn().map(|_| ())
}

/// `process.kill(pid, 0)` succeeds (or fails with EPERM).
pub fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() { return GetLastError() == ERROR_ACCESS_DENIED; }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        ok && code == STILL_ACTIVE as u32
    }
    #[cfg(unix)]
    unsafe { libc::kill(pid as i32, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

/// `process.kill(pid)` (SIGTERM; TerminateProcess on Windows).
pub fn kill(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h.is_null() { return Err(format!("kill ESRCH")); }
        let ok = TerminateProcess(h, 1) != 0;
        CloseHandle(h);
        if ok { Ok(()) } else { Err("kill EPERM".into()) }
    }
    #[cfg(unix)]
    unsafe { if libc::kill(pid as i32, libc::SIGTERM) == 0 { Ok(()) } else { Err("kill failed".into()) } }
}

pub fn compile_text_pub(t: &str) -> Vec<Rule> { compile_text(t).unwrap_or_default() }

/// Per-thread Windows quality of service. `interactive = true` (request threads, someone is waiting) opts out of
/// execution-speed throttling so the thread can run on performance cores; `false` (background scanning, pings)
/// opts in to EcoQoS. No-op elsewhere.
pub fn set_thread_qos(interactive: bool) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadInformation, ThreadPowerThrottling, THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE};
        let st = THREAD_POWER_THROTTLING_STATE {
            Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: if interactive { 0 } else { THREAD_POWER_THROTTLING_EXECUTION_SPEED },
        };
        SetThreadInformation(GetCurrentThread(), ThreadPowerThrottling, &st as *const _ as *const core::ffi::c_void, std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32);
    }
    #[cfg(not(windows))]
    let _ = interactive;
}
