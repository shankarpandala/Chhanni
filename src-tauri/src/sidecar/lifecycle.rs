use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tracing::{info, warn};

use crate::error::{SidecarError, SidecarResult};

/// Spec describing how to launch the sidecar binary.
#[derive(Clone, Debug)]
pub struct SidecarSpec {
    pub binary_path: PathBuf,
    pub model_path: PathBuf,
    /// Extra args appended after the canonical ones.
    pub extra_args: Vec<String>,
    /// Max time we'll wait for the server to print a "listening on" line.
    pub startup_timeout: Duration,
}

impl SidecarSpec {
    pub fn new(binary_path: PathBuf, model_path: PathBuf) -> Self {
        Self {
            binary_path,
            model_path,
            extra_args: Vec::new(),
            startup_timeout: Duration::from_secs(60),
        }
    }
}

/// Live handle to a spawned sidecar. Dropping it kills the child.
pub struct SidecarHandle {
    child: Mutex<Option<Child>>,
    pub port: u16,
}

impl std::fmt::Debug for SidecarHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidecarHandle")
            .field("port", &self.port)
            .field("running", &self.child.lock().is_some())
            .finish()
    }
}

impl SidecarHandle {
    pub async fn shutdown(&self) {
        let Some(mut child) = self.child.lock().take() else {
            return;
        };
        if let Err(e) = child.start_kill() {
            warn!(error = %e, "failed to send kill signal to sidecar");
        }
        // Don't wait forever; if it doesn't die in 5s we move on.
        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    }
}

impl Drop for SidecarHandle {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.start_kill();
        }
    }
}

/// One-shot manager: launches the binary, parses the port, and hands back a
/// `SidecarHandle`. Re-spawning is the caller's responsibility (e.g. restart
/// on health-check failure).
pub struct SidecarManager;

impl SidecarManager {
    /// Spawn `llama-server` and wait for it to advertise a port on stdout.
    pub async fn spawn(spec: SidecarSpec) -> SidecarResult<Arc<SidecarHandle>> {
        let mut cmd = Command::new(&spec.binary_path);
        cmd.arg("--model")
            .arg(&spec.model_path)
            .arg("--port")
            .arg("0")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--embeddings")
            .args(&spec.extra_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        info!(
            binary = %spec.binary_path.display(),
            model = %spec.model_path.display(),
            "spawning llama-server"
        );

        let mut child = cmd
            .spawn()
            .map_err(|e| SidecarError::Spawn(e.to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SidecarError::Spawn("no stdout pipe".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SidecarError::Spawn("no stderr pipe".to_owned()))?;

        let port_task = tokio::spawn(scan_for_port(BufReader::new(stdout)));
        let stderr_task = tokio::spawn(drain_stderr(BufReader::new(stderr)));

        let port = match tokio::time::timeout(spec.startup_timeout, port_task).await {
            Ok(Ok(Some(p))) => p,
            Ok(Ok(None)) => {
                let _ = child.start_kill();
                stderr_task.abort();
                return Err(SidecarError::PortTimeout {
                    seconds: spec.startup_timeout.as_secs(),
                });
            }
            Ok(Err(e)) => {
                let _ = child.start_kill();
                stderr_task.abort();
                return Err(SidecarError::Spawn(format!("port scanner panicked: {e}")));
            }
            Err(_) => {
                let _ = child.start_kill();
                stderr_task.abort();
                return Err(SidecarError::PortTimeout {
                    seconds: spec.startup_timeout.as_secs(),
                });
            }
        };

        info!(port, "llama-server ready");
        Ok(Arc::new(SidecarHandle {
            child: Mutex::new(Some(child)),
            port,
        }))
    }
}

async fn scan_for_port<R: tokio::io::AsyncBufRead + Unpin>(mut reader: R) -> Option<u16> {
    let mut buf = String::new();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if Instant::now() > deadline {
            return None;
        }
        buf.clear();
        match reader.read_line(&mut buf).await {
            Ok(0) => return None,
            Ok(_) => {
                if let Some(p) = parse_port_from_line(&buf) {
                    return Some(p);
                }
            }
            Err(_) => return None,
        }
    }
}

async fn drain_stderr<R: tokio::io::AsyncBufRead + Unpin>(mut reader: R) {
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf).await {
            Ok(0) => break,
            Ok(_) => {
                // Don't log every line at info; only flag warnings/errors.
                let trimmed = buf.trim_end();
                if trimmed.contains("error") || trimmed.contains("ERROR") {
                    warn!(line = trimmed, "llama-server stderr");
                }
            }
            Err(_) => break,
        }
    }
}

/// Parse llama-server's "listening on 127.0.0.1:NNNN" lines. Across versions
/// llama.cpp uses slightly different phrasings, so we look for any obvious
/// hostname:port substring on a 127.0.0.1 / localhost line.
pub(crate) fn parse_port_from_line(line: &str) -> Option<u16> {
    for marker in ["127.0.0.1:", "localhost:", "0.0.0.0:"] {
        if let Some(idx) = line.find(marker) {
            let after = &line[idx + marker.len()..];
            let port_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(p) = port_str.parse::<u16>() {
                if p > 0 {
                    return Some(p);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_port_from_modern_line() {
        assert_eq!(
            parse_port_from_line("main: server is listening on http://127.0.0.1:51234 - starting"),
            Some(51234)
        );
    }

    #[test]
    fn parses_port_from_legacy_line() {
        assert_eq!(
            parse_port_from_line("HTTP server listening at localhost:8080 ..."),
            Some(8080)
        );
    }

    #[test]
    fn rejects_lines_without_loopback_marker() {
        assert!(parse_port_from_line("loading model").is_none());
        assert!(parse_port_from_line("model size: 4.07 GiB").is_none());
    }

    #[test]
    fn ignores_zero_port() {
        assert!(parse_port_from_line("listening on 127.0.0.1:0").is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_errors_when_binary_missing() {
        let spec = SidecarSpec::new(
            std::path::PathBuf::from("/nonexistent/llama-server"),
            std::path::PathBuf::from("/nonexistent/model.gguf"),
        );
        let err = SidecarManager::spawn(spec).await.unwrap_err();
        assert!(matches!(err, SidecarError::Spawn(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn handle_drop_terminates_child() {
        // Use a long-running shell process to verify Drop kills it.
        let mut cmd = Command::new("sleep");
        cmd.arg("600")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = cmd.spawn().unwrap();
        let id = child.id().unwrap();
        {
            let _h = SidecarHandle {
                child: Mutex::new(Some(child)),
                port: 12345,
            };
        } // dropped here
        // Give the kernel a moment.
        tokio::time::sleep(Duration::from_millis(200)).await;
        // /proc check: pid should be gone (or a zombie).
        let exists = std::fs::metadata(format!("/proc/{id}")).is_ok();
        // On non-Linux we'd skip this assertion; we only run tests on linux here.
        if cfg!(target_os = "linux") {
            assert!(!exists, "process should have been killed");
        }
    }
}
