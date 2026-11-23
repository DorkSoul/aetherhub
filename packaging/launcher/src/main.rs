//! Aetherhub launcher for the standalone (no Docker) releases.
//!
//! Expects this layout next to the executable:
//!   node/            the Node.js runtime (node/bin/node on Linux, node/node.exe on Windows)
//!   app/server/      the bundled server (index.js, cli.js)
//!   app/client/      the built web UI
//!   app/native/      the rules engine addon (libmtg_node.so / mtg_node.dll)
//!   app/cards/       the card definitions
//!   app/node_modules the server's runtime dependencies
//!
//! It starts the server on all network interfaces (so other devices on the home network can join) and shows a small
//! window with the addresses. Closing the window stops the server. The server's stdin is a pipe held by this process:
//! when the launcher ends for any reason the pipe closes and the server shuts down cleanly (it saves running games).
//! Server output goes to `<data folder>/aetherhub.log`.
//!
//! Without a display (or with `--no-gui` / AETHERHUB_NO_GUI=1) it runs in the terminal instead: Ctrl+C stops it.
//! `aetherhub reset-password <username>` prints a temporary password for that user and exits.
//!
//! Environment overrides: PORT (default 8080), DATA_DIR (default: a per-user folder), HOST (default 0.0.0.0),
//! AETHERHUB_NO_BROWSER=1 (don't open the browser when the server is ready).

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::fs::File;
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("reset-password") {
        attach_parent_console();
        return match reset_password(&args) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    let setup = match Setup::new() {
        Ok(s) => s,
        Err(e) => {
            fail(&format!("Aetherhub could not start: {e}"));
            return ExitCode::FAILURE;
        }
    };
    if !gui_wanted(&args) {
        attach_parent_console();
        return run_console(setup);
    }
    run_gui(setup)
}

/// Paths and settings shared by every mode.
struct Setup {
    app: PathBuf,
    node: PathBuf,
    data_dir: PathBuf,
    port: u16,
    open_browser: bool,
}

impl Setup {
    fn new() -> Result<Setup, String> {
        let root = install_root()?;
        let node = node_binary(&root);
        if !node.exists() {
            return Err(format!("the bundled Node.js runtime is missing ({})", node.display()));
        }
        let data_dir = data_dir()?;
        std::fs::create_dir_all(&data_dir).map_err(|e| format!("can't create the data folder {}: {e}", data_dir.display()))?;
        let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
        let open_browser = !(std::env::var("AETHERHUB_NO_BROWSER").map_or(false, |v| v == "1") || std::env::args().any(|a| a == "--no-browser"));
        Ok(Setup { app: root.join("app"), node, data_dir, port, open_browser })
    }

    fn log_path(&self) -> PathBuf {
        self.data_dir.join("aetherhub.log")
    }

    fn local_url(&self) -> String {
        format!("http://localhost:{}/", self.port)
    }

    fn lan_url(&self) -> Option<String> {
        lan_ip().map(|ip| format!("http://{ip}:{}/", self.port))
    }

    /// Starts the server. `log` = write its output to the log file (otherwise it shares this process's terminal).
    fn start(&self, log: bool) -> Result<Server, String> {
        if TcpStream::connect_timeout(&([127, 0, 0, 1], self.port).into(), Duration::from_millis(300)).is_ok() {
            return Err(format!("port {} is already in use (is Aetherhub already running?). Set PORT to use another one.", self.port));
        }
        let mut cmd = Command::new(&self.node);
        cmd.arg(self.app.join("server").join("index.js"))
            .current_dir(&self.app)
            .env("NODE_ENV", "production")
            .env("PORT", self.port.to_string())
            .env("HOST", std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()))
            .env("DATA_DIR", &self.data_dir)
            .env("CLIENT_DIR", self.app.join("client"))
            .env("RULES_CARDS", self.app.join("cards"))
            .env("RULES_NATIVE", self.app.join("native").join(native_name()))
            .env("AETHERHUB_STOP_ON_STDIN_EOF", "1")
            .stdin(Stdio::piped());
        if log {
            let f = File::create(self.log_path()).map_err(|e| format!("can't write {}: {e}", self.log_path().display()))?;
            let f2 = f.try_clone().map_err(|e| e.to_string())?;
            cmd.stdout(Stdio::from(f)).stderr(Stdio::from(f2));
        }
        hide_child_console(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| format!("can't start {}: {e}", self.node.display()))?;
        let stdin = child.stdin.take();
        Ok(Server { child, stdin })
    }
}

/// The running server process. Dropping `stdin` asks it to shut down.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Server {
    /// Clean shutdown: close its stdin and wait for it to save and exit; force it after 15 s.
    fn stop(&mut self) {
        self.stdin.take();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(15) {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------- window

#[derive(Clone, PartialEq)]
enum Status {
    Starting,
    Running,
    Stopped(String),
    Failed(String),
}

struct Shared {
    status: Status,
    server: Option<Server>,
}

struct LauncherApp {
    setup: Arc<Setup>,
    shared: Arc<Mutex<Shared>>,
    lan: Option<String>,
}

fn run_gui(setup: Setup) -> ExitCode {
    let setup = Arc::new(setup);
    let shared = Arc::new(Mutex::new(Shared { status: Status::Starting, server: None }));
    match setup.start(true) {
        Ok(s) => shared.lock().unwrap().server = Some(s),
        Err(e) => shared.lock().unwrap().status = Status::Failed(e),
    }
    let lan = setup.lan_url();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Aetherhub")
            .with_inner_size([460.0, 230.0])
            .with_min_inner_size([380.0, 210.0])
            .with_icon(icon()),
        ..Default::default()
    };
    let (s2, sh2) = (setup.clone(), shared.clone());
    let result = eframe::run_native(
        "Aetherhub",
        options,
        Box::new(move |cc| {
            watch(s2.clone(), sh2.clone(), cc.egui_ctx.clone());
            Ok(Box::new(LauncherApp { setup: s2, shared: sh2, lan }))
        }),
    );
    let window_failed = result.is_err();
    if let Err(e) = &result {
        // No window could be opened (no graphics driver, remote session ...): keep the server reachable and let a
        // simpler window or the terminal stop it.
        let running = matches!(shared.lock().unwrap().status, Status::Starting | Status::Running);
        if running {
            fallback_wait(&setup, &format!("{e}"));
        }
    }
    if let Some(mut s) = shared.lock().unwrap().server.take() {
        s.stop();
    }
    if window_failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Polls the server: marks it running once it answers (and opens the browser), or stopped if the process exits.
fn watch(setup: Arc<Setup>, shared: Arc<Mutex<Shared>>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let started = Instant::now();
        let mut opened = false;
        loop {
            {
                let mut g = shared.lock().unwrap();
                if matches!(g.status, Status::Failed(_) | Status::Stopped(_)) {
                    break;
                }
                let exited = g.server.as_mut().and_then(|s| s.child.try_wait().ok().flatten());
                if let Some(code) = exited {
                    g.status = Status::Stopped(format!("The server stopped ({code})."));
                    g.server = None;
                    ctx.request_repaint();
                    break;
                }
            }
            if healthy(setup.port) {
                let mut g = shared.lock().unwrap();
                if g.status == Status::Starting {
                    g.status = Status::Running;
                    ctx.request_repaint();
                }
                if !opened && setup.open_browser {
                    opened = true;
                    open_url(&setup.local_url());
                }
            } else if started.elapsed() > Duration::from_secs(180) && shared.lock().unwrap().status == Status::Starting {
                shared.lock().unwrap().status = Status::Failed("The server did not answer within 3 minutes.".into());
                ctx.request_repaint();
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

impl eframe::App for LauncherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let status = self.shared.lock().unwrap().status.clone();
        let mut quit = false;
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("Aetherhub");
                ui.label(egui::RichText::new(format!("v{VERSION}")).weak());
            });
            ui.add_space(8.0);
            match &status {
                Status::Starting => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Starting the server…");
                    });
                }
                Status::Running => {
                    ui.horizontal(|ui| {
                        let green = egui::Color32::from_rgb(108, 194, 138);
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.5, green);
                        ui.label(egui::RichText::new("Running").color(green));
                    });
                    ui.add_space(6.0);
                    egui::Grid::new("urls").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("On this computer:");
                        ui.hyperlink(self.setup.local_url());
                        ui.end_row();
                        if let Some(lan) = &self.lan {
                            ui.label("On your home network:");
                            ui.hyperlink(lan);
                            ui.end_row();
                        }
                    });
                }
                Status::Stopped(m) | Status::Failed(m) => {
                    ui.label(egui::RichText::new(m).color(egui::Color32::from_rgb(224, 112, 96)));
                    ui.label(format!("Details are in {}", self.setup.log_path().display()));
                }
            }
            ui.add_space(10.0);
            ui.label(egui::RichText::new(format!("Data: {}", self.setup.data_dir.display())).small().weak());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if status == Status::Running && ui.button("Open in browser").clicked() {
                    open_url(&self.setup.local_url());
                }
                let label = if matches!(status, Status::Starting | Status::Running) { "Stop Aetherhub" } else { "Close" };
                if ui.button(label).clicked() {
                    quit = true;
                }
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Closing this window stops Aetherhub.").small().weak());
        });
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if status == Status::Starting {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../icon.png")).unwrap_or_default()
}

// ---------------------------------------------------------------- terminal mode

fn run_console(setup: Setup) -> ExitCode {
    println!("Aetherhub {VERSION}: data in {}", setup.data_dir.display());
    let mut server = match setup.start(false) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Aetherhub could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    let started = Instant::now();
    loop {
        if let Ok(Some(status)) = server.child.try_wait() {
            eprintln!("The server stopped ({status}).");
            return ExitCode::FAILURE;
        }
        if healthy(setup.port) {
            break;
        }
        if started.elapsed() > Duration::from_secs(180) {
            eprintln!("The server did not answer within 3 minutes; it keeps running, open {} yourself.", setup.local_url());
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    println!();
    println!("  Aetherhub is running.");
    println!("  On this computer:      {}", setup.local_url());
    if let Some(lan) = setup.lan_url() {
        println!("  On your home network:  {lan}");
    }
    println!("  Press Ctrl+C to stop it.");
    println!();
    if setup.open_browser {
        open_url(&setup.local_url());
    }
    let status = server.child.wait();
    if status.map_or(false, |s| s.success()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The window could not be opened: wait in a message box (Windows) or the terminal until the user stops it.
fn fallback_wait(setup: &Setup, why: &str) {
    let lan = setup.lan_url().map(|l| format!("\nOn your home network: {l}")).unwrap_or_default();
    let text = format!(
        "Aetherhub is running.\n\nOn this computer: {}{lan}\n\nClose this message to stop Aetherhub.\n\n(The normal window could not be opened: {why})",
        setup.local_url()
    );
    if cfg!(windows) {
        message_box(&text);
    } else {
        eprintln!("{text}\nPress Ctrl+C to stop it.");
        if setup.open_browser {
            open_url(&setup.local_url());
        }
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
}

// ---------------------------------------------------------------- helpers

fn gui_wanted(args: &[String]) -> bool {
    if args.iter().any(|a| a == "--no-gui") || std::env::var("AETHERHUB_NO_GUI").map_or(false, |v| v == "1") {
        return false;
    }
    if cfg!(windows) || cfg!(target_os = "macos") {
        return true;
    }
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn reset_password(args: &[String]) -> Result<ExitCode, String> {
    let root = install_root()?;
    let node = node_binary(&root);
    let data_dir = data_dir()?;
    let app = root.join("app");
    let mut cmd = Command::new(&node);
    cmd.arg(app.join("server").join("cli.js")).args(args).current_dir(&app).env("DATA_DIR", &data_dir);
    let status = cmd.status().map_err(|e| format!("can't start {}: {e}", node.display()))?;
    Ok(if status.success() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// The folder holding `node/` and `app/`: the executable's own folder. On Linux symlinks are resolved (an AppImage's
/// AppRun); on Windows the path is used as is, because canonical paths there carry a `\\?\` prefix Node doesn't accept.
fn install_root() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = if cfg!(windows) { exe } else { exe.canonicalize().unwrap_or(exe) };
    exe.parent().map(Path::to_path_buf).ok_or_else(|| "can't find the install folder".into())
}

fn node_binary(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join("node").join("node.exe")
    } else {
        root.join("node").join("bin").join("node")
    }
}

fn native_name() -> &'static str {
    if cfg!(windows) {
        "mtg_node.dll"
    } else {
        "libmtg_node.so"
    }
}

/// $DATA_DIR, else %LOCALAPPDATA%\Aetherhub on Windows and $XDG_DATA_HOME/aetherhub (~/.local/share/aetherhub) elsewhere.
fn data_dir() -> Result<PathBuf, String> {
    if let Some(d) = std::env::var_os("DATA_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(d));
    }
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("APPDATA")).ok_or("LOCALAPPDATA is not set")?;
        return Ok(PathBuf::from(base).join("Aetherhub"));
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(x).join("aetherhub"));
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local").join("share").join("aetherhub"))
}

/// True when GET /api/health answers 200.
fn healthy(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if s.write_all(format!("GET /api/health HTTP/1.0\r\nHost: localhost:{port}\r\n\r\n").as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 32];
    let n = s.read(&mut buf).unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..n]);
    head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200")
}

/// The address other devices on the network reach this computer at (the interface of the default route; no packets are sent).
fn lan_ip() -> Option<std::net::IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn open_url(url: &str) {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    hide_child_console(&mut cmd);
    let _ = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

fn fail(msg: &str) {
    if cfg!(windows) {
        message_box(msg);
    } else {
        eprintln!("{msg}");
    }
}

#[cfg(windows)]
fn hide_child_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_child_console(_cmd: &mut Command) {}

/// Windows builds have no console of their own: use the one of the terminal they were started from, if any.
#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

#[cfg(windows)]
fn message_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (t, c) = (wide(text), wide("Aetherhub"));
    unsafe {
        MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONINFORMATION);
    }
}

#[cfg(not(windows))]
fn message_box(text: &str) {
    eprintln!("{text}");
}
