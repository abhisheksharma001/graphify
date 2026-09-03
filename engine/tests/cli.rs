use assert_cmd::Command;

#[test]
fn version_prints_name_and_version() {
    Command::cargo_bin("graphify")
        .unwrap()
        .arg("version")
        .assert()
        .success()
        .stdout("graphify 0.1.0\n");
}

#[test]
fn no_subcommand_is_an_error_with_usage() {
    let output = Command::cargo_bin("graphify").unwrap().assert().failure();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr).to_string();
    assert!(stderr.contains("Usage:"), "stderr was: {stderr}");
}
