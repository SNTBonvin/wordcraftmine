use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli"))
}

#[test]
fn mcp_help_flag_prints_usage_and_options() {
    let out = cli().args(["mcp", "--help"]).output().expect("runs wordcraft-cli");
    assert!(out.status.success(), "status {:?}, stderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("wordcraft-cli mcp — MCP server on stdio"));
    assert!(stdout.contains("USAGE:"));
    assert!(stdout.contains("wordcraft-cli mcp [--connect HOST:PORT | --author NAME] [--automation-read-root DIR] [--automation-write-root DIR]"));
    assert!(stdout.contains("--connect HOST:PORT"));
    assert!(stdout.contains("--author NAME"));
    assert!(stdout.contains("--automation-read-root DIR"));
    assert!(stdout.contains("--automation-write-root DIR"));
    assert!(stdout.contains("-h, --help"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn mcp_short_help_flag_prints_usage_and_options() {
    let out = cli().args(["mcp", "-h"]).output().expect("runs wordcraft-cli");
    assert!(out.status.success(), "status {:?}, stderr: {}", out.status, String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("wordcraft-cli mcp"));
    assert!(stdout.contains("USAGE:"));
    assert!(stdout.contains("OPTIONS:"));
}
