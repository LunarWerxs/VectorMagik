//! The engine's MCP server (`--mcp`) over its real pipes, as an AI
//! assistant drives it: the handshake, the tool list, a conversion that
//! saves its file and shows a picture of it, and a refusal that is the
//! tool's answer while the server carries on. Every line on stdout must be
//! a JSON-RPC message: one stray print on the conversion path breaks the
//! protocol for every client.
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[test]
fn an_assistant_converts_a_picture_through_the_mcp_server() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = root
        .join("work/app-tests")
        .join(format!("{}-mcp", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = root.join("kit/fixtures/samples/logo-without-blending.png");
    let output = dir.join("logo.svg");
    let _ = std::fs::remove_file(&output);

    let mut server = Command::new(env!("CARGO_BIN_EXE_vector-magic-rebuild"))
        .arg("--mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let requests = [
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-03-26", "capabilities": {},
            "clientInfo": {"name": "test", "version": "1"}}}),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "name": "vectorize",
            "arguments": {"input": source.to_str().unwrap(), "output": output.to_str().unwrap()}}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
            "name": "vectorize", "arguments": {"input": dir.join("missing.png").to_str().unwrap()}}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 5, "method": "ping"}),
    ];
    let mut stdin = server.stdin.take().unwrap();
    for request in &requests {
        writeln!(stdin, "{request}").unwrap();
    }
    drop(stdin);
    let replies: Vec<serde_json::Value> = BufReader::new(server.stdout.take().unwrap())
        .lines()
        .map(|line| {
            let line = line.unwrap();
            serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON-RPC: {line:?}: {e}"))
        })
        .collect();
    assert!(server.wait().unwrap().success());
    // One reply per request, none to the notification, in order.
    let ids: Vec<i64> = replies.iter().map(|r| r["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, [1, 2, 3, 4, 5], "{replies:?}");

    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
    let tools: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["vectorize", "inspect", "view"]);

    let converted = &replies[2]["result"];
    assert_eq!(converted["isError"], false, "{converted}");
    let report = converted["content"][0]["text"].as_str().unwrap();
    assert!(report.contains("\"backend\":\"recovered-rust\""), "{report}");
    assert!(report.contains("Saved "), "{report}");
    let saved = std::fs::read_to_string(&output).unwrap();
    assert!(saved.contains("<svg") && saved.contains("<path"), "{saved:.200}");
    #[cfg(feature = "render")]
    {
        use base64::Engine as _;
        let picture = &converted["content"][1];
        assert_eq!(picture["type"], "image");
        assert_eq!(picture["mimeType"], "image/png");
        let png = base64::engine::general_purpose::STANDARD
            .decode(picture["data"].as_str().unwrap())
            .unwrap();
        let preview = image::load_from_memory(&png).unwrap();
        assert_eq!(preview.width().max(preview.height()), 1024);
    }

    let refused = &replies[3]["result"];
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(refused["content"][0]["text"].as_str().unwrap().contains("missing.png"));
    assert_eq!(replies[4]["result"], serde_json::json!({}));
}
