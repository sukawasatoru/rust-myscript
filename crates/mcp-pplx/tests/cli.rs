use std::process::{Command, Output, Stdio};

fn run(args: &[&str], env_key: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mcp-pplx"))
        .args(args)
        .env("PERPLEXITY_API_KEY", env_key)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn serve_uses_environment_key() {
    let output = run(&["serve"], "invalid key");
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("without whitespace")
    );
}

#[test]
fn serve_option_takes_precedence_over_environment() {
    // EOF ends the MCP handshake without issuing an API request.
    let output = run(&["serve", "--api-key", "test-key"], "invalid key");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = run(&["serve", "--api-key", ""], "test-key");
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("nonempty ASCII")
    );
}

#[test]
fn help_does_not_display_environment_key() {
    let output = run(&["serve", "--help"], "test-secret-never-display");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("PERPLEXITY_API_KEY"));
    assert!(!help.contains("test-secret-never-display"));

    let output = run(&[], "test-secret-never-display");
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("serve"));
}
