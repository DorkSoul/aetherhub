//! Aetherhub launcher for the standalone (no Docker) releases.
//!
//! Expects this layout next to the executable:
//!   node/            the Node.js runtime (node/bin/node on Linux, node/node.exe on Windows)
//!   app/server/      the bundled server (index.js)
//!   app/client/      the built web UI
//!   app/native/      the rules engine addon (libmtg_node.so / mtg_node.dll)
//!   app/cards/       the card definitions
//!   app/node_modules the server's runtime dependencies
//!
//! It starts the server on all network interfaces (so other devices on the home network can join), waits until it
//! answers, prints the addresses and opens the browser. Closing the window or pressing Ctrl+C stops the server.
//!
//! Environment overrides: PORT (default 8080), DATA_DIR (default: a per-user folder), HOST (default 0.0.0.0),
//! AETHERHUB_NO_BROWSER=1 (don't open the browser). `--no-browser` on the command line does the same.

use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode};
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Aetherhub could not start: {e}");
            pause_on_windows();
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let root = install_root()?;
    let app = root.join("app");
    let node = node_binary(&root);
    if !node.exists() {
        return Err(format!("the bundled Node.js runtime is missing ({})", node.display()));
    }
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let data_dir = data_dir()?;
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("can't create the data folder {}: {e}", data_dir.display()))?;

    // `aetherhub reset-password <username>`: prints a temporary password for that user and exits.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("reset-password") {
        let status = Command::new(&node)
            .arg(app.join("server").join("cli.js"))
            .args(&args)
            .current_dir(&app)
            .env("DATA_DIR", &data_dir)
            .status()
            .map_err(|e| format!("can't start {}: {e}", node.display()))?;
        return Ok(if status.success() { ExitCode::SUCCESS } else { ExitCode::FAILURE });
    }

    if TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(300)).is_ok() {
        return Err(format!("port {port} is already in use (is Aetherhub already running?). Set PORT to use another one."));
    }

    println!("Aetherhub: data in {}", data_dir.display());
    let mut child: Child = Command::new(&node)
        .arg(app.join("server").join("index.js"))
        .current_dir(&app)
        .env("NODE_ENV", "production")
        .env("PORT", port.to_string())
        .env("HOST", std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()))
        .env("DATA_DIR", &data_dir)
        .env("CLIENT_DIR", app.join("client"))
        .env("RULES_CARDS", app.join("cards"))
        .env("RULES_NATIVE", app.join("native").join(native_name()))
        .spawn()
        .map_err(|e| format!("can't start {}: {e}", node.display()))?;

    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            eprintln!("The server stopped ({status}).");
            pause_on_windows();
            return Ok(ExitCode::FAILURE);
        }
        if healthy(port) {
            break;
        }
        if started.elapsed() > Duration::from_secs(180) {
            eprintln!("The server did not answer within 3 minutes; it keeps running, open http://localhost:{port}/ yourself.");
            break;
        }
        std::thread::sleep(Duration::from_millis(400));
    }

    let local = format!("http://localhost:{port}/");
    println!();
    println!("  Aetherhub is running.");
    println!("  On this computer:      {local}");
    if let Some(ip) = lan_ip() {
        println!("  On your home network:  http://{ip}:{port}/");
    }
    println!("  Close this window or press Ctrl+C to stop it.");
    println!();

    let no_browser = std::env::var("AETHERHUB_NO_BROWSER").map_or(false, |v| v == "1") || std::env::args().any(|a| a == "--no-browser");
    if !no_browser {
        open_browser(&local);
    }

    let status = child.wait().map_err(|e| e.to_string())?;
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

fn open_browser(url: &str) {
    let r = if cfg!(windows) {
        Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
    if r.is_err() {
        println!("  (Couldn't open a browser; go to {url})");
    }
}

/// Keeps the console window open on Windows so an error can be read before it closes.
fn pause_on_windows() {
    if cfg!(windows) {
        eprintln!("Press Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}
