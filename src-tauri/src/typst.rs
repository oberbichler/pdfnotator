//! Runs `typst watch` for a `.typ` source and reports compile results.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// Result of one compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileEvent {
    /// Compiled successfully (possibly with warnings).
    Ok,
    /// Compilation failed; contains the diagnostics as printed by typst.
    Error(String),
}

/// How long typst has to stay quiet before a collected error block is reported.
/// typst prints nothing after the diagnostics, so there is no end marker.
const QUIET_PERIOD: Duration = Duration::from_millis(250);

/// Incremental parser for the (stderr) output of `typst watch`.
///
/// ```text
/// [12:00:00] compiled successfully in 2.06 ms
/// [12:00:01] compiled with errors
///
/// error: unclosed delimiter
///    ┌─ doc.typ:2:4
/// ```
#[derive(Debug, Default)]
pub struct LogParser {
    error: Option<String>,
}

impl LogParser {
    /// Feed one output line. Returns an event when a compile result is known.
    pub fn feed(&mut self, line: &str) -> Option<CompileEvent> {
        let line = strip_ansi(line);
        let trimmed = line.trim();
        if trimmed.contains("compiled successfully") || trimmed.contains("compiled with warnings") {
            self.error = None;
            Some(CompileEvent::Ok)
        } else if trimmed.contains("compiled with errors") {
            // an unreported previous error is superseded by the new one
            self.error = Some(String::new());
            None
        } else if trimmed.starts_with("watching ")
            || trimmed.starts_with("writing to ")
            || trimmed.contains("compiling ...")
        {
            self.flush()
        } else {
            if let Some(buf) = self.error.as_mut() {
                buf.push_str(&line);
                buf.push('\n');
            }
            None
        }
    }

    /// Report a pending error block (call when the output went quiet).
    pub fn flush(&mut self) -> Option<CompileEvent> {
        self.error
            .take()
            .map(|e| CompileEvent::Error(e.trim().to_owned()))
    }
}

/// Remove ANSI escape sequences (colour codes) from a line.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Locate the typst binary. GUI apps started from the Finder have a minimal
/// `PATH`, so common install locations are probed as well.
pub fn find_typst() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PDFNOTATOR_TYPST") {
        return Some(PathBuf::from(p));
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        dirs.extend([home.join(".cargo/bin"), home.join(".local/bin")]);
    }
    let exe = if cfg!(windows) { "typst.exe" } else { "typst" };
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

/// A running `typst watch` process. Killed on drop.
#[derive(Debug)]
pub struct TypstWatch {
    child: Child,
}

impl Drop for TypstWatch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl TypstWatch {
    /// Start `typst watch <typ> <pdf>`; `on_event` is called after every compile.
    pub fn spawn(
        typ: &Path,
        pdf: &Path,
        root: Option<&Path>,
        on_event: impl Fn(CompileEvent) + Send + 'static,
    ) -> Result<Self, String> {
        let bin = find_typst().ok_or("typst not found — install it or set PDFNOTATOR_TYPST")?;
        let mut cmd = Command::new(bin);
        cmd.arg("watch");
        if let Some(root) = root {
            cmd.arg("--root").arg(root);
        }
        cmd.arg(typ)
            .arg(pdf)
            .current_dir(typ.parent().unwrap_or_else(|| Path::new("/")))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to start typst watch: {e}"))?;
        let stderr = child.stderr.take().ok_or("typst stderr unavailable")?;

        // Reader thread → channel; the processing thread can then flush an
        // error block after a quiet period.
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut parser = LogParser::default();
            loop {
                let event = match rx.recv_timeout(QUIET_PERIOD) {
                    Ok(line) => parser.feed(&line),
                    Err(RecvTimeoutError::Timeout) => parser.flush(),
                    Err(RecvTimeoutError::Disconnected) => {
                        if let Some(ev) = parser.flush() {
                            on_event(ev);
                        }
                        break;
                    }
                };
                if let Some(ev) = event {
                    on_event(ev);
                }
            }
        });
        Ok(Self { child })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[&str]) -> Vec<CompileEvent> {
        let mut p = LogParser::default();
        let mut out: Vec<_> = lines.iter().filter_map(|l| p.feed(l)).collect();
        out.extend(p.flush());
        out
    }

    #[test]
    fn success() {
        let ev = run(&[
            "watching a.typ",
            "writing to a.pdf",
            "",
            "[12:00:00] compiled successfully in 2 ms",
        ]);
        assert_eq!(ev, vec![CompileEvent::Ok]);
    }

    #[test]
    fn warnings_count_as_success() {
        assert_eq!(
            run(&["[12:00:00] compiled with warnings", "warning: unused"]),
            vec![CompileEvent::Ok]
        );
    }

    #[test]
    fn error_block_is_collected_until_quiet() {
        let ev = run(&[
            "[22:21:11] compiled with errors",
            "",
            "error: unclosed delimiter",
            "  ┌─ w.typ:2:4",
            "2 │ #foo(",
        ]);
        let [CompileEvent::Error(msg)] = ev.as_slice() else {
            panic!("{ev:?}")
        };
        assert!(msg.starts_with("error: unclosed delimiter"));
        assert!(msg.contains("w.typ:2:4"));
        assert!(msg.contains("#foo("));
    }

    #[test]
    fn error_is_flushed_by_next_cycle_then_success() {
        let ev = run(&[
            "[1] compiled with errors",
            "error: boom",
            "watching a.typ",
            "[2] compiling ...",
            "[2] compiled successfully in 1 ms",
        ]);
        assert_eq!(
            ev,
            vec![CompileEvent::Error("error: boom".into()), CompileEvent::Ok]
        );
    }

    #[test]
    fn strips_ansi_sequences() {
        assert_eq!(strip_ansi("\x1b[1;31merror\x1b[0m: x"), "error: x");
        let ev = run(&["\x1b[32m[1] compiled successfully\x1b[0m"]);
        assert_eq!(ev, vec![CompileEvent::Ok]);
    }

    /// End-to-end with the real typst binary (skipped when it is not installed).
    #[test]
    fn watches_real_typst() {
        if find_typst().is_none() {
            eprintln!("typst not installed — skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let typ = dir.path().join("doc.typ");
        let pdf = dir.path().join("doc.pdf");
        std::fs::write(&typ, "= Hello").unwrap();
        let (tx, rx) = mpsc::channel();
        let _watch = TypstWatch::spawn(&typ, &pdf, None, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        let first = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert_eq!(first, CompileEvent::Ok);
        assert!(pdf.exists());

        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(&typ, "= Hello\n#foo(").unwrap();
        let second = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert!(
            matches!(second, CompileEvent::Error(ref m) if m.contains("unclosed")),
            "{second:?}"
        );
    }
}
