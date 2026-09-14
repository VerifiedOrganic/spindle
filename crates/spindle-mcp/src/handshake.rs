//! Handshake tolerance shim for MCP stdio clients that probe the server with
//! proprietary requests before `initialize`.
//!
//! WHY: rmcp's server handshake accepts only `initialize` (plus `ping`) before
//! initialization and treats anything else as fatal — the process exits with
//! `expect initialized request, but received: ...`. Antigravity (`agy`) opens
//! the stdio pipe and sends a proprietary `server/discover` request *first*,
//! which killed Spindle before it ever advertised a tool. Standard SDK servers
//! answer unknown methods with JSON-RPC `-32601` and stay alive, so this shim
//! reproduces that behaviour in front of rmcp instead of forking it.
//!
//! The shim only inspects traffic during the handshake window (until the
//! `notifications/initialized` notification is forwarded); afterwards every
//! byte passes through untouched.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

/// JSON-RPC "method not found" code.
const METHOD_NOT_FOUND: i64 = -32601;

/// Methods rmcp's own handshake understands before initialization completes.
const HANDSHAKE_METHODS: [&str; 3] = ["initialize", "ping", "notifications/initialized"];

/// What to do with a client line seen before the MCP handshake finishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreInitAction {
    /// Pass the line through to the MCP server unchanged.
    Forward,
    /// Answer with JSON-RPC `-32601` here and never forward it.
    Reject {
        /// The request id to echo back in the error response.
        id: Value,
        /// The unsupported method name, echoed in the error message.
        method: String,
    },
    /// Discard silently — a notification cannot be answered.
    Drop,
}

/// Decide how to treat a raw client line received during the handshake window.
///
/// Anything that is not valid JSON is forwarded so rmcp reports the framing
/// error itself; this shim never invents protocol semantics it does not own.
pub fn classify_pre_init(line: &str) -> PreInitAction {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return PreInitAction::Forward;
    };
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        // Responses to server-initiated requests: not our business.
        return PreInitAction::Forward;
    };
    if HANDSHAKE_METHODS.contains(&method) {
        return PreInitAction::Forward;
    }
    match value.get("id") {
        Some(id) if !id.is_null() => PreInitAction::Reject {
            id: id.clone(),
            method: method.to_string(),
        },
        _ => PreInitAction::Drop,
    }
}

/// True once this forwarded line closes the handshake window.
pub fn completes_handshake(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(|method| method == "notifications/initialized")
        })
        .unwrap_or(false)
}

/// Render the JSON-RPC error a standard MCP server would return for an
/// unsupported method, newline-framed for the stdio transport.
pub fn method_not_found_response(id: &Value, method: &str) -> String {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": METHOD_NOT_FOUND,
            "message": format!("Method not found: {method}"),
        }
    });
    format!("{body}\n")
}

/// Bridge a client byte stream to an MCP server duplex, tolerating
/// pre-`initialize` probes.
///
/// Runs until the client stream closes or the server side drops.
pub async fn bridge_with_shim<R, W>(
    client_in: R,
    client_out: W,
    server: tokio::io::DuplexStream,
) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (server_read, mut server_write) = tokio::io::split(server);
    // Single owner of the client sink: the reader task (shim replies) and the
    // server task (real responses) both publish through this channel, so the
    // two producers can never interleave a half-written line.
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);

    let mut client_out = client_out;
    let to_client = tokio::spawn(async move {
        while let Some(chunk) = rx.recv().await {
            if client_out.write_all(&chunk).await.is_err() {
                break;
            }
            if client_out.flush().await.is_err() {
                break;
            }
        }
    });

    let shim_tx = tx.clone();
    let to_server = tokio::spawn(async move {
        let mut lines = BufReader::new(client_in).lines();
        let mut handshaking = true;
        while let Ok(Some(line)) = lines.next_line().await {
            if handshaking {
                match classify_pre_init(&line) {
                    PreInitAction::Reject { id, method } => {
                        tracing::debug!(
                            %method,
                            "answering pre-initialize probe with method-not-found"
                        );
                        let reply = method_not_found_response(&id, &method);
                        if shim_tx.send(reply.into_bytes()).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    PreInitAction::Drop => {
                        tracing::debug!("dropping pre-initialize notification");
                        continue;
                    }
                    PreInitAction::Forward => {
                        if completes_handshake(&line) {
                            handshaking = false;
                        }
                    }
                }
            }
            let mut msg = line.into_bytes();
            msg.push(b'\n');
            if server_write.write_all(&msg).await.is_err() {
                break;
            }
        }
        // Client pipe closed: propagate EOF so the MCP server ends its session
        // and the process can exit. `split` keeps the duplex alive while the
        // read half lives, so dropping this half is not enough.
        let _ = server_write.shutdown().await;
    });

    // Server → client: raw byte copy. Tool listings run to hundreds of
    // kilobytes on one line, so never buffer whole lines here.
    let mut server_read = server_read;
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match server_read.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
        }
    }
    drop(tx);

    let _ = to_client.await;
    to_server.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    const DISCOVER: &str = r#"{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{}}"#;
    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{}}"#;
    const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;

    #[test]
    fn handshake_methods_are_forwarded() {
        for line in [
            INITIALIZE,
            INITIALIZED,
            r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#,
        ] {
            assert_eq!(classify_pre_init(line), PreInitAction::Forward, "{line}");
        }
    }

    #[test]
    fn unknown_pre_init_request_is_rejected_with_its_id() {
        assert_eq!(
            classify_pre_init(DISCOVER),
            PreInitAction::Reject {
                id: json!(1),
                method: "server/discover".to_string(),
            }
        );
        assert_eq!(
            classify_pre_init(r#"{"jsonrpc":"2.0","id":"abc","method":"vendor/probe"}"#),
            PreInitAction::Reject {
                id: json!("abc"),
                method: "vendor/probe".to_string(),
            }
        );
    }

    #[test]
    fn unknown_pre_init_notification_is_dropped() {
        assert_eq!(
            classify_pre_init(r#"{"jsonrpc":"2.0","method":"vendor/hello"}"#),
            PreInitAction::Drop
        );
        assert_eq!(
            classify_pre_init(r#"{"jsonrpc":"2.0","id":null,"method":"vendor/hello"}"#),
            PreInitAction::Drop
        );
    }

    #[test]
    fn malformed_and_response_lines_are_forwarded_untouched() {
        assert_eq!(classify_pre_init("not json"), PreInitAction::Forward);
        assert_eq!(classify_pre_init(""), PreInitAction::Forward);
        assert_eq!(
            classify_pre_init(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#),
            PreInitAction::Forward
        );
    }

    #[test]
    fn only_initialized_notification_closes_the_window() {
        assert!(completes_handshake(INITIALIZED));
        assert!(!completes_handshake(INITIALIZE));
        assert!(!completes_handshake("not json"));
    }

    #[test]
    fn method_not_found_response_is_wire_shaped() {
        let raw = method_not_found_response(&json!(1), "server/discover");
        assert!(raw.ends_with('\n'), "must be newline framed");
        let parsed: Value = serde_json::from_str(raw.trim()).expect("valid json");
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], json!(1));
        assert_eq!(parsed["error"]["code"], json!(METHOD_NOT_FOUND));
        assert_eq!(
            parsed["error"]["message"],
            json!("Method not found: server/discover")
        );
        assert!(parsed.get("result").is_none());
    }

    /// The Antigravity failure, end to end: `server/discover` must be answered
    /// by the shim and must never reach rmcp, while `initialize` still lands.
    #[tokio::test]
    async fn discover_probe_is_answered_and_never_reaches_the_server() {
        let (mut client, client_side) = tokio::io::duplex(8 * 1024);
        let (client_in, client_out) = tokio::io::split(client_side);
        let (server_side, mut fake_server) = tokio::io::duplex(8 * 1024);

        tokio::spawn(bridge_with_shim(client_in, client_out, server_side));

        client
            .write_all(format!("{DISCOVER}\n{INITIALIZE}\n{INITIALIZED}\n").as_bytes())
            .await
            .expect("client write");

        // The server only ever sees the two handshake messages.
        let mut server_lines = BufReader::new(&mut fake_server).lines();
        let first = server_lines
            .next_line()
            .await
            .expect("server read")
            .expect("initialize line");
        assert_eq!(first, INITIALIZE);
        let second = server_lines
            .next_line()
            .await
            .expect("server read")
            .expect("initialized line");
        assert_eq!(second, INITIALIZED);

        // The client got a well-formed method-not-found for the probe.
        let mut client_lines = BufReader::new(&mut client).lines();
        let reply = client_lines
            .next_line()
            .await
            .expect("client read")
            .expect("shim reply");
        let parsed: Value = serde_json::from_str(&reply).expect("valid json");
        assert_eq!(parsed["id"], json!(1));
        assert_eq!(parsed["error"]["code"], json!(METHOD_NOT_FOUND));
    }

    /// Closing the client pipe must close the server side too, or the MCP
    /// process outlives its client and orphans the workspace DB lock.
    #[tokio::test]
    async fn client_eof_closes_the_server_side() {
        let (client, client_side) = tokio::io::duplex(8 * 1024);
        let (client_in, client_out) = tokio::io::split(client_side);
        let (server_side, mut fake_server) = tokio::io::duplex(8 * 1024);

        tokio::spawn(bridge_with_shim(client_in, client_out, server_side));

        drop(client);

        let mut buf = Vec::new();
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fake_server.read_to_end(&mut buf),
        )
        .await
        .expect("server side must observe EOF, not hang");
        assert_eq!(read.expect("read to end"), 0);
    }

    /// After the handshake, unknown methods belong to rmcp — the shim must not
    /// swallow them (that is where real MCP error semantics live).
    #[tokio::test]
    async fn post_handshake_traffic_passes_through_both_ways() {
        let (mut client, client_side) = tokio::io::duplex(8 * 1024);
        let (client_in, client_out) = tokio::io::split(client_side);
        let (server_side, mut fake_server) = tokio::io::duplex(64 * 1024);

        tokio::spawn(bridge_with_shim(client_in, client_out, server_side));

        let late_probe = r#"{"jsonrpc":"2.0","id":7,"method":"server/discover"}"#;
        client
            .write_all(format!("{INITIALIZE}\n{INITIALIZED}\n{late_probe}\n").as_bytes())
            .await
            .expect("client write");

        let mut server_lines = BufReader::new(&mut fake_server).lines();
        for expected in [INITIALIZE, INITIALIZED, late_probe] {
            let line = server_lines
                .next_line()
                .await
                .expect("server read")
                .expect("line");
            assert_eq!(line, expected);
        }

        // A large single-line response (tool listings are ~500 KB) survives.
        let big = json!({"jsonrpc":"2.0","id":7,"result":{"blob":"x".repeat(40_000)}});
        fake_server
            .write_all(format!("{big}\n").as_bytes())
            .await
            .expect("server write");

        let mut client_lines = BufReader::new(&mut client).lines();
        let echoed = client_lines
            .next_line()
            .await
            .expect("client read")
            .expect("line");
        assert_eq!(echoed, big.to_string());
    }
}
