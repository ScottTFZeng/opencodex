#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime, WebviewWindow, WindowEvent,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

const SIDECAR_NAME: &str = "bun-x86_64-pc-windows-msvc.exe";
const START_TIMEOUT: Duration = Duration::from_secs(10);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(8);
const LIVENESS_INTERVAL: Duration = Duration::from_secs(2);
const STARTUP_LOG_NAME: &str = "desktop-startup.log";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct RuntimeState {
    pid: u32,
    port: u16,
    hostname: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Healthz {
    service: String,
    pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LiveProxy {
    pid: u32,
    port: u16,
    hostname: String,
}

impl LiveProxy {
    fn dashboard_url(&self) -> String {
        format!("http://{}:{}/", display_host(&self.hostname), self.port)
    }
}

struct OwnedProxy {
    child: Child,
    pid: u32,
    control_file: PathBuf,
    control_token: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProxyStatus {
    Starting,
    Owned,
    Attached,
    Unavailable,
}

impl ProxyStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Starting => "Proxy: starting",
            Self::Owned => "Proxy: running",
            Self::Attached => "Proxy: attached",
            Self::Unavailable => "Proxy: unavailable",
        }
    }
}

struct DesktopProxy {
    owned: Option<OwnedProxy>,
    attached: Option<LiveProxy>,
    status: ProxyStatus,
}

impl Default for DesktopProxy {
    fn default() -> Self {
        Self {
            owned: None,
            attached: None,
            status: ProxyStatus::Starting,
        }
    }
}

struct RuntimePaths {
    runtime: PathBuf,
    bun: PathBuf,
    config: PathBuf,
}

struct TrayControls<R: Runtime> {
    status: MenuItem<R>,
    restart: MenuItem<R>,
    update_status: MenuItem<R>,
    update_action: MenuItem<R>,
    update_secondary: MenuItem<R>,
    exit_after_update: MenuItem<R>,
}

struct AppState<R: Runtime> {
    proxy: Mutex<DesktopProxy>,
    updater: Mutex<DesktopUpdater>,
    tray: TrayControls<R>,
    did_notify_hide: AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdatePhase {
    Idle,
    Checking,
    NoUpdate,
    CheckFailed,
    Available,
    Downloading,
    DownloadFailed,
    Ready,
    Installing,
    InstallFailed,
    StopFailed,
    Recovering,
    RecoveryFailed,
    Recovered,
}

impl UpdatePhase {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "Update: Check for updates",
            Self::Checking => "Update: Checking for updates...",
            Self::NoUpdate => "Update: No updates available",
            Self::CheckFailed => "Update: Couldn't check for updates",
            Self::Available => "Update: Available",
            Self::Downloading => "Update: Downloading…",
            Self::DownloadFailed => "Update: Download failed",
            Self::Ready => "Update: Ready to install",
            Self::Installing => "Update: Installing...",
            Self::InstallFailed => "Update: Installation failed",
            Self::StopFailed => "Update: Proxy couldn't stop",
            Self::Recovering => "Update: Restoring proxy...",
            Self::RecoveryFailed => "Update: Proxy restart failed",
            Self::Recovered => "Update: Install failed; proxy restored",
        }
    }

    fn action_label(self) -> &'static str {
        match self {
            Self::Idle | Self::NoUpdate => "Check for Updates",
            Self::CheckFailed => "Try again",
            Self::Available => "Download and install",
            Self::DownloadFailed => "Retry download",
            Self::InstallFailed => "Retry download",
            Self::Ready => "Install and restart",
            Self::StopFailed => "Retry proxy stop",
            Self::RecoveryFailed => "Retry proxy start",
            Self::Recovered => "Dismiss",
            Self::Checking | Self::Downloading | Self::Installing | Self::Recovering => {
                "Working..."
            }
        }
    }

    fn action_enabled(self) -> bool {
        matches!(
            self,
            Self::Idle
                | Self::NoUpdate
                | Self::CheckFailed
                | Self::Available
                | Self::DownloadFailed
                | Self::Ready
                | Self::InstallFailed
                | Self::StopFailed
                | Self::RecoveryFailed
                | Self::Recovered
        )
    }

    fn blocks_proxy_restart(self) -> bool {
        matches!(self, Self::Checking | Self::Downloading | Self::Installing)
    }

    fn is_active(self) -> bool {
        matches!(self, Self::Downloading | Self::Installing)
    }

    fn secondary_label(self) -> Option<&'static str> {
        match self {
            Self::Available | Self::Ready => Some("Not now"),
            Self::DownloadFailed | Self::InstallFailed | Self::Recovered => Some("Dismiss"),
            _ => None,
        }
    }
}

struct DesktopUpdater {
    phase: UpdatePhase,
    exit_after_update: bool,
    owned_proxy_stopped: bool,
    pending_update: Option<Update>,
    pending_download: Option<Vec<u8>>,
    downloaded_bytes: u64,
    download_percent: Option<u8>,
}

impl Default for DesktopUpdater {
    fn default() -> Self {
        Self {
            phase: UpdatePhase::Idle,
            exit_after_update: false,
            owned_proxy_stopped: false,
            pending_update: None,
            pending_download: None,
            downloaded_bytes: 0,
            download_percent: None,
        }
    }
}

#[derive(Clone, Serialize)]
struct StartupEvent {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

enum StartError {
    Slow,
    Failed(String),
}

fn opencodex_home() -> PathBuf {
    if let Some(raw) = env::var_os("OPENCODEX_HOME").filter(|value| !value.is_empty()) {
        let value = PathBuf::from(raw);
        if value == Path::new("~") {
            return user_home();
        }
        if let Some(stripped) = value.to_string_lossy().strip_prefix("~/") {
            return user_home().join(stripped);
        }
        return value;
    }
    user_home().join(".opencodex")
}

fn user_home() -> PathBuf {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn runtime_paths<R: Runtime>(app: &AppHandle<R>) -> Result<RuntimePaths, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    let bun = [
        resource_dir.join("binaries").join(SIDECAR_NAME),
        resource_dir.join(SIDECAR_NAME),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or_else(|| "The bundled Bun sidecar is missing.".to_string())?;
    Ok(RuntimePaths {
        runtime: resource_dir.join("runtime"),
        bun,
        config: opencodex_home(),
    })
}

fn canonical_loopback_host(hostname: Option<&str>) -> Option<String> {
    let Some(hostname) = hostname else {
        return Some("127.0.0.1".to_string());
    };
    let host = hostname.trim();
    if host.eq_ignore_ascii_case("localhost") || matches!(host, "0.0.0.0" | "::" | "[::]") {
        return Some("127.0.0.1".to_string());
    }
    let host = if host.starts_with('[') || host.ends_with(']') {
        host.strip_prefix('[')?.strip_suffix(']')?
    } else {
        host
    };
    host.parse::<IpAddr>()
        .ok()
        .filter(IpAddr::is_loopback)
        .map(|address| address.to_string())
}

fn display_host(hostname: &str) -> String {
    if hostname.contains(':') {
        format!("[{hostname}]")
    } else {
        hostname.to_string()
    }
}

fn is_verified_identity(state: &RuntimeState, health: &Healthz) -> bool {
    health.service == "opencodex" && health.pid == state.pid
}

fn verified_runtime_proxy(config_dir: &Path) -> Option<LiveProxy> {
    let state: RuntimeState =
        serde_json::from_slice(&fs::read(config_dir.join("runtime-port.json")).ok()?).ok()?;
    // Runtime state is an untrusted handoff. Never resolve or navigate to non-loopback hosts.
    let hostname = canonical_loopback_host(state.hostname.as_deref())?;
    let address = (hostname.as_str(), state.port)
        .to_socket_addrs()
        .ok()?
        .next()?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(750)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(750)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(750)))
        .ok()?;
    stream
        .write_all(
            format!(
                "GET /healthz HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                display_host(&hostname)
            )
            .as_bytes(),
        )
        .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let (head, body) = response.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return None;
    }
    let health: Healthz = serde_json::from_str(body).ok()?;
    is_verified_identity(&state, &health).then_some(LiveProxy {
        pid: state.pid,
        port: state.port,
        hostname,
    })
}

fn random_hex() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn control_values(config_dir: &Path) -> Result<(PathBuf, String), String> {
    let file_token = random_hex()?;
    let control_token = random_hex()?;
    Ok((
        config_dir.join(format!("desktop-control-{file_token}.request")),
        control_token,
    ))
}

fn startup_log_path(config_dir: &Path) -> PathBuf {
    config_dir.join(STARTUP_LOG_NAME)
}

fn append_startup_log(config_dir: &Path) -> Result<File, String> {
    fs::create_dir_all(config_dir).map_err(|error| error.to_string())?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(startup_log_path(config_dir))
        .map_err(|error| error.to_string())
}

fn write_startup_failure(config_dir: &Path, message: &str) {
    if let Ok(mut log) = append_startup_log(config_dir) {
        let _ = writeln!(log, "OpenCodex desktop startup failed: {message}");
        let _ = log.sync_all();
    }
}

#[cfg(unix)]
fn restrict_control_file(file: &File, _path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn restrict_control_file(_file: &File, path: &Path) -> Result<(), String> {
    let username = env::var("USERNAME").map_err(|_| {
        "Cannot determine current Windows user for control-file ACL hardening.".to_string()
    })?;
    let user = env::var("USERDOMAIN")
        .ok()
        .filter(|domain| !domain.is_empty())
        .map(|domain| format!("{domain}\\{username}"))
        .unwrap_or(username);
    let path = path
        .to_str()
        .ok_or_else(|| "Control-file path is not valid Unicode.".to_string())?;
    let inheritance = Command::new("icacls.exe")
        .args([path, "/inheritance:r"])
        .status()
        .map_err(|error| error.to_string())?;
    if !inheritance.success() {
        return Err("Could not restrict control-file permissions.".to_string());
    }
    let broad_sids = ["*S-1-1-0", "*S-1-5-11", "*S-1-5-32-545"];
    let removal = Command::new("icacls.exe")
        .args([
            path,
            "/remove:g",
            broad_sids[0],
            broad_sids[1],
            broad_sids[2],
        ])
        .status()
        .map_err(|error| error.to_string())?;
    if !removal.success() {
        for sid in broad_sids {
            let probe = Command::new("icacls.exe")
                .args([path, "/findsid", sid])
                .output()
                .map_err(|error| error.to_string())?;
            if !probe.status.success() || String::from_utf8_lossy(&probe.stdout).contains(path) {
                return Err("Could not restrict control-file permissions.".to_string());
            }
        }
    }
    let grant = format!("{user}:(F)");
    let grant_status = Command::new("icacls.exe")
        .args([path, "/grant:r", &grant])
        .status()
        .map_err(|error| error.to_string())?;
    if !grant_status.success() {
        return Err("Could not restrict control-file permissions.".to_string());
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn restrict_control_file(_file: &File, _path: &Path) -> Result<(), String> {
    Ok(())
}

fn create_control_file(path: &Path) -> Result<(), String> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| "Control-file path has no parent directory.".to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    if let Err(error) = restrict_control_file(&file, path) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

fn write_control_request(path: &Path, token: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(token.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

impl DesktopProxy {
    fn start_or_attach(&mut self, paths: &RuntimePaths) -> Result<LiveProxy, StartError> {
        if self.owned.is_some() {
            return self.wait_for_owned(paths);
        }
        if let Some(live) = verified_runtime_proxy(&paths.config) {
            self.attached = Some(live.clone());
            self.status = ProxyStatus::Attached;
            return Ok(live);
        }

        self.start_new_owned(paths)
    }

    fn start_new_owned(&mut self, paths: &RuntimePaths) -> Result<LiveProxy, StartError> {
        self.attached = None;
        self.status = ProxyStatus::Starting;
        let (control_file, control_token) =
            control_values(&paths.config).map_err(StartError::Failed)?;
        create_control_file(&control_file).map_err(StartError::Failed)?;
        let log = append_startup_log(&paths.config).map_err(StartError::Failed)?;
        let log_err = log
            .try_clone()
            .map_err(|error| StartError::Failed(error.to_string()))?;
        let child = Command::new(&paths.bun)
            .arg(paths.runtime.join("src/cli/index.ts"))
            .arg("start")
            .current_dir(&paths.runtime)
            .env("OCX_GUI_DIST", paths.runtime.join("gui/dist"))
            .env("OPENCODEX_BUN_PATH", &paths.bun)
            .env("OCX_DESKTOP_CONTROL_FILE", &control_file)
            .env("OCX_DESKTOP_CONTROL_TOKEN", &control_token)
            .env_remove("OCX_SERVICE")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .spawn()
            .map_err(|error| {
                StartError::Failed(format!("Could not start the bundled proxy: {error}"))
            })?;
        let pid = child.id();
        self.owned = Some(OwnedProxy {
            child,
            pid,
            control_file,
            control_token,
        });
        self.wait_for_owned(paths)
    }

    fn wait_for_owned(&mut self, paths: &RuntimePaths) -> Result<LiveProxy, StartError> {
        let expected_pid = self.owned.as_ref().expect("owned proxy checked").pid;
        let deadline = Instant::now() + START_TIMEOUT;
        while Instant::now() < deadline {
            if let Some(live) = verified_runtime_proxy(&paths.config) {
                if live.pid == expected_pid {
                    self.attached = None;
                    self.status = ProxyStatus::Owned;
                    return Ok(live);
                }
            }
            let exited = self
                .owned
                .as_mut()
                .and_then(|owned| owned.child.try_wait().ok())
                .flatten();
            if let Some(status) = exited {
                self.owned = None;
                self.status = ProxyStatus::Unavailable;
                return Err(StartError::Failed(format!(
                    "The bundled proxy exited before it became ready ({status})."
                )));
            }
            thread::sleep(Duration::from_millis(150));
        }
        self.status = ProxyStatus::Starting;
        Err(StartError::Slow)
    }

    fn request_owned_shutdown(&mut self) -> Result<(), String> {
        self.request_owned_shutdown_with_timeout(SHUTDOWN_TIMEOUT)
    }

    fn request_owned_shutdown_with_timeout(
        &mut self,
        shutdown_timeout: Duration,
    ) -> Result<(), String> {
        if self.owned.is_none() {
            return Ok(());
        }
        let exited = self
            .owned
            .as_mut()
            .expect("owned proxy checked")
            .child
            .try_wait()
            .map_err(|error| error.to_string())?;
        if exited.is_some() {
            self.owned = None;
            self.status = ProxyStatus::Unavailable;
            return Ok(());
        }
        // Attached proxies never enter this branch. The control request is private to the
        // child held by this shell, so a missing listener cannot make that child unowned.
        let (control_file, control_token) = {
            let owned = self.owned.as_ref().expect("owned proxy checked");
            (owned.control_file.clone(), owned.control_token.clone())
        };
        // Best effort: the listener may already be gone, but the owned child still needs
        // reaping through its handle after the normal graceful deadline.
        let _ = write_control_request(&control_file, &control_token);
        let deadline = Instant::now() + shutdown_timeout;
        while Instant::now() < deadline {
            let exited = self
                .owned
                .as_mut()
                .expect("owned proxy checked")
                .child
                .try_wait()
                .map_err(|error| error.to_string())?;
            if exited.is_some() {
                self.owned = None;
                self.status = ProxyStatus::Unavailable;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        let owned = self.owned.as_mut().expect("owned proxy checked");
        // This child was spawned by this shell and is held by its Child handle. Attached
        // proxies cannot reach this path.
        owned.child.kill().map_err(|error| error.to_string())?;
        let _ = owned.child.wait();
        let _ = fs::remove_file(&owned.control_file);
        self.owned = None;
        self.status = ProxyStatus::Unavailable;
        Ok(())
    }
}

fn update_tray<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState<R>>();
    let updater = state.updater.lock().expect("desktop updater lock");
    let proxy = state.proxy.lock().expect("desktop proxy lock");
    let _ = state.tray.status.set_text(proxy.status.label());
    let _ = state
        .tray
        .restart
        .set_enabled(proxy.owned.is_some() && !updater.phase.blocks_proxy_restart());
    let update_label = updater
        .download_percent
        .map(|percent| format!("{} {percent}%", updater.phase.label()))
        .unwrap_or_else(|| updater.phase.label().to_string());
    let _ = state.tray.update_status.set_text(update_label);
    let _ = state
        .tray
        .update_action
        .set_text(updater.phase.action_label());
    let _ = state
        .tray
        .update_action
        .set_enabled(updater.phase.action_enabled());
    if let Some(label) = updater.phase.secondary_label() {
        let _ = state.tray.update_secondary.set_text(label);
        let _ = state.tray.update_secondary.set_enabled(true);
    } else {
        let _ = state.tray.update_secondary.set_enabled(false);
    }
    let _ = state
        .tray
        .exit_after_update
        .set_enabled(updater.phase.is_active() && !updater.exit_after_update);
}

fn set_update_phase<R: Runtime>(app: &AppHandle<R>, phase: UpdatePhase) {
    let state = app.state::<AppState<R>>();
    let mut updater = state.updater.lock().expect("desktop updater lock");
    updater.phase = phase;
    if matches!(
        phase,
        UpdatePhase::Idle
            | UpdatePhase::NoUpdate
            | UpdatePhase::CheckFailed
            | UpdatePhase::Available
            | UpdatePhase::DownloadFailed
            | UpdatePhase::InstallFailed
            | UpdatePhase::StopFailed
            | UpdatePhase::Recovering
            | UpdatePhase::RecoveryFailed
            | UpdatePhase::Recovered
    ) {
        updater.exit_after_update = false;
    }
    if !matches!(phase, UpdatePhase::Downloading) {
        updater.download_percent = None;
    }
    drop(updater);
    update_tray(app);
}

fn select_exit_after_update<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState<R>>();
    let mut updater = state.updater.lock().expect("desktop updater lock");
    if updater.phase.is_active() {
        updater.exit_after_update = true;
    }
    drop(updater);
    update_tray(app);
}

fn stop_owned_proxy_for_update<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    let state = app.state::<AppState<R>>();
    let was_owned = {
        let mut proxy = state.proxy.lock().expect("desktop proxy lock");
        if proxy.owned.is_none() {
            return Ok(false);
        }
        proxy.request_owned_shutdown()?;
        true
    };
    if was_owned {
        state
            .updater
            .lock()
            .expect("desktop updater lock")
            .owned_proxy_stopped = true;
        update_tray(app);
    }
    Ok(was_owned)
}

fn restore_owned_proxy_after_update_failure<R: Runtime>(app: &AppHandle<R>) {
    if !app
        .state::<AppState<R>>()
        .updater
        .lock()
        .expect("desktop updater lock")
        .owned_proxy_stopped
    {
        return;
    }
    set_update_phase(app, UpdatePhase::Recovering);
    let result = runtime_paths(app).and_then(|paths| {
        let state = app.state::<AppState<R>>();
        let mut proxy = state.proxy.lock().expect("desktop proxy lock");
        let result = if proxy.owned.is_some() {
            proxy.wait_for_owned(&paths)
        } else {
            proxy.start_new_owned(&paths)
        };
        result.map_err(|error| match error {
            StartError::Slow => "proxy recovery is still starting".to_string(),
            StartError::Failed(message) => message,
        })
    });
    let state = app.state::<AppState<R>>();
    let mut updater = state.updater.lock().expect("desktop updater lock");
    if result.is_ok() {
        updater.owned_proxy_stopped = false;
        updater.phase = UpdatePhase::Recovered;
    } else {
        updater.phase = UpdatePhase::RecoveryFailed;
    }
    drop(updater);
    update_tray(app);
}

fn check_for_updates<R: Runtime>(app: AppHandle<R>) {
    set_update_phase(&app, UpdatePhase::Checking);
    tauri::async_runtime::spawn(async move {
        let phase = match app.updater() {
            Ok(updater) => match updater.check().await {
                Ok(Some(_)) => UpdatePhase::Available,
                Ok(None) => UpdatePhase::NoUpdate,
                Err(_) => UpdatePhase::CheckFailed,
            },
            Err(_) => UpdatePhase::CheckFailed,
        };
        set_update_phase(&app, phase);
    });
}

fn update_download_progress<R: Runtime>(
    app: &AppHandle<R>,
    chunk_length: usize,
    content_length: Option<u64>,
) {
    let state = app.state::<AppState<R>>();
    let mut updater = state.updater.lock().expect("desktop updater lock");
    updater.downloaded_bytes = updater.downloaded_bytes.saturating_add(chunk_length as u64);
    updater.download_percent = content_length
        .filter(|length| *length > 0)
        .map(|length| ((updater.downloaded_bytes.saturating_mul(100) / length).min(100)) as u8);
    drop(updater);
    update_tray(app);
}

fn download_update<R: Runtime>(app: AppHandle<R>) {
    set_update_phase(&app, UpdatePhase::Downloading);
    {
        let state = app.state::<AppState<R>>();
        let mut updater = state.updater.lock().expect("desktop updater lock");
        updater.downloaded_bytes = 0;
        updater.download_percent = None;
    }
    tauri::async_runtime::spawn(async move {
        let updater = match app.updater() {
            Ok(updater) => updater,
            Err(_) => {
                set_update_phase(&app, UpdatePhase::DownloadFailed);
                return;
            }
        };
        let update = match updater.check().await {
            Ok(Some(update)) => update,
            Ok(None) => {
                set_update_phase(&app, UpdatePhase::NoUpdate);
                return;
            }
            Err(_) => {
                set_update_phase(&app, UpdatePhase::DownloadFailed);
                return;
            }
        };
        let progress_app = app.clone();
        match update
            .download(
                move |chunk_length, content_length| {
                    update_download_progress(&progress_app, chunk_length, content_length)
                },
                || {},
            )
            .await
        {
            Ok(download) => {
                let state = app.state::<AppState<R>>();
                let mut pending = state.updater.lock().expect("desktop updater lock");
                pending.pending_update = Some(update);
                pending.pending_download = Some(download);
                pending.phase = UpdatePhase::Ready;
                pending.download_percent = None;
                drop(pending);
                update_tray(&app);
            }
            Err(_) => set_update_phase(&app, UpdatePhase::DownloadFailed),
        }
    });
}

fn install_ready_update<R: Runtime>(app: AppHandle<R>) {
    if stop_owned_proxy_for_update(&app).is_err() {
        set_update_phase(&app, UpdatePhase::StopFailed);
        return;
    }
    let (update, download) = {
        let state = app.state::<AppState<R>>();
        let mut updater = state.updater.lock().expect("desktop updater lock");
        let Some(update) = updater.pending_update.take() else {
            return;
        };
        let Some(download) = updater.pending_download.take() else {
            return;
        };
        updater.phase = UpdatePhase::Installing;
        (update, download)
    };
    update_tray(&app);
    tauri::async_runtime::spawn(async move {
        match update.install(download) {
            Ok(()) => {
                let exit_after_update = app
                    .state::<AppState<R>>()
                    .updater
                    .lock()
                    .expect("desktop updater lock")
                    .exit_after_update;
                if exit_after_update {
                    app.exit(0);
                } else {
                    app.restart();
                }
            }
            Err(_) => handle_update_install_failure(&app),
        }
    });
}

fn handle_update_install_failure<R: Runtime>(app: &AppHandle<R>) {
    if app
        .state::<AppState<R>>()
        .updater
        .lock()
        .expect("desktop updater lock")
        .owned_proxy_stopped
    {
        restore_owned_proxy_after_update_failure(app);
    } else {
        // Attached proxies are never stopped or claimed. Keep their lifecycle separate
        // while exposing a retryable/dismissible update failure in the tray.
        set_update_phase(app, UpdatePhase::InstallFailed);
    }
}

fn dismiss_update<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState<R>>();
    let mut updater = state.updater.lock().expect("desktop updater lock");
    updater.pending_update = None;
    updater.pending_download = None;
    updater.downloaded_bytes = 0;
    updater.download_percent = None;
    updater.phase = UpdatePhase::Idle;
    updater.exit_after_update = false;
    drop(updater);
    update_tray(app);
}

fn handle_update_action<R: Runtime>(app: AppHandle<R>) {
    let phase = app
        .state::<AppState<R>>()
        .updater
        .lock()
        .expect("desktop updater lock")
        .phase;
    match phase {
        UpdatePhase::Idle | UpdatePhase::NoUpdate | UpdatePhase::CheckFailed => {
            check_for_updates(app)
        }
        UpdatePhase::Available | UpdatePhase::DownloadFailed | UpdatePhase::InstallFailed => {
            download_update(app)
        }
        UpdatePhase::Ready | UpdatePhase::StopFailed => install_ready_update(app),
        UpdatePhase::RecoveryFailed => restore_owned_proxy_after_update_failure(&app),
        UpdatePhase::Recovered => dismiss_update(&app),
        UpdatePhase::Checking
        | UpdatePhase::Downloading
        | UpdatePhase::Installing
        | UpdatePhase::Recovering => {}
    }
}

fn handle_update_secondary_action<R: Runtime>(app: &AppHandle<R>) {
    match app
        .state::<AppState<R>>()
        .updater
        .lock()
        .expect("desktop updater lock")
        .phase
    {
        UpdatePhase::Available
        | UpdatePhase::Ready
        | UpdatePhase::DownloadFailed
        | UpdatePhase::InstallFailed
        | UpdatePhase::Recovered => dismiss_update(app),
        _ => {}
    }
}

fn start_proxy<R: Runtime>(app: &AppHandle<R>) {
    let config_dir = opencodex_home();
    let result = runtime_paths(app).and_then(|paths| {
        app.state::<AppState<R>>()
            .proxy
            .lock()
            .expect("desktop proxy lock")
            .start_or_attach(&paths)
            .map_err(|error| match error {
                StartError::Slow => "startup is still in progress".to_string(),
                StartError::Failed(message) => message,
            })
    });
    update_tray(app);
    let event = match result {
        Ok(live) => StartupEvent {
            state: "ready",
            url: Some(live.dashboard_url()),
        },
        Err(message) if message == "startup is still in progress" => StartupEvent {
            state: "slow",
            url: None,
        },
        Err(message) => {
            eprintln!("OpenCodex desktop startup failed: {message}");
            write_startup_failure(&config_dir, &message);
            StartupEvent {
                state: "failed",
                url: None,
            }
        }
    };
    let _ = app.emit("proxy-startup", event);
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn hide_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let state = app.state::<AppState<R>>();
    if state.did_notify_hide.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = app
        .notification()
        .builder()
        .title("OpenCodex")
        .body("OpenCodex is still running in the notification area.")
        .show();
}

fn exit_shell<R: Runtime>(app: &AppHandle<R>) {
    if let Err(error) = app
        .state::<AppState<R>>()
        .proxy
        .lock()
        .expect("desktop proxy lock")
        .request_owned_shutdown()
    {
        eprintln!("OpenCodex desktop exit: {error}");
    }
    app.exit(0);
}

#[cfg(feature = "desktop-test-hook")]
fn install_ci_exit_hook<R: Runtime>(app: AppHandle<R>) {
    let Some(request_path) = env::var_os("OCX_DESKTOP_TEST_EXIT_REQUEST") else {
        return;
    };
    thread::spawn(move || {
        let request_path = PathBuf::from(request_path);
        for _ in 0..300 {
            if request_path.is_file() {
                exit_shell(&app);
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
}

fn monitor_proxy<R: Runtime>(app: &AppHandle<R>) {
    let (owned_exited, attached) = {
        let state = app.state::<AppState<R>>();
        let mut proxy = state.proxy.lock().expect("desktop proxy lock");
        let owned_exited = proxy
            .owned
            .as_mut()
            .and_then(|owned| owned.child.try_wait().ok())
            .is_some();
        if owned_exited {
            if let Some(owned) = proxy.owned.take() {
                let _ = fs::remove_file(owned.control_file);
            }
            proxy.status = ProxyStatus::Unavailable;
        }
        (owned_exited, proxy.attached.clone())
    };
    if owned_exited {
        update_tray(app);
    }

    // The bounded health probe runs after releasing the state lock.
    let Some(attached) = attached else {
        return;
    };
    let alive = verified_runtime_proxy(&opencodex_home()).as_ref() == Some(&attached);
    if alive {
        return;
    }
    let changed = {
        let state = app.state::<AppState<R>>();
        let mut proxy = state.proxy.lock().expect("desktop proxy lock");
        if proxy.attached.as_ref() == Some(&attached) {
            proxy.attached = None;
            proxy.status = ProxyStatus::Unavailable;
            true
        } else {
            false
        }
    };
    if changed {
        update_tray(app);
    }
}

fn start_liveness_monitor<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || loop {
        thread::sleep(LIVENESS_INTERVAL);
        monitor_proxy(&app);
    });
}

#[tauri::command]
fn retry_proxy(app: AppHandle) {
    thread::spawn(move || start_proxy(&app));
}

#[tauri::command]
fn begin_startup(app: AppHandle) {
    thread::spawn(move || start_proxy(&app));
}

#[tauri::command]
fn open_logs() -> Result<(), String> {
    let directory = opencodex_home();
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let log = startup_log_path(&directory);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|error| error.to_string())?;
    Command::new("notepad.exe")
        .arg(log)
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn exit_app(app: AppHandle) {
    exit_shell(&app);
}

fn install_window_behavior(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if app
                .state::<AppState<tauri::Wry>>()
                .updater
                .lock()
                .expect("desktop updater lock")
                .phase
                .is_active()
            {
                show_update_in_progress_dialog(&app);
            } else {
                hide_main(&app);
            }
        }
    });
}

fn show_update_in_progress_dialog<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    app.dialog()
        .message("OpenCodex is downloading an update.")
        .title("Update in progress")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Keep Updating".into(),
            "Exit after Update".into(),
        ))
        .show(move |keep_updating| {
            if !keep_updating {
                select_exit_after_update(&app);
            }
            hide_main(&app);
        });
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main(app)
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            begin_startup,
            retry_proxy,
            open_logs,
            exit_app
        ])
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Open OpenCodex", true, None::<&str>)?;
            let status = MenuItem::with_id(app, "status", "Proxy: starting", false, None::<&str>)?;
            let restart = MenuItem::with_id(app, "restart", "Restart Proxy", false, None::<&str>)?;
            let update_status = MenuItem::with_id(
                app,
                "update-status",
                "Check for Updates",
                false,
                None::<&str>,
            )?;
            let update_action = MenuItem::with_id(
                app,
                "update-action",
                "Check for Updates",
                true,
                None::<&str>,
            )?;
            let update_secondary =
                MenuItem::with_id(app, "update-secondary", "Not now", false, None::<&str>)?;
            let exit_after_update = MenuItem::with_id(
                app,
                "exit-after-update",
                "Exit after Update",
                false,
                None::<&str>,
            )?;
            let separator = PredefinedMenuItem::separator(app)?;
            let exit = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &open,
                    &status,
                    &restart,
                    &update_status,
                    &update_action,
                    &update_secondary,
                    &exit_after_update,
                    &separator,
                    &exit,
                ],
            )?;
            let tray = TrayIconBuilder::with_id("opencodex")
                .menu(&menu)
                .tooltip("OpenCodex")
                .icon(
                    app.default_window_icon()
                        .expect("desktop tray icon configured")
                        .clone(),
                )
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_main(&tray.app_handle());
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "restart" => {
                        if app
                            .state::<AppState<tauri::Wry>>()
                            .updater
                            .lock()
                            .expect("desktop updater lock")
                            .phase
                            .blocks_proxy_restart()
                        {
                            return;
                        }
                        let app = app.clone();
                        thread::spawn(move || {
                            let result = {
                                let state = app.state::<AppState<tauri::Wry>>();
                                let mut proxy = state.proxy.lock().expect("desktop proxy lock");
                                if proxy.owned.is_some() {
                                    proxy.request_owned_shutdown().map(|()| true)
                                } else {
                                    Ok(false)
                                }
                            };
                            match result {
                                Ok(true) => start_proxy(&app),
                                Ok(false) => {}
                                Err(error) => eprintln!("OpenCodex proxy restart: {error}"),
                            }
                            update_tray(&app);
                        });
                    }
                    "update-action" => handle_update_action(app.clone()),
                    "update-secondary" => handle_update_secondary_action(app),
                    "exit-after-update" => select_exit_after_update(app),
                    "exit" => {
                        if app
                            .state::<AppState<tauri::Wry>>()
                            .updater
                            .lock()
                            .expect("desktop updater lock")
                            .phase
                            .is_active()
                        {
                            select_exit_after_update(app);
                        } else {
                            exit_shell(app);
                        }
                    }
                    _ => {}
                })
                .build(app)?;
            let _ = tray;
            app.manage(AppState {
                proxy: Mutex::new(DesktopProxy::default()),
                updater: Mutex::new(DesktopUpdater::default()),
                tray: TrayControls {
                    status,
                    restart,
                    update_status,
                    update_action,
                    update_secondary,
                    exit_after_update,
                },
                did_notify_hide: AtomicBool::new(false),
            });
            install_window_behavior(&app.get_webview_window("main").expect("main window exists"));
            start_liveness_monitor(app.handle().clone());
            #[cfg(feature = "desktop-test-hook")]
            install_ci_exit_hook(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running OpenCodex desktop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_only_local_runtime_hosts() {
        assert_eq!(canonical_loopback_host(None).as_deref(), Some("127.0.0.1"));
        assert_eq!(
            canonical_loopback_host(Some("localhost")).as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            canonical_loopback_host(Some("0.0.0.0")).as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            canonical_loopback_host(Some("::")).as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            canonical_loopback_host(Some("127.0.0.2")).as_deref(),
            Some("127.0.0.2")
        );
        assert_eq!(
            canonical_loopback_host(Some("[::1]")).as_deref(),
            Some("::1")
        );
        assert_eq!(canonical_loopback_host(Some("192.168.1.5")), None);
        assert_eq!(canonical_loopback_host(Some("example.com")), None);
        assert_eq!(canonical_loopback_host(Some("[::1")), None);
    }

    #[test]
    fn dashboard_url_uses_the_verified_runtime_endpoint() {
        let live = LiveProxy {
            pid: 42,
            port: 58195,
            hostname: "::1".to_string(),
        };
        assert_eq!(live.dashboard_url(), "http://[::1]:58195/");
    }

    #[test]
    fn control_values_are_independent_cryptographic_hex_strings() {
        let (path, token) = control_values(std::path::Path::new("C:/control")).unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();
        let file_token = filename
            .strip_prefix("desktop-control-")
            .unwrap()
            .strip_suffix(".request")
            .unwrap();
        assert!(token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(file_token.len() == 64 && file_token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(file_token, token);
    }

    #[cfg(unix)]
    #[test]
    fn control_file_is_created_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory =
            std::env::temp_dir().join(format!("opencodex-control-test-{}", random_hex().unwrap()));
        let path = directory.join("request");
        create_control_file(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn owned_shutdown_reaps_child_when_no_health_endpoint_exists() {
        let directory = std::env::temp_dir().join(format!(
            "opencodex-owned-shutdown-test-{}",
            random_hex().unwrap()
        ));
        let control_file = directory.join("request");
        create_control_file(&control_file).unwrap();
        let child = Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut proxy = DesktopProxy {
            owned: Some(OwnedProxy {
                child,
                pid,
                control_file,
                control_token: "test-token".to_string(),
            }),
            attached: None,
            status: ProxyStatus::Owned,
        };

        proxy
            .request_owned_shutdown_with_timeout(Duration::ZERO)
            .unwrap();

        assert!(proxy.owned.is_none());
        assert_eq!(proxy.status, ProxyStatus::Unavailable);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn runtime_state_requires_a_matching_health_identity() {
        let state: RuntimeState =
            serde_json::from_str(r#"{"pid":42,"port":10100,"hostname":"127.0.0.1"}"#).unwrap();
        let health: Healthz = serde_json::from_str(r#"{"service":"opencodex","pid":42}"#).unwrap();
        assert!(is_verified_identity(&state, &health));
        assert!(!is_verified_identity(
            &state,
            &Healthz {
                service: "another-service".to_string(),
                pid: 42
            }
        ));
        assert!(!is_verified_identity(
            &state,
            &Healthz {
                service: "opencodex".to_string(),
                pid: 7
            }
        ));
    }

    #[test]
    fn update_phases_keep_proxy_restart_disabled_only_while_active_work_runs() {
        assert_eq!(
            UpdatePhase::CheckFailed.label(),
            "Update: Couldn't check for updates"
        );
        assert_eq!(UpdatePhase::CheckFailed.action_label(), "Try again");
        assert_eq!(
            UpdatePhase::DownloadFailed.label(),
            "Update: Download failed"
        );
        assert_eq!(UpdatePhase::DownloadFailed.action_label(), "Retry download");
        assert_eq!(UpdatePhase::Downloading.label(), "Update: Downloading…");
        assert_eq!(
            UpdatePhase::Available.action_label(),
            "Download and install"
        );
        assert_eq!(UpdatePhase::Available.secondary_label(), Some("Not now"));
        assert_eq!(UpdatePhase::Ready.action_label(), "Install and restart");
        assert_eq!(UpdatePhase::Ready.secondary_label(), Some("Not now"));
        assert_eq!(
            UpdatePhase::RecoveryFailed.action_label(),
            "Retry proxy start"
        );
        assert_eq!(UpdatePhase::InstallFailed.action_label(), "Retry download");
        assert_eq!(
            UpdatePhase::InstallFailed.secondary_label(),
            Some("Dismiss")
        );
        assert!(UpdatePhase::Checking.blocks_proxy_restart());
        assert!(UpdatePhase::Downloading.blocks_proxy_restart());
        assert!(UpdatePhase::Installing.blocks_proxy_restart());
        assert!(!UpdatePhase::Available.blocks_proxy_restart());
        assert!(!UpdatePhase::Available.is_active());
        assert!(UpdatePhase::Downloading.is_active());
    }
}
