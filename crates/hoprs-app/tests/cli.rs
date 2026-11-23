//! The `hoprs` binary's argument handling, end to end: these paths exit before the
//! instance lock and the window.

use std::process::Command;

fn hoprs(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hoprs"))
        .args(args)
        .output()
        .expect("run hoprs")
}

#[test]
fn version_prints_name_and_version() {
    for flag in ["--version", "-V"] {
        let out = hoprs(&[flag]);
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("hoprs {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn help_prints_usage_and_exits_zero() {
    let out = hoprs(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("Usage: hoprs"));
}

#[test]
fn unknown_argument_exits_two() {
    let out = hoprs(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--bogus"));
}

#[test]
fn help_names_mcp() {
    let out = hoprs(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "Usage: hoprs [mcp | --version | --help]\n"
    );
}

/// `hoprs mcp`: the relay an MCP client starts. Its channel directory is an empty
/// temporary one (debug builds only read `HOPRS_AGENT_DIR`), so hoprs is never reachable.
mod mcp {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};

    /// A `hoprs mcp` with its standard streams as pipes, and a thread that hands on
    /// each line of its stdout.
    struct Relay {
        child: Child,
        stdin: Option<ChildStdin>,
        lines: mpsc::Receiver<String>,
        stderr: std::thread::JoinHandle<String>,
        _dir: tempfile::TempDir,
    }

    const WAIT: Duration = Duration::from_secs(10);

    impl Relay {
        fn start() -> Relay {
            let dir = tempfile::tempdir().unwrap();
            let mut child = Command::new(env!("CARGO_BIN_EXE_hoprs"))
                .arg("mcp")
                .env("HOPRS_AGENT_DIR", dir.path())
                .env_remove("RUST_LOG")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("run hoprs mcp");
            let stdout = child.stdout.take().unwrap();
            let (tx, lines) = mpsc::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { return };
                    if tx.send(line).is_err() {
                        return;
                    }
                }
            });
            let mut stderr = child.stderr.take().unwrap();
            let stderr = std::thread::spawn(move || {
                let mut text = String::new();
                let _ = std::io::Read::read_to_string(&mut stderr, &mut text);
                text
            });
            Relay {
                stdin: child.stdin.take(),
                child,
                lines,
                stderr,
                _dir: dir,
            }
        }

        fn send(&mut self, line: &str) {
            let stdin = self.stdin.as_mut().unwrap();
            stdin.write_all(line.as_bytes()).unwrap();
            stdin.write_all(b"\n").unwrap();
            stdin.flush().unwrap();
        }

        fn request(&mut self, id: u64, method: &str, params: Value) {
            let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
            self.send(&message.to_string());
        }

        /// The next stdout line, which must be one JSON-RPC message.
        fn recv(&self) -> Value {
            let line = self
                .lines
                .recv_timeout(WAIT)
                .expect("a line from hoprs mcp");
            protocol_line(&line)
        }

        /// The legacy opening: `initialize`, its answer, `notifications/initialized`.
        fn open(&mut self) {
            self.request(
                1,
                "initialize",
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0"},
                }),
            );
            let answer = self.recv();
            assert_eq!(answer["id"], 1, "{answer}");
            assert_eq!(answer["result"]["serverInfo"]["name"], "hoprs");
            self.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        }

        fn tool_names(&mut self, id: u64) -> Vec<String> {
            self.request(id, "tools/list", json!({}));
            let answer = self.recv();
            assert_eq!(answer["id"], id, "{answer}");
            answer["result"]["tools"]
                .as_array()
                .expect("a tool list")
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_owned())
                .collect()
        }

        /// Closes stdin and waits (at most `within`) for the exit status.
        fn close(mut self, within: Duration) -> (Option<i32>, Vec<Value>, String) {
            drop(self.stdin.take());
            let started = Instant::now();
            let status = loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    break status;
                }
                if started.elapsed() > within {
                    let _ = self.child.kill();
                    panic!("hoprs mcp still running {within:?} after its input closed");
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            let rest: Vec<Value> = self.lines.iter().map(|l| protocol_line(&l)).collect();
            (status.code(), rest, self.stderr.join().unwrap())
        }
    }

    /// A line of the relay's stdout: one JSON object with `"jsonrpc": "2.0"`.
    fn protocol_line(line: &str) -> Value {
        let value: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("stdout carried a line that is not JSON ({e}): {line:?}"));
        assert!(value.is_object(), "not an object: {line:?}");
        assert_eq!(value["jsonrpc"], "2.0", "not JSON-RPC: {line:?}");
        value
    }

    #[test]
    fn stdout_is_protocol_only() {
        if !cfg!(debug_assertions) {
            return;
        }
        let mut relay = Relay::start();
        relay.open();
        assert_eq!(relay.tool_names(2).len(), 13);
        relay.request(
            3,
            "tools/call",
            json!({"name": "list_sites", "arguments": {}}),
        );
        let answer = relay.recv();
        assert_eq!(answer["id"], 3, "{answer}");
        assert_eq!(answer["result"]["isError"], true, "{answer}");
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Settings › Agents (MCP)"), "{text}");
        relay.send("this is not json {");
        relay.send("");
        let (code, rest, stderr) = relay.close(Duration::from_secs(2));
        assert_eq!(code, Some(0), "stderr: {stderr}");
        // Every line was checked as it was read; none was a log line or a notice.
        for line in &rest {
            assert!(line.is_object());
        }
        assert!(
            !stderr.contains("not json"),
            "a line of input was logged: {stderr}"
        );
    }

    #[test]
    fn mcp_does_not_take_the_instance_lock() {
        if !cfg!(debug_assertions) {
            return;
        }
        let mut first = Relay::start();
        first.open();
        assert_eq!(first.tool_names(2).len(), 13);
        // The first is still running (its input is open) while the second starts.
        let mut second = Relay::start();
        second.open();
        assert_eq!(second.tool_names(2).len(), 13);
        assert_eq!(first.tool_names(3).len(), 13);
        for relay in [first, second] {
            let (code, _, stderr) = relay.close(Duration::from_secs(2));
            assert_eq!(code, Some(0), "stderr: {stderr}");
        }
    }
}
