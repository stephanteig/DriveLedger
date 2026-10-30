//! MCP server tests over stdio: the same answers as the CLI's `--json`.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};

use common::{Env, bin, index_id, setup};
use serde_json::{Value, json};

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn start(env: &Env) -> Self {
        let mut child = bin()
            .args(["mcp", "--library"])
            .arg(&env.library)
            .env("DRIVELEDGER_CONFIG_DIR", &env.config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = McpClient {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        let init = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "driveledger-test", "version": "0" }
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "driveledger");
        client.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        client
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Send a request and return its response, skipping notifications.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "server closed"
            );
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == id {
                return message;
            }
        }
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let response = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        assert!(response["error"].is_null(), "{tool}: {response}");
        response["result"].clone()
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn tools_are_listed_with_schemas_and_hints() {
    let env = setup();
    let mut client = McpClient::start(&env);
    let tools = client.request("tools/list", json!({}))["result"]["tools"].clone();
    let mut names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "compare",
            "find_duplicates",
            "find_file",
            "folder_overview",
            "list_indexes",
            "list_volumes",
            "scan",
            "search_files",
        ]
    );
    for tool in tools.as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert_eq!(tool["outputSchema"]["type"], "object", "{name}");
        let read_only = tool["annotations"]["readOnlyHint"].as_bool();
        if name == "scan" {
            assert_eq!(read_only, Some(false));
            assert_eq!(tool["annotations"]["destructiveHint"], false);
        } else {
            assert_eq!(read_only, Some(true), "{name}");
        }
    }
}

#[test]
fn compare_matches_cli_json() {
    let env = setup();
    let src = index_id(&env.json(&["scan", env.source.to_str().unwrap(), "--json"]));
    let tgt = index_id(&env.json(&["scan", env.target.to_str().unwrap(), "--json"]));
    let cli = env.json(&["compare", &src, &tgt, "--json"]);

    let mut client = McpClient::start(&env);
    let result = client.call(
        "compare",
        json!({ "source": src, "target": tgt, "limit": 5000 }),
    );
    assert_eq!(result["isError"], false);
    let mcp = &result["structuredContent"];
    for key in ["source", "target", "summary", "folders", "warnings"] {
        assert_eq!(mcp[key], cli[key], "{key} differs between MCP and CLI");
    }
    assert_eq!(mcp["files"]["items"], cli["files"]);
    assert_eq!(mcp["files"]["next_cursor"], Value::Null);
    let summary = result["content"][0]["text"].as_str().unwrap();
    assert!(summary.contains("Content NOT found: 2"), "{summary}");

    // Paging walks the same list.
    let first = client.call(
        "compare",
        json!({ "source": src, "target": tgt, "limit": 3 }),
    );
    let cursor = first["structuredContent"]["files"]["next_cursor"].clone();
    let second = client.call(
        "compare",
        json!({ "source": src, "target": tgt, "limit": 3, "cursor": cursor }),
    );
    let mut paged: Vec<Value> = first["structuredContent"]["files"]["items"]
        .as_array()
        .unwrap()
        .clone();
    paged.extend(
        second["structuredContent"]["files"]["items"]
            .as_array()
            .unwrap()
            .clone(),
    );
    assert_eq!(Value::Array(paged), cli["files"]);
}

#[test]
fn unmounted_target_is_an_error_not_missing_files() {
    let env = setup();
    let src = index_id(&env.json(&["scan", env.source.to_str().unwrap(), "--json"]));
    let mut client = McpClient::start(&env);
    let result = client.call(
        "compare",
        json!({ "source": src, "target": "/Volumes/DriveLedger-test-not-mounted/ByHola" }),
    );
    assert_eq!(result["isError"], true);
    assert_eq!(
        result["structuredContent"]["error"]["kind"],
        "target_unavailable"
    );
}

#[test]
fn scan_tool_creates_an_index_usable_by_other_tools() {
    let env = setup();
    let mut client = McpClient::start(&env);
    let scan = client.call(
        "scan",
        json!({ "path": env.target.to_str().unwrap(), "wait_seconds": 60 }),
    );
    let status = &scan["structuredContent"];
    assert_eq!(status["state"], "done", "{scan}");
    let id = status["index_id"].as_str().unwrap().to_string();

    let indexes = client.call("list_indexes", json!({}));
    assert_eq!(indexes["structuredContent"]["indexes"]["total"], 1);

    let hits = client.call("search_files", json!({ "glob": "*.mp4" }));
    assert_eq!(hits["structuredContent"]["total"], 1);
    let fingerprint = hits["structuredContent"]["items"][0]["file"]["fingerprint"].clone();

    let found = client.call(
        "find_file",
        json!({ "index": id, "path": "Other/SW-FX30001.MP4" }),
    );
    assert_eq!(found["structuredContent"]["fingerprint"], fingerprint);
    assert_eq!(
        found["structuredContent"]["locations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let overview = client.call("folder_overview", json!({ "index": id, "depth": 1 }));
    assert_eq!(overview["structuredContent"]["folders"]["total"], 2);

    let dups = client.call("find_duplicates", json!({}));
    assert_eq!(dups["structuredContent"]["groups"]["total"], 0);
}

#[test]
fn http_refuses_non_loopback_addresses() {
    let env = setup();
    let out = bin()
        .args(["mcp", "--http", "0.0.0.0:7341", "--library"])
        .arg(&env.library)
        .env("DRIVELEDGER_CONFIG_DIR", &env.config)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("only binds to loopback"));
}
