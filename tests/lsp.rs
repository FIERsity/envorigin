//! End-to-end LSP session over stdio: initialize, didOpen, hover,
//! go-to-definition, shutdown, exit.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};

struct LspClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl LspClient {
    fn start() -> Self {
        let bin = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/envorigin");
        let mut child = Command::new(bin)
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn envorigin lsp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, message: Value) {
        let payload = serde_json::to_vec(&message).expect("serialize");
        self.stdin
            .write_all(format!("Content-Length: {}\r\n\r\n", payload.len()).as_bytes())
            .expect("write header");
        self.stdin.write_all(&payload).expect("write body");
        self.stdin.flush().expect("flush");
    }

    fn receive(&mut self) -> Value {
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).expect("read header line");
            if line == "\r\n" {
                break;
            }
            headers.push_str(&line);
        }
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length:"))
            .expect("content-length header")
            .trim()
            .parse()
            .expect("parse length");
        let mut payload = vec![0u8; length];
        self.stdout.read_exact(&mut payload).expect("read body");
        serde_json::from_slice(&payload).expect("parse json")
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.receive()
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Receive the next message matching `method`, skipping unrelated
    /// notifications (e.g. window/logMessage).
    fn receive_until(&mut self, method: &str) -> Value {
        loop {
            let message = self.receive();
            if message["method"].as_str() == Some(method) {
                return message;
            }
        }
    }

    fn shutdown(mut self) -> std::io::Result<()> {
        self.send(json!({"jsonrpc": "2.0", "id": 99, "method": "shutdown"}));
        self.receive();
        self.notify("exit", json!({}));
        drop(self.stdin);
        self.child.wait_with_output()?;
        Ok(())
    }
}

trait ReadExact {
    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()>;
}

impl ReadExact for BufReader<ChildStdout> {
    fn read_exact(&mut self, buf: &mut [u8]) -> std::io::Result<()> {
        std::io::Read::read_exact(self, buf)
    }
}

#[test]
fn lsp_session_hover_definition_and_diagnostics() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/precedence/compose.yaml");
    let uri = format!("file://{}", fixture.display());
    let content = std::fs::read_to_string(&fixture).unwrap();

    let mut client = LspClient::start();
    let initialize = client.request(
        1,
        "initialize",
        json!({"capabilities": {}, "processId": null}),
    );
    let capabilities = &initialize["result"]["capabilities"];
    assert_eq!(capabilities["hoverProvider"], true);
    assert_eq!(capabilities["definitionProvider"], true);

    client.notify("initialized", json!({}));
    client.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "languageId": "yaml",
                "version": 1,
                "text": content,
            }
        }),
    );

    // The server publishes diagnostics after didOpen.
    let _published = client.receive_until("textDocument/publishDiagnostics");

    // Hover over the P definition line (compose.yaml:9 -> 0-based line 8).
    let hover = client.request(
        2,
        "textDocument/hover",
        json!({"textDocument": {"uri": uri}, "position": {"line": 8, "character": 0}}),
    );
    let hover_text = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(
        hover_text.contains("P"),
        "hover names the variable: {hover_text}"
    );
    assert!(
        hover_text.contains("ServiceEnvironment"),
        "hover names the source kind: {hover_text}"
    );

    // Go-to-definition jumps to the winner source line (9 -> 0-based 8).
    let definition = client.request(
        3,
        "textDocument/definition",
        json!({"textDocument": {"uri": uri}, "position": {"line": 8, "character": 0}}),
    );
    let location = &definition["result"];
    assert_eq!(location["uri"], uri);
    assert_eq!(location["range"]["start"]["line"], 8);

    client.shutdown().expect("graceful shutdown");
}

#[test]
fn lsp_unknown_file_type_yields_no_diagnostics() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic/.env");
    let uri = format!("file://{}", fixture.display());
    let content = std::fs::read_to_string(&fixture).unwrap();

    let mut client = LspClient::start();
    client.request(1, "initialize", json!({"capabilities": {}}));
    client.notify("initialized", json!({}));
    client.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": {"uri": uri, "languageId": "dotenv", "version": 1, "text": content}
        }),
    );
    let published = client.receive_until("textDocument/publishDiagnostics");
    assert_eq!(
        published["params"]["diagnostics"].as_array().unwrap().len(),
        0
    );
    client.shutdown().expect("graceful shutdown");
}

// Keep Duration import used for possible future timeouts.
#[allow(dead_code)]
fn _timeout_hint() -> Duration {
    Duration::from_secs(10)
}
