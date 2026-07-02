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
#![cfg(target_os = "macos")]

use std::process::Command;

fn bin() -> std::path::PathBuf {
    // For integration tests (tests/*.rs), Cargo always defines this.
    env!("CARGO_BIN_EXE_mytimeout").into()
}

fn slow_child() -> Vec<&'static str> {
    vec!["sh", "-c", "sleep 0.3"]
}

fn quick_child() -> Vec<&'static str> {
    vec!["sh", "-c", "exit 0"]
}

fn quick_child_42() -> Vec<&'static str> {
    vec!["sh", "-c", "exit 42"]
}

#[test]
fn timeout_returns_124() {
    let exe = bin();
    let slow = slow_child();
    let mut cmd = Command::new(exe);
    cmd.arg("0.1").args(&slow);
    let status = cmd.status().unwrap();
    assert_eq!(status.code(), Some(124));
}

#[test]
fn preserve_status_on_timeout() {
    let exe = bin();
    let slow = slow_child();
    let mut cmd = Command::new(exe);
    cmd.arg("-p").arg("0.1").args(&slow);
    let status = cmd.status().unwrap();
    let code = status.code().unwrap();
    assert_ne!(code, 124, "expected preserved child code, got 124");
}

#[test]
fn zero_duration_runs_child() {
    let exe = bin();
    let quick = quick_child();
    let mut cmd = Command::new(exe);
    cmd.arg("0").args(&quick);
    let status = cmd.status().unwrap();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn normal_run_returns_child_code() {
    let exe = bin();
    let quick = quick_child_42();
    let mut cmd = Command::new(exe);
    cmd.arg("5").args(&quick);
    let status = cmd.status().unwrap();
    assert_eq!(status.code(), Some(42));
}
