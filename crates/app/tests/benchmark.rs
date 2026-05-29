use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn benchmark_flag_prints_elapsed_millis() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("edit-benchmark-{unique}"));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_edit"))
        .arg("--benchmark")
        .arg(&dir)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let last_line = stdout.lines().last().unwrap_or_default().trim();
    assert!(!last_line.is_empty());
    assert!(last_line.parse::<u128>().is_ok(), "stdout was: {stdout}");
}
