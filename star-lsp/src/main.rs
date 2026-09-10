use std::env;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcMessage {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<serde_json::Value>,
}

// C* compiler diagnostics types
#[derive(Deserialize, Debug, Clone)]
struct CStarSpan {
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
}

#[derive(Deserialize, Debug, Clone)]
struct CStarNote {
    message: String,
    span: CStarSpan,
}

#[derive(Deserialize, Debug, Clone)]
struct CStarDiagnostic {
    code: String,
    severity: String,
    message: String,
    file: String,
    span: CStarSpan,
    #[serde(default)]
    notes: Vec<CStarNote>,
}

#[derive(Deserialize, Debug)]
struct CStarReport {
    version: String,
    diagnostics: Vec<CStarDiagnostic>,
}

// LSP types
#[derive(Serialize, Debug)]
struct LspPosition {
    line: usize,
    character: usize,
}

#[derive(Serialize, Debug)]
struct LspRange {
    start: LspPosition,
    end: LspPosition,
}

#[derive(Serialize, Debug)]
struct LspDiagnostic {
    range: LspRange,
    severity: i32,
    code: String,
    source: String,
    message: String,
}

#[derive(Serialize, Debug)]
struct PublishDiagnosticsParams {
    uri: String,
    diagnostics: Vec<LspDiagnostic>,
}

fn percent_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let mut hex = String::new();
            if let Some(h1) = chars.next() { hex.push(h1); }
            if let Some(h2) = chars.next() { hex.push(h2); }
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn uri_to_path(uri: &str) -> Option<String> {
    if uri.starts_with("file://") {
        let mut path_str = uri.trim_start_matches("file://");
        if path_str.starts_with('/') && cfg!(windows) {
            path_str = &path_str[1..];
        }
        let decoded = percent_decode(path_str);
        if cfg!(windows) {
            Some(decoded.replace('/', "\\"))
        } else {
            Some(decoded)
        }
    } else {
        None
    }
}

fn find_starc() -> Option<PathBuf> {
    if let Ok(current_exe) = env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let starc_path = dir.join(if cfg!(windows) { "starc.exe" } else { "starc" });
            if starc_path.exists() {
                return Some(starc_path);
            }
        }
    }
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let debug_starc = workspace_root.join("target").join("debug").join(if cfg!(windows) { "starc.exe" } else { "starc" });
    if debug_starc.exists() {
        return Some(debug_starc);
    }
    if let Ok(path_var) = env::var("PATH") {
        for path in env::split_paths(&path_var) {
            let p = path.join(if cfg!(windows) { "starc.exe" } else { "starc" });
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn send_response(msg: JsonRpcMessage) {
    let json = serde_json::to_string(&msg).unwrap();
    let response = format!("Content-Length: {}\r\n\r\n{}", json.len(), json);
    let mut stdout = io::stdout();
    let _ = stdout.write_all(response.as_bytes());
    let _ = stdout.flush();
}

fn send_notification(method: &str, params: serde_json::Value) {
    let msg = JsonRpcMessage {
        jsonrpc: "2.0".to_string(),
        id: None,
        method: Some(method.to_string()),
        params: Some(params),
        result: None,
        error: None,
    };
    send_response(msg);
}

fn handle_diagnostics(file_path: &str, uri: &str, starc_path: &Path) {
    let mut cmd = Command::new(starc_path);
    cmd.arg(file_path)
       .arg("--error-format=json");

    // Forward LLVM target configurations
    let env_path = format!("C:\\PROGRA~1\\LLVM\\bin;{}", env::var("PATH").unwrap_or_default());
    cmd.env("PATH", &env_path);

    let output = match cmd.output() {
        Ok(out) => out,
        Err(e) => {
            eprintln!("[LSP] Failed to run compiler diagnostics: {}", e);
            return;
        }
    };

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stderr_str = String::from_utf8_lossy(&output.stderr);

    // Diagnostics are printed to stdout in JSON format
    let report_str = if stdout_str.trim().starts_with('{') {
        stdout_str.trim()
    } else if stderr_str.trim().starts_with('{') {
        stderr_str.trim()
    } else {
        ""
    };

    let mut lsp_diagnostics = Vec::new();

    if !report_str.is_empty() {
        if let Ok(report) = serde_json::from_str::<CStarReport>(report_str) {
            for diag in report.diagnostics {
                let severity = match diag.severity.as_str() {
                    "error" => 1,
                    "warning" => 2,
                    _ => 3,
                };

                // LSP positions are 0-indexed
                let range = LspRange {
                    start: LspPosition {
                        line: diag.span.start_line.saturating_sub(1),
                        character: diag.span.start_col.saturating_sub(1),
                    },
                    end: LspPosition {
                        line: diag.span.end_line.saturating_sub(1),
                        character: diag.span.end_col.saturating_sub(1),
                    },
                };

                let mut message = diag.message.clone();
                for note in diag.notes {
                    message.push_str(&format!("\nNote: {} (line {})", note.message, note.span.start_line));
                }

                lsp_diagnostics.push(LspDiagnostic {
                    range,
                    severity,
                    code: diag.code,
                    source: "starc".to_string(),
                    message,
                });
            }
        }
    }

    let params = serde_json::to_value(PublishDiagnosticsParams {
        uri: uri.to_string(),
        diagnostics: lsp_diagnostics,
    }).unwrap();

    send_notification("textDocument/publishDiagnostics", params);
}

fn main() {
    eprintln!("[LSP] Starting C* Language Server (star-lsp)");

    let starc_path = match find_starc() {
        Some(p) => p,
        None => {
            eprintln!("[LSP] Error: Could not find starc compiler. Diagnostics will not work.");
            process_exit_fallback();
            return;
        }
    };

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        let mut line = String::new();
        // 1. Read headers
        let mut content_length = 0;
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 {
                return; // EOF
            }
            if line == "\r\n" || line == "\n" {
                break; // Header section finished
            }
            if line.to_lowercase().starts_with("content-length:") {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 2 {
                    content_length = parts[1].trim().parse::<usize>().unwrap_or(0);
                }
            }
        }

        if content_length == 0 {
            continue;
        }

        // 2. Read JSON body
        let mut body_buf = vec![0u8; content_length];
        reader.read_exact(&mut body_buf).unwrap();

        let raw_msg: serde_json::Value = match serde_json::from_slice(&body_buf) {
            Ok(val) => val,
            Err(e) => {
                eprintln!("[LSP] Error parsing JSON-RPC: {}", e);
                continue;
            }
        };

        if let Ok(msg) = serde_json::from_value::<JsonRpcMessage>(raw_msg.clone()) {
            if let Some(ref method) = msg.method {
                match method.as_str() {
                    "initialize" => {
                        let capabilities = serde_json::json!({
                            "capabilities": {
                                "textDocumentSync": 1 // Full synchronization
                            }
                        });
                        let response = JsonRpcMessage {
                            jsonrpc: "2.0".to_string(),
                            id: msg.id,
                            method: None,
                            params: None,
                            result: Some(capabilities),
                            error: None,
                        };
                        send_response(response);
                    }
                    "textDocument/didOpen" | "textDocument/didSave" | "textDocument/didChange" => {
                        if let Some(ref params) = msg.params {
                            if let Some(uri_val) = params.get("textDocument").and_then(|td| td.get("uri")) {
                                if let Some(uri) = uri_val.as_str() {
                                    if let Some(file_path) = uri_to_path(uri) {
                                        handle_diagnostics(&file_path, uri, &starc_path);
                                    }
                                }
                            }
                        }
                    }
                    "exit" | "shutdown" => {
                        return;
                    }
                    _ => {
                        // Return empty result for unsupported requests to satisfy LSP client
                        if msg.id.is_some() {
                            let response = JsonRpcMessage {
                                jsonrpc: "2.0".to_string(),
                                id: msg.id,
                                method: None,
                                params: None,
                                result: Some(serde_json::json!({})),
                                error: None,
                            };
                            send_response(response);
                        }
                    }
                }
            }
        }
    }
}

fn process_exit_fallback() {
    std::process::exit(1);
}
