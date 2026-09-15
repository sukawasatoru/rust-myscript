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
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const INPUT_STRING: &str = r#"
schema_version = 1
collected_at = "2014-11-28T21:00:09+09:00"

[[readings]]
name = "リビング"
temperature_celsius = 20
humidity_percent = 60
"#;

#[test]
fn input_parse_file_successfully() {
    let output = run_with_file(INPUT_STRING);

    assert!(output.status.success());

    let stdout_string = String::from_utf8(output.stdout).unwrap();
    let expected = r#"リビング:
  temperature: 20
  humidity: 60
"#;
    assert_eq!(stdout_string, expected);
}

#[test]
fn input_parse_stdin_successfully() {
    let mut child = Command::new(bin())
        .args(["-i", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child
        .stdin
        .take()
        .unwrap()
        .write_all(INPUT_STRING.as_bytes())
        .unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());

    let stdout_string = String::from_utf8(output.stdout).unwrap();
    let expected = r#"リビング:
  temperature: 20
  humidity: 60
"#;
    assert_eq!(stdout_string, expected);
}

#[test]
fn input_parse_file_should_skip_duplicated_entry() {
    let input = r#"
schema_version = 1
collected_at = "2014-11-28T21:00:09+09:00"

[[readings]]
name = "リビング"
temperature_celsius = 20
humidity_percent = 60

[[readings]]
name = "リビング"
temperature_celsius = 21
humidity_percent = 61
"#;
    let output = run_with_file(input);

    assert!(output.status.success());

    let stdout_string = String::from_utf8(output.stdout).unwrap();
    let expected = r#"リビング:
  temperature: 20
  humidity: 60
"#;
    assert_eq!(stdout_string, expected);
}

fn bin() -> PathBuf {
    env!("CARGO_BIN_EXE_temperature-sensor").into()
}

fn run_with_file(input: &str) -> std::process::Output {
    let mut file = tempfile::NamedTempFile::with_suffix(".toml").unwrap();
    file.write_all(input.as_bytes()).unwrap();
    Command::new(bin())
        .args(["-i", file.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}
