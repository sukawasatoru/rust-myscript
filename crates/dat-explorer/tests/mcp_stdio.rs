/*
 * Copyright 2026 sukawasatoru
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct ServerProcess(Child);

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn send(server: &mut ServerProcess, message: Value) {
    let stdin = server.0.stdin.as_mut().unwrap();
    writeln!(stdin, "{message}").unwrap();
    stdin.flush().unwrap();
}

fn receive(receiver: &mpsc::Receiver<String>, id: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let line = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        let value: Value =
            serde_json::from_str(&line).expect("stdout must only contain JSON-RPC messages");
        if value["id"] == id {
            assert!(value.get("error").is_none(), "{value}");
            return value["result"].clone();
        }
    }
}

#[test]
fn mcp_subcommand_serves_stdio_and_honors_disable_body_limit() {
    let dir = tempfile::TempDir::new().unwrap();
    let body = "本文".repeat(30000);
    std::fs::write(
        dir.path().join("board_700_123.dat"),
        format!("名無し<><>2026/01/01 ID:a<>{body}<>タイトル\n名無し<><>2026/01/02 ID:b<>二番目<>"),
    )
    .unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_dat-explorer"))
        .arg("mcp")
        .arg(dir.path())
        .arg("--disable-body-limit")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut server = ServerProcess(child);
    let stdout = server.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    send(
        &mut server,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-03-26", "capabilities": {},
                "clientInfo": {"name": "stdio-test", "version": "1"}}
        }),
    );
    let init = receive(&receiver, 1);
    assert_eq!(init["serverInfo"]["name"], "dat-explorer");
    send(
        &mut server,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    );
    send(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "read_posts", "arguments": {"file": "700", "include_urls": true}
        }}),
    );
    let result = receive(&receiver, 2);
    assert_ne!(result["isError"], true);
    let parsed = &result["structuredContent"];
    assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["rows"][0][2], body);
    assert_eq!(parsed["rows"][1][0], 2);
    assert!(parsed.get("omitted_count").is_none());
    drop(server);
    reader.join().unwrap();
}
