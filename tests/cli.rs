use std::process::{Command, Output};

fn presswerk(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_presswerk"))
        .args(args)
        .output()
        .expect("failed to run presswerk")
}

#[test]
fn no_arguments_prints_help() {
    let output = presswerk(&[]);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: presswerk"));
}

#[test]
fn help_flag_prints_help() {
    let output = presswerk(&["--help"]);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: presswerk"));
}

#[test]
fn version_flag_prints_cargo_version() {
    let output = presswerk(&["--version"]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "presswerk 0.0.0\n");
}
