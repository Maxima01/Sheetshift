use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sheetshift"))
        .arg("--data-dir")
        .arg(data)
        .args(args)
        .output()
        .expect("CLI process must start")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected clean JSON: {error}; stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn batch_failures_exit_nonzero_and_history_persists_between_processes() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("state");
    let input = temp.path().join("accounts.csv");
    let missing = temp.path().join("missing.docx");
    let output_dir = temp.path().join("output");
    std::fs::write(&input, "name,amount\nAda,12\nGrace,34\n").unwrap();

    let rejected = cli(
        &data,
        &[
            "convert",
            input.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!rejected.status.success());
    assert_eq!(json(&rejected)["failed"], 1);
    assert!(
        !output_dir.exists(),
        "unaccepted terms must prevent conversion"
    );

    let accepted = cli(&data, &["terms", "--accept"]);
    assert!(accepted.status.success());

    let converted = cli(
        &data,
        &[
            "convert",
            input.to_str().unwrap(),
            missing.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(
        !converted.status.success(),
        "a mixed batch must exit nonzero"
    );
    let report = json(&converted);
    assert_eq!(report["succeeded"], 1);
    assert_eq!(report["failed"], 1);
    assert_eq!(report["results"][0]["status"], "success");
    let output_path = report["results"][0]["output_path"].as_str().unwrap();
    assert!(Path::new(output_path).is_file());

    let history = cli(&data, &["history", "--json"]);
    assert!(history.status.success());
    let records = json(&history);
    assert_eq!(records.as_array().unwrap().len(), 2);
    assert_eq!(records[0]["status"], "failed");

    assert!(cli(&data, &["settings", "history", "off"]).status.success());
    let unrecorded = cli(
        &data,
        &[
            "convert",
            input.to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(unrecorded.status.success());
    assert_eq!(
        json(&cli(&data, &["history", "--json"]))
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let no_confirmation = cli(&data, &["history", "--clear"]);
    assert!(!no_confirmation.status.success());
    assert_eq!(
        json(&cli(&data, &["history", "--json"]))
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let cleared = cli(&data, &["history", "--clear", "--yes", "--json"]);
    assert!(cleared.status.success());
    assert_eq!(json(&cleared)["cleared"], true);
    assert!(json(&cli(&data, &["history", "--json"]))
        .as_array()
        .unwrap()
        .is_empty());
    assert!(input.exists());
    assert!(
        Path::new(output_path).exists(),
        "clearing history must keep converted files"
    );
}

#[test]
fn reading_terms_and_formats_does_not_create_local_state() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("state");
    let terms = cli(&data, &["terms"]);
    assert!(terms.status.success());
    assert!(String::from_utf8_lossy(&terms.stdout).contains(sheetshift_core::TERMS_VERSION));
    let formats = cli(&data, &["formats", "--json"]);
    assert!(formats.status.success());
    assert!(json(&formats)
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "docx"));
    assert!(!data.exists());
}

#[test]
fn invalid_output_directory_is_reported_as_clean_json() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("state");
    let input = temp.path().join("notes.txt");
    let blocked_output = temp.path().join("not-a-directory");
    std::fs::write(&input, "First line\nSecond line\n").unwrap();
    std::fs::write(&blocked_output, "an existing regular file").unwrap();
    let output = cli(
        &data,
        &[
            "convert",
            input.to_str().unwrap(),
            "--output",
            blocked_output.to_str().unwrap(),
            "--accept-terms",
            "--json",
        ],
    );
    assert!(!output.status.success());
    let report = json(&output);
    assert_eq!(report["failed"], 1);
    assert!(report["results"][0]["error"]
        .as_str()
        .unwrap()
        .contains("output folder"));
}
