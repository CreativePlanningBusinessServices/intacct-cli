use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_exits_zero_and_prints_the_about_line() {
    let mut command = Command::cargo_bin("intacct-cli").unwrap();
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Sage Intacct REST API CLI"));
}

// Windows-only: `directories::ProjectDirs` resolves config paths through Known Folder APIs,
// not the $HOME/$XDG_CONFIG_HOME env vars this test uses to force an empty, hermetic config —
// so on Windows it could pick up a real config file on the CI/dev machine instead. The --help
// smoke test above still runs everywhere.
#[cfg(not(windows))]
#[test]
fn unknown_account_alias_with_empty_config_is_a_usage_error() {
    let home_dir = tempfile::tempdir().unwrap();
    let mut command = Command::cargo_bin("intacct-cli").unwrap();
    command
        .args([
            "object",
            "get",
            "accounts-payable/vendor",
            "1",
            "--account",
            "nonexistent-alias-xyz",
        ])
        .env("HOME", home_dir.path())
        .env("XDG_CONFIG_HOME", home_dir.path())
        .env_remove("INTACCT_ACCOUNT")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(r#""kind":"usage""#));
}
