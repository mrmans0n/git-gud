use crate::helpers::{create_test_repo, run_gg, run_git};

use std::fs;

#[test]
fn test_gg_help() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["--help"]);

    assert!(success);
    assert!(stdout.contains("stacked-diffs CLI tool"));
    assert!(stdout.contains("co"));
    assert!(stdout.contains("sync"));
    assert!(stdout.contains("ls"));
    assert!(
        stdout.contains("--client-operation-id <ID>"),
        "root help must advertise native-client operation correlation: {stdout}"
    );
}

#[test]
fn test_client_operation_id_help_is_truthful_for_non_recording_commands() {
    let (_temp_dir, repo_path) = create_test_repo();

    for args in [
        vec!["--help"],
        vec!["ls", "--help"],
        vec!["continue", "--help"],
        vec!["abort", "--help"],
    ] {
        let (success, stdout, stderr) = run_gg(&repo_path, &args);
        assert!(success, "help failed for {args:?}: {stderr}");
        assert!(
            stdout.contains("--client-operation-id <ID>"),
            "global option missing from help for {args:?}: {stdout}"
        );
        assert!(
            stdout.contains("if this command") && stdout.contains("creates one"),
            "help must not promise an operation record for {args:?}: {stdout}"
        );
    }
}

#[test]
fn test_client_operation_id_is_preserved_in_mutation_record() {
    let (_temp_dir, repo_path) = create_test_repo();

    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    let client_id = "alas:018f84c0-3a14-7b45-a397-93c87266391d";
    let (success, _stdout, stderr) = run_gg(
        &repo_path,
        &[
            "co",
            "correlated-operation",
            "--client-operation-id",
            client_id,
        ],
    );
    assert!(success, "checkout mutation should succeed: {stderr}");

    let operations_dir = gg_dir.join("operations");
    let record = fs::read_dir(&operations_dir)
        .expect("mutation must create the operation directory")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .find(|record| {
            record["args"]
                .as_array()
                .is_some_and(|args| args.iter().any(|arg| arg == client_id))
        })
        .expect("mutation record must preserve the client operation id in raw args");

    assert_eq!(
        record["args"],
        serde_json::json!([
            "co",
            "correlated-operation",
            "--client-operation-id",
            client_id
        ])
    );
    assert!(
        record["id"].as_str().is_some_and(
            |operation_id| operation_id.starts_with("op_") && operation_id != client_id
        ),
        "the client token must not override GG's operation id: {record:?}"
    );
}

#[test]
fn test_client_operation_id_rejects_unsafe_tokens() {
    let (_temp_dir, repo_path) = create_test_repo();
    let oversized = "a".repeat(129);

    for invalid in ["", "unsafe/value", "contains space", oversized.as_str()] {
        let (success, stdout, stderr) =
            run_gg(&repo_path, &["--client-operation-id", invalid, "ls"]);
        assert!(
            !success,
            "unsafe client operation id must be rejected: {invalid:?}; stdout={stdout} stderr={stderr}"
        );
        assert!(
            stderr.contains("client operation id"),
            "validation error should identify the invalid field: {stderr}"
        );
    }
}

#[test]
fn test_gg_version() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, stdout, _stderr) = run_gg(&repo_path, &["--version"]);

    assert!(success);
    assert!(stdout.contains("gg"));
}

#[test]
fn test_completions() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Test bash completions
    let (success, stdout, _) = run_gg(&repo_path, &["completions", "bash"]);
    assert!(success);
    assert!(stdout.contains("_gg") || stdout.contains("complete"));
    assert!(
        stdout.contains("--client-operation-id"),
        "bash completions must advertise the global native-client flag"
    );

    // Test zsh completions
    let (success, stdout, _) = run_gg(&repo_path, &["completions", "zsh"]);
    assert!(success);
    assert!(stdout.contains("#compdef") || stdout.contains("_gg"));
    assert!(
        stdout.contains("--client-operation-id"),
        "zsh completions must advertise the global native-client flag"
    );

    // Test fish completions
    let (success, stdout, _) = run_gg(&repo_path, &["completions", "fish"]);
    assert!(success);
    assert!(stdout.contains("complete") || stdout.contains("gg"));
    assert!(
        stdout.contains("client-operation-id"),
        "fish completions must advertise the global native-client flag"
    );
}

#[test]
fn test_init_shell_integration() {
    let (_temp_dir, repo_path) = create_test_repo();

    let (success, stdout, stderr) = run_gg(&repo_path, &["init", "bash"]);
    assert!(success, "bash init should succeed: {}", stderr);
    assert!(stdout.contains("gg()"));
    assert!(stdout.contains("GG_CD_FILE"));
    assert!(stdout.contains("command gg"));
    assert!(stdout.contains("cd \"$gg_cd_target\""));

    let (success, stdout, stderr) = run_gg(&repo_path, &["init", "zsh"]);
    assert!(success, "zsh init should succeed: {}", stderr);
    assert!(stdout.contains("gg()"));
    assert!(stdout.contains("GG_CD_FILE"));
    assert!(stdout.contains("command gg"));
    assert!(stdout.contains("cd \"$gg_cd_target\""));

    let (success, stdout, stderr) = run_gg(&repo_path, &["init", "fish"]);
    assert!(success, "fish init should succeed: {}", stderr);
    assert!(stdout.contains("function gg"));
    assert!(stdout.contains("GG_CD_FILE"));
    assert!(stdout.contains("command gg $argv"));
    assert!(stdout.contains("cd \"$gg_cd_target\""));
}

#[test]
fn test_init_rejects_unsupported_shell() {
    let (_temp_dir, repo_path) = create_test_repo();
    let (success, _stdout, stderr) = run_gg(&repo_path, &["init", "powershell"]);

    assert!(!success);
    assert!(stderr.contains("invalid value") || stderr.contains("possible values"));
}

#[test]
fn test_stack_name_sanitization_spaces_to_kebab() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack with spaces in the name - should be converted to hyphens
    let (success, stdout, stderr) = run_gg(&repo_path, &["co", "my feature branch"]);

    assert!(success, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("my-feature-branch") || stdout.contains("Converted"),
        "Should convert spaces to hyphens: stdout={}",
        stdout
    );

    // Verify we're on the kebab-case branch
    let (_, branch) = run_git(&repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(
        branch.trim(),
        "testuser/my-feature-branch",
        "Branch should use kebab-case"
    );
}

#[test]
fn test_stack_name_rejects_slash() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Try to create a stack with slash - should fail
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "feature/subfeature"]);

    assert!(!success, "Should fail with slash in name");
    assert!(
        stderr.contains("cannot contain '/'") || stderr.contains("Invalid stack name"),
        "Should mention invalid character: stderr={}",
        stderr
    );
}

#[test]
fn test_stack_name_rejects_double_dash() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up config
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Try to create a stack with double dash - should fail (conflicts with entry branch format)
    let (success, _stdout, stderr) = run_gg(&repo_path, &["co", "my--feature"]);

    assert!(!success, "Should fail with double dash in name");
    assert!(
        stderr.contains("cannot contain '--'") || stderr.contains("Invalid stack name"),
        "Should mention invalid sequence: stderr={}",
        stderr
    );
}

#[test]
fn test_config_auto_add_gg_ids_default() {
    let (_temp_dir, repo_path) = create_test_repo();

    // Set up minimal config without auto_add_gg_ids
    let gg_dir = repo_path.join(".git/gg");
    fs::create_dir_all(&gg_dir).expect("Failed to create gg dir");
    fs::write(
        gg_dir.join("config.json"),
        r#"{"defaults":{"branch_username":"testuser"}}"#,
    )
    .expect("Failed to write config");

    // Create a stack
    run_gg(&repo_path, &["co", "gg-id-test"]);

    fs::write(repo_path.join("test.txt"), "content").expect("Failed to write file");
    run_git(&repo_path, &["add", "."]);
    run_git(&repo_path, &["commit", "-m", "Test commit without GG-ID"]);

    // Verify the config doesn't have auto_add_gg_ids explicitly set
    // (it should default to true)
    let config_content =
        fs::read_to_string(gg_dir.join("config.json")).expect("Failed to read config");

    // The config should NOT contain auto_add_gg_ids: false
    // (either it's not present, meaning default true, or explicitly true)
    assert!(
        !config_content.contains("\"auto_add_gg_ids\":false")
            && !config_content.contains("\"auto_add_gg_ids\": false"),
        "auto_add_gg_ids should not be explicitly false: {}",
        config_content
    );
}
