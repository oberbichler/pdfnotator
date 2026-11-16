//! `PDFnotator` — a PDF viewer for pair-designing Typst documents with AI coding
//! agents over the Model Context Protocol.
//!
//! Module overview:
//! - [`session`]: all open documents (tabs) and which agent works on which
//! - [`document`]: one document — file watching, typst compilation, feedback queue
//! - [`feedback`]: comments and the queue handing them to the agent
//! - [`mcp`]: MCP protocol (tools, prompts, rendering of feedback)
//! - [`http`]: loopback HTTP server (MCP endpoint + REST API)
//! - [`bridge`]: `--mcp` stdio transport that launches the viewer on demand
//! - [`commands`]: Tauri commands for the UI
//! - [`remote`]: optional LAN server that shows the UI on a tablet (pen annotation)

#![doc(html_no_source)]

mod bridge;
mod commands;
mod document;
mod events;
mod feedback;
mod http;
mod mcp;
mod paths;
mod remote;
mod session;
mod typst;

use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use session::Session;

/// Port of the local HTTP server (override with `PDFNOTATOR_PORT`).
pub const DEFAULT_PORT: u16 = 4747;

const HELP: &str = "\
PDFnotator — PDF viewer for pair-designing Typst documents with AI agents

USAGE:
    pdfnotator [FILE]      open the viewer (FILE: .typ or .pdf)
    pdfnotator --mcp       run as stdio MCP server (launches the viewer on demand)

ENVIRONMENT:
    PDFNOTATOR_PORT        HTTP port (default 4747)
    PDFNOTATOR_TYPST       path of the typst binary
    PDFNOTATOR_REMOTE      1 = share with tablets on start (prints the pairing URL)
    PDFNOTATOR_REMOTE_PORT port for tablets (default 4748)";

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Viewer { file: Option<String> },
    Mcp,
    Help,
    Version,
}

impl Cli {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut file = None;
        for arg in args {
            match arg.as_str() {
                "--mcp" => return Self::Mcp,
                "-h" | "--help" => return Self::Help,
                "-V" | "--version" => return Self::Version,
                // ignore unknown flags (e.g. macOS -psn_…)
                a if a.starts_with('-') => {}
                _ => file = file.or(Some(arg)),
            }
        }
        Self::Viewer { file }
    }
}

fn port() -> u16 {
    std::env::var("PDFNOTATOR_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// Application entry point.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let port = port();
    match Cli::parse(std::env::args().skip(1)) {
        Cli::Help => println!("{HELP}"),
        Cli::Version => println!("pdfnotator {}", env!("CARGO_PKG_VERSION")),
        Cli::Mcp => bridge::run(port),
        Cli::Viewer { file } => run_viewer(port, file),
    }
}

fn run_viewer(port: u16, file: Option<String>) {
    let listener = TcpListener::bind(("127.0.0.1", port));
    // Single instance: hand the file to the running viewer and quit.
    if listener.is_err()
        && let Some(file) = &file
        && hand_over(port, file)
    {
        return;
    }

    let session = Session::new(port);
    let exit_session = session.clone();
    let remote = Arc::new(remote::Remote::default());
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(session.clone())
        .manage(remote.clone())
        .invoke_handler(tauri::generate_handler![
            commands::open_document,
            commands::close_document,
            commands::read_pdf,
            commands::get_status,
            commands::submit_feedback,
            commands::set_view,
            commands::app_info,
            commands::remote_start,
            commands::remote_stop,
            commands::remote_info,
        ])
        .setup(move |app| {
            session.set_sink(Arc::new(app.handle().clone()));
            match listener {
                Ok(listener) => serve(&session, listener)?,
                Err(e) => session.set_server_error(format!("port {port} unavailable: {e}")),
            }
            exit_on_signal(app.handle().clone(), session.clone());
            // PDFNOTATOR_REMOTE=1: share with tablets right away and print the pairing URL
            if std::env::var("PDFNOTATOR_REMOTE").is_ok_and(|v| v == "1") {
                let (handle, session, remote) = (app.handle().clone(), session.clone(), remote.clone());
                tauri::async_runtime::spawn(async move {
                    let assets = commands::remote_assets(handle);
                    match remote.start(session, assets, commands::remote_port()).await {
                        Ok(info) => eprintln!("pdfnotator: tablet URL {}", info.url),
                        Err(e) => eprintln!("pdfnotator: tablet sharing failed: {e}"),
                    }
                });
            }
            if let Some(file) = file {
                let session = session.clone();
                std::thread::spawn(move || {
                    if let Err(e) = session.open(&paths::absolutize(&file), None) {
                        eprintln!("pdfnotator: {e}");
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application");

    app.run(move |_, event| match event {
        // "Open with…" / double click in Finder (macOS sends an Apple Event, not argv)
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        tauri::RunEvent::Opened { urls } => {
            if let Some(path) = urls.into_iter().find_map(|u| u.to_file_path().ok()) {
                let session = exit_session.clone();
                std::thread::spawn(move || {
                    if let Err(e) = session.open(&path.display().to_string(), None) {
                        eprintln!("pdfnotator: {e}");
                    }
                });
            }
        }
        tauri::RunEvent::Exit => exit_session.shutdown(),
        _ => {}
    });
}

fn serve(session: &Arc<Session>, listener: TcpListener) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let session = session.clone();
    tauri::async_runtime::spawn(async move {
        let result = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => axum::serve(listener, http::router(session.clone())).await,
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            session.set_server_error(e.to_string());
        }
    });
    Ok(())
}

/// Quit cleanly on SIGTERM/SIGINT/SIGHUP so `typst watch` children are stopped
/// (otherwise they would outlive the app when it is killed or the session ends).
#[cfg(unix)]
fn exit_on_signal(app: tauri::AppHandle, session: Arc<Session>) {
    use tokio::signal::unix::{SignalKind, signal};
    tauri::async_runtime::spawn(async move {
        let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
            signal(SignalKind::hangup()),
        ) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
            _ = hup.recv() => {}
        }
        session.shutdown();
        app.exit(0);
    });
}

#[cfg(not(unix))]
fn exit_on_signal(_app: tauri::AppHandle, _session: Arc<Session>) {}

/// Ask an already running instance to open `file`. Returns `true` on success.
fn hand_over(port: u16, file: &str) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(40)))
        .build()
        .into();
    agent
        .post(&format!("http://127.0.0.1:{port}/api/open"))
        .send_json(serde_json::json!({ "path": paths::absolutize(file) }))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse(args.iter().map(ToString::to_string))
    }

    #[test]
    fn parses_cli() {
        assert_eq!(parse(&[]), Cli::Viewer { file: None });
        assert_eq!(
            parse(&["a.typ"]),
            Cli::Viewer {
                file: Some("a.typ".into())
            }
        );
        assert_eq!(
            parse(&["-psn_0_123", "a.pdf", "b.pdf"]),
            Cli::Viewer {
                file: Some("a.pdf".into())
            }
        );
        assert_eq!(parse(&["--mcp"]), Cli::Mcp);
        assert_eq!(parse(&["x", "--help"]), Cli::Help);
        assert_eq!(parse(&["-V"]), Cli::Version);
    }
}
