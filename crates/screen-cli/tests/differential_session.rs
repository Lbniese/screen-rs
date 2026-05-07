use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use screen_testkit::{PtySize, PtyTestProcess};

#[test]
fn detached_session_lifecycle_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("ref");
    let candidate_runtime = TempDir::new("cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential lifecycle test: Unix socket bind is not permitted");
        return;
    }

    let reference_result = run_lifecycle(
        Implementation::Reference,
        &reference,
        reference_runtime.path(),
        "diff-ref",
    );
    let candidate_result = run_lifecycle(
        Implementation::Candidate,
        &candidate,
        candidate_runtime.path(),
        "diff-cand",
    );

    let report = LifecycleComparison::new(reference_result, candidate_result);
    if !report.is_match() {
        eprintln!("{report}");
    }

    assert!(
        report.reference.completed,
        "reference lifecycle failed:\n{report}"
    );
    assert!(
        report.candidate.completed,
        "candidate lifecycle failed:\n{report}"
    );
}

#[test]
fn attached_create_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("attached-ref");
    let candidate_runtime = TempDir::new("attached-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential attached create test: Unix socket bind is not permitted");
        return;
    }

    let reference_result = run_attached_create_case(
        Implementation::Reference,
        &reference,
        reference_runtime.path(),
        "attachedcase",
    );
    let candidate_result = run_attached_create_case(
        Implementation::Candidate,
        &candidate,
        candidate_runtime.path(),
        "attachedcase",
    );

    if !reference_result.completed() {
        eprintln!(
            "skipping differential attached create comparison: reference GNU Screen did not complete (likely PTY incompatibility on this platform)"
        );
        eprintln!("reference flags: {:?}", reference_result.flags());
        eprintln!("reference diagnostics: {:?}", reference_result.diagnostics);
    } else if !candidate_result.completed() {
        eprintln!("attached_create differential report");
        eprintln!("reference flags: {:?}", reference_result.flags());
        eprintln!("candidate flags: {:?}", candidate_result.flags());
        eprintln!("reference diagnostics: {:?}", reference_result.diagnostics);
        eprintln!("candidate diagnostics: {:?}", candidate_result.diagnostics);
    }

    assert!(
        candidate_result.completed(),
        "candidate attached create failed"
    );
    if reference_result.completed() {
        assert_eq!(candidate_result.flags(), reference_result.flags());
    }
}

#[test]
fn attach_or_create_create_branch_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("rr-create-ref");
    let candidate_runtime = TempDir::new("rr-create-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!(
            "skipping differential attach-or-create create test: Unix socket bind is not permitted"
        );
        return;
    }

    let reference_result = run_attach_or_create_create_case(
        Implementation::Reference,
        &reference,
        reference_runtime.path(),
        "rrcreatecase",
    );
    let candidate_result = run_attach_or_create_create_case(
        Implementation::Candidate,
        &candidate,
        candidate_runtime.path(),
        "rrcreatecase",
    );

    if !reference_result.completed() {
        eprintln!(
            "skipping differential attach-or-create create comparison: reference GNU Screen did not complete (likely PTY incompatibility on this platform)"
        );
        eprintln!("reference flags: {:?}", reference_result.flags());
        eprintln!("reference diagnostics: {:?}", reference_result.diagnostics);
    } else if !candidate_result.completed() {
        eprintln!("attach_or_create_create differential report");
        eprintln!("reference flags: {:?}", reference_result.flags());
        eprintln!("candidate flags: {:?}", candidate_result.flags());
        eprintln!("reference diagnostics: {:?}", reference_result.diagnostics);
        eprintln!("candidate diagnostics: {:?}", candidate_result.diagnostics);
    }

    assert!(
        candidate_result.completed(),
        "candidate attach-or-create create branch failed"
    );
    if reference_result.completed() {
        assert_eq!(candidate_result.flags(), reference_result.flags());
    }
}

#[test]
fn detached_child_environment_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("env-ref");
    let candidate_runtime = TempDir::new("env-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential environment test: Unix socket bind is not permitted");
        return;
    }

    let reference_result = run_child_environment_case(
        Implementation::Reference,
        &reference,
        reference_runtime.path(),
        "envcase",
        None,
    );
    let candidate_result = run_child_environment_case(
        Implementation::Candidate,
        &candidate,
        candidate_runtime.path(),
        "envcase",
        None,
    );

    let reference_env = reference_result.normalized_environment("envcase");
    let candidate_env = candidate_result.normalized_environment("envcase");
    if reference_env != candidate_env
        || !reference_result.cleanup_success
        || !candidate_result.cleanup_success
    {
        eprintln!("detached_child_environment differential report");
        eprintln!("reference env:\n{reference_env}");
        eprintln!("candidate env:\n{candidate_env}");
        eprintln!("reference diagnostics: {:?}", reference_result.diagnostics);
        eprintln!("candidate diagnostics: {:?}", candidate_result.diagnostics);
    }

    assert!(
        reference_result.cleanup_success,
        "reference child-exit cleanup failed"
    );
    assert!(
        candidate_result.cleanup_success,
        "candidate child-exit cleanup failed"
    );
    assert_eq!(candidate_env, reference_env);
}

#[test]
fn detached_child_terminal_override_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("term-ref");
    let candidate_runtime = TempDir::new("term-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!(
            "skipping differential terminal override test: Unix socket bind is not permitted"
        );
        return;
    }

    let terminal = OsStr::new("screen-256color");
    let reference_result = run_child_environment_case(
        Implementation::Reference,
        &reference,
        reference_runtime.path(),
        "termcase",
        Some(terminal),
    );
    let candidate_result = run_child_environment_case(
        Implementation::Candidate,
        &candidate,
        candidate_runtime.path(),
        "termcase",
        Some(terminal),
    );

    let reference_env = reference_result.normalized_environment("termcase");
    let candidate_env = candidate_result.normalized_environment("termcase");
    if reference_env != candidate_env
        || !reference_result.cleanup_success
        || !candidate_result.cleanup_success
    {
        eprintln!("detached_child_terminal_override differential report");
        eprintln!("reference env:\n{reference_env}");
        eprintln!("candidate env:\n{candidate_env}");
        eprintln!("reference diagnostics: {:?}", reference_result.diagnostics);
        eprintln!("candidate diagnostics: {:?}", candidate_result.diagnostics);
    }

    assert!(
        reference_result.cleanup_success,
        "reference child-exit cleanup failed"
    );
    assert!(
        candidate_result.cleanup_success,
        "candidate child-exit cleanup failed"
    );
    assert_eq!(candidate_env, reference_env);
}

#[test]
fn remote_stuff_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("stuff-ref");
    let candidate_runtime = TempDir::new("stuff-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential stuff test: Unix socket bind is not permitted");
        return;
    }

    let reference_result = run_remote_stuff_case(
        Implementation::Reference,
        &reference,
        reference_runtime.path(),
        "stuffcase",
    );
    let candidate_result = run_remote_stuff_case(
        Implementation::Candidate,
        &candidate,
        candidate_runtime.path(),
        "stuffcase",
    );

    if reference_result.output != candidate_result.output
        || !reference_result.completed
        || !candidate_result.completed
    {
        eprintln!("remote_stuff differential report");
        eprintln!(
            "reference output: {:?}",
            String::from_utf8_lossy(&reference_result.output)
        );
        eprintln!(
            "candidate output: {:?}",
            String::from_utf8_lossy(&candidate_result.output)
        );
        eprintln!("reference diagnostics: {:?}", reference_result.diagnostics);
        eprintln!("candidate diagnostics: {:?}", candidate_result.diagnostics);
    }

    assert!(reference_result.completed, "reference stuff case failed");
    assert!(candidate_result.completed, "candidate stuff case failed");
    assert_eq!(candidate_result.output, reference_result.output);
}

#[test]
fn session_listing_output_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("list-ref");
    let candidate_runtime = TempDir::new("list-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential list test: Unix socket bind is not permitted");
        return;
    }

    let empty_reference = run_screen(
        &reference,
        reference_runtime.path(),
        [OsStr::new("-ls")],
        Duration::from_secs(5),
    )
    .expect("reference empty list should run");
    let empty_candidate = run_screen(
        &candidate,
        candidate_runtime.path(),
        [OsStr::new("-ls")],
        Duration::from_secs(5),
    )
    .expect("candidate empty list should run");
    assert_eq!(
        normalize_list_output(&empty_candidate, candidate_runtime.path(), "listcase"),
        normalize_list_output(&empty_reference, reference_runtime.path(), "listcase")
    );
    let empty_wipe_reference = run_screen(
        &reference,
        reference_runtime.path(),
        [OsStr::new("-wipe")],
        Duration::from_secs(5),
    )
    .expect("reference empty wipe should run");
    let empty_wipe_candidate = run_screen(
        &candidate,
        candidate_runtime.path(),
        [OsStr::new("-wipe")],
        Duration::from_secs(5),
    )
    .expect("candidate empty wipe should run");
    assert_eq!(
        normalize_list_output(&empty_wipe_candidate, candidate_runtime.path(), "listcase"),
        normalize_list_output(&empty_wipe_reference, reference_runtime.path(), "listcase")
    );

    start_detached_loop(&reference, reference_runtime.path(), "listcase")
        .expect("reference list session should start");
    start_detached_loop(&candidate, candidate_runtime.path(), "listcase")
        .expect("candidate list session should start");

    let listed_reference = run_screen(
        &reference,
        reference_runtime.path(),
        [OsStr::new("-ls")],
        Duration::from_secs(5),
    )
    .expect("reference populated list should run");
    let listed_candidate = run_screen(
        &candidate,
        candidate_runtime.path(),
        [OsStr::new("-ls")],
        Duration::from_secs(5),
    )
    .expect("candidate populated list should run");
    if normalize_list_output(&listed_candidate, candidate_runtime.path(), "listcase")
        != normalize_list_output(&listed_reference, reference_runtime.path(), "listcase")
    {
        eprintln!("session_listing_output differential report");
        eprintln!(
            "reference:\n{}",
            format_output("reference_list", &listed_reference)
        );
        eprintln!(
            "candidate:\n{}",
            format_output("candidate_list", &listed_candidate)
        );
    }
    assert_eq!(
        normalize_list_output(&listed_candidate, candidate_runtime.path(), "listcase"),
        normalize_list_output(&listed_reference, reference_runtime.path(), "listcase")
    );
    let active_wipe_reference = run_screen(
        &reference,
        reference_runtime.path(),
        [OsStr::new("-wipe")],
        Duration::from_secs(5),
    )
    .expect("reference active wipe should run");
    let active_wipe_candidate = run_screen(
        &candidate,
        candidate_runtime.path(),
        [OsStr::new("-wipe")],
        Duration::from_secs(5),
    )
    .expect("candidate active wipe should run");
    assert_eq!(
        normalize_list_output(&active_wipe_candidate, candidate_runtime.path(), "listcase"),
        normalize_list_output(&active_wipe_reference, reference_runtime.path(), "listcase")
    );

    let _ = quit_session(
        Implementation::Reference,
        &reference,
        reference_runtime.path(),
        "listcase",
    );
    let _ = quit_session(
        Implementation::Candidate,
        &candidate,
        candidate_runtime.path(),
        "listcase",
    );
    assert!(
        wait_until_no_session(
            Implementation::Reference,
            &reference,
            reference_runtime.path(),
            "listcase",
        )
        .success
    );
    assert!(
        wait_until_no_session(
            Implementation::Candidate,
            &candidate,
            candidate_runtime.path(),
            "listcase",
        )
        .success
    );
}

#[test]
fn session_listing_filter_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("list-filter-ref");
    let candidate_runtime = TempDir::new("list-filter-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential list filter test: Unix socket bind is not permitted");
        return;
    }

    start_detached_loop(&reference, reference_runtime.path(), "filtercase-a")
        .expect("reference filtered session A should start");
    start_detached_loop(&reference, reference_runtime.path(), "filtercase-b")
        .expect("reference filtered session B should start");
    start_detached_loop(&candidate, candidate_runtime.path(), "filtercase-a")
        .expect("candidate filtered session A should start");
    start_detached_loop(&candidate, candidate_runtime.path(), "filtercase-b")
        .expect("candidate filtered session B should start");

    let listed_reference = run_screen(
        &reference,
        reference_runtime.path(),
        [OsStr::new("-ls"), OsStr::new("filtercase-a")],
        Duration::from_secs(5),
    )
    .expect("reference filtered list should run");
    let listed_candidate = run_screen(
        &candidate,
        candidate_runtime.path(),
        [OsStr::new("-ls"), OsStr::new("filtercase-a")],
        Duration::from_secs(5),
    )
    .expect("candidate filtered list should run");
    if normalize_list_output(&listed_candidate, candidate_runtime.path(), "filtercase-a")
        != normalize_list_output(&listed_reference, reference_runtime.path(), "filtercase-a")
    {
        eprintln!("session_listing_filter differential report");
        eprintln!(
            "reference:\n{}",
            format_output("reference_filtered_list", &listed_reference)
        );
        eprintln!(
            "candidate:\n{}",
            format_output("candidate_filtered_list", &listed_candidate)
        );
    }
    assert_eq!(
        normalize_list_output(&listed_candidate, candidate_runtime.path(), "filtercase-a"),
        normalize_list_output(&listed_reference, reference_runtime.path(), "filtercase-a")
    );

    for session_name in ["filtercase-a", "filtercase-b"] {
        let _ = quit_session(
            Implementation::Reference,
            &reference,
            reference_runtime.path(),
            session_name,
        );
        let _ = quit_session(
            Implementation::Candidate,
            &candidate,
            candidate_runtime.path(),
            session_name,
        );
    }
}

#[test]
fn shell_option_uses_custom_shell_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("shell-ref");
    let candidate_runtime = TempDir::new("shell-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential shell option test: Unix socket bind is not permitted");
        return;
    }

    let reference_result = run_shell_option_case(&reference, reference_runtime.path(), "shellcase")
        .expect("reference shell option case should run");
    let candidate_result = run_shell_option_case(&candidate, candidate_runtime.path(), "shellcase")
        .expect("candidate shell option case should run");

    if reference_result != candidate_result {
        eprintln!("shell_option differential report");
        eprintln!(
            "reference output: {:?}",
            String::from_utf8_lossy(&reference_result)
        );
        eprintln!(
            "candidate output: {:?}",
            String::from_utf8_lossy(&candidate_result)
        );
    }

    assert_eq!(candidate_result, reference_result);
}

#[test]
fn compact_detached_create_options_compare_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("compact-ref");
    let candidate_runtime = TempDir::new("compact-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential compact option test: Unix socket bind is not permitted");
        return;
    }

    let reference_result =
        run_compact_detached_create_case(&reference, reference_runtime.path(), "compactcase")
            .expect("reference compact detached create case should run");
    let candidate_result =
        run_compact_detached_create_case(&candidate, candidate_runtime.path(), "compactcase")
            .expect("candidate compact detached create case should run");

    if reference_result != candidate_result {
        eprintln!("compact_detached_create differential report");
        eprintln!(
            "reference output: {:?}",
            String::from_utf8_lossy(&reference_result)
        );
        eprintln!(
            "candidate output: {:?}",
            String::from_utf8_lossy(&candidate_result)
        );
    }

    assert_eq!(candidate_result, reference_result);
}

#[test]
fn config_file_shell_and_term_compare_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("config-ref");
    let candidate_runtime = TempDir::new("config-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential config file test: Unix socket bind is not permitted");
        return;
    }

    let reference_result =
        run_config_shell_term_case(&reference, reference_runtime.path(), "configcase")
            .expect("reference config file case should run");
    let candidate_result =
        run_config_shell_term_case(&candidate, candidate_runtime.path(), "configcase")
            .expect("candidate config file case should run");

    if reference_result != candidate_result {
        eprintln!("config_file differential report");
        eprintln!(
            "reference output: {:?}",
            String::from_utf8_lossy(&reference_result)
        );
        eprintln!(
            "candidate output: {:?}",
            String::from_utf8_lossy(&candidate_result)
        );
    }

    assert_eq!(candidate_result, reference_result);
}

#[test]
fn config_file_source_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("source-ref");
    let candidate_runtime = TempDir::new("source-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential config source test: Unix socket bind is not permitted");
        return;
    }

    let reference_result =
        run_config_source_case(&reference, reference_runtime.path(), "sourcecase")
            .expect("reference config source case should run");
    let candidate_result =
        run_config_source_case(&candidate, candidate_runtime.path(), "sourcecase")
            .expect("candidate config source case should run");

    if reference_result != candidate_result {
        eprintln!("config_source differential report");
        eprintln!(
            "reference output: {:?}",
            String::from_utf8_lossy(&reference_result)
        );
        eprintln!(
            "candidate output: {:?}",
            String::from_utf8_lossy(&candidate_result)
        );
    }

    assert_eq!(candidate_result, reference_result);
}

#[test]
fn home_screenrc_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("home-rc-ref");
    let candidate_runtime = TempDir::new("home-rc-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential home .screenrc test: Unix socket bind is not permitted");
        return;
    }

    let reference_result =
        run_home_screenrc_case(&reference, reference_runtime.path(), "homescreenrccase")
            .expect("reference home .screenrc case should run");
    let candidate_result =
        run_home_screenrc_case(&candidate, candidate_runtime.path(), "homescreenrccase")
            .expect("candidate home .screenrc case should run");

    if reference_result != candidate_result {
        eprintln!("home_screenrc differential report");
        eprintln!(
            "reference output: {:?}",
            String::from_utf8_lossy(&reference_result)
        );
        eprintln!(
            "candidate output: {:?}",
            String::from_utf8_lossy(&candidate_result)
        );
    }

    assert_eq!(candidate_result, reference_result);
}

#[test]
fn config_file_chdir_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("chdir-ref");
    let candidate_runtime = TempDir::new("chdir-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential chdir config test: Unix socket bind is not permitted");
        return;
    }

    let reference_result = run_config_chdir_case(&reference, reference_runtime.path(), "chdircase")
        .expect("reference chdir config case should run");
    let candidate_result = run_config_chdir_case(&candidate, candidate_runtime.path(), "chdircase")
        .expect("candidate chdir config case should run");
    let reference_normalized = normalize_runtime_path(&reference_result, reference_runtime.path());
    let candidate_normalized = normalize_runtime_path(&candidate_result, candidate_runtime.path());

    if reference_normalized != candidate_normalized {
        eprintln!("config_chdir differential report");
        eprintln!(
            "reference output: {:?}",
            String::from_utf8_lossy(&reference_result)
        );
        eprintln!(
            "candidate output: {:?}",
            String::from_utf8_lossy(&candidate_result)
        );
    }

    assert_eq!(candidate_normalized, reference_normalized);
}

#[test]
fn screenrc_env_logging_commands_compare_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("screenrc-log-ref");
    let candidate_runtime = TempDir::new("screenrc-log-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential SCREENRC logging test: Unix socket bind is not permitted");
        return;
    }

    let reference_log =
        run_screenrc_logging_case(&reference, reference_runtime.path(), "screenrclogcase")
            .expect("reference SCREENRC logging case should run");
    let candidate_log =
        run_screenrc_logging_case(&candidate, candidate_runtime.path(), "screenrclogcase")
            .expect("candidate SCREENRC logging case should run");

    if candidate_log != reference_log {
        eprintln!("SCREENRC logging differential report");
        eprintln!("reference bytes: {reference_log:?}");
        eprintln!("candidate bytes: {candidate_log:?}");
    }

    assert_eq!(candidate_log, reference_log);
}

#[test]
fn logging_option_compares_with_gnu_screen() {
    let reference = std::env::var_os("SCREEN_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screen"));
    let candidate = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));

    let reference_runtime = TempDir::new("log-ref");
    let candidate_runtime = TempDir::new("log-cand");
    if !unix_socket_bind_allowed(candidate_runtime.path()) {
        eprintln!("skipping differential logging test: Unix socket bind is not permitted");
        return;
    }

    let reference_log = run_logging_case(&reference, reference_runtime.path(), "logcase")
        .expect("reference logging case should run");
    let candidate_log = run_logging_case(&candidate, candidate_runtime.path(), "logcase")
        .expect("candidate logging case should run");

    if candidate_log != reference_log {
        eprintln!("logging differential report");
        eprintln!("reference bytes: {reference_log:?}");
        eprintln!("candidate bytes: {candidate_log:?}");
    }

    assert_eq!(candidate_log, reference_log);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Implementation {
    Reference,
    Candidate,
}

#[derive(Debug)]
struct LifecycleResult {
    completed: bool,
    start_success: bool,
    list_after_start_has_session: bool,
    attach_output_has_ready: bool,
    detach_status_success: bool,
    list_after_detach_has_session: bool,
    quit_success: bool,
    cleanup_success: bool,
    diagnostics: Vec<String>,
}

impl LifecycleResult {
    fn new(_implementation: Implementation) -> Self {
        Self {
            completed: false,
            start_success: false,
            list_after_start_has_session: false,
            attach_output_has_ready: false,
            detach_status_success: false,
            list_after_detach_has_session: false,
            quit_success: false,
            cleanup_success: false,
            diagnostics: Vec::new(),
        }
    }

    fn flags(&self) -> [bool; 7] {
        [
            self.start_success,
            self.list_after_start_has_session,
            self.attach_output_has_ready,
            self.detach_status_success,
            self.list_after_detach_has_session,
            self.quit_success,
            self.cleanup_success,
        ]
    }
}

struct LifecycleComparison {
    reference: LifecycleResult,
    candidate: LifecycleResult,
}

impl LifecycleComparison {
    fn new(reference: LifecycleResult, candidate: LifecycleResult) -> Self {
        Self {
            reference,
            candidate,
        }
    }

    fn is_match(&self) -> bool {
        self.reference.flags() == self.candidate.flags()
    }
}

impl std::fmt::Display for LifecycleComparison {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(formatter, "detached_session_lifecycle differential report")?;
        writeln!(formatter, "reference: {:?}", self.reference.flags())?;
        writeln!(formatter, "candidate: {:?}", self.candidate.flags())?;
        if !self.reference.diagnostics.is_empty() {
            writeln!(formatter, "reference diagnostics:")?;
            for diagnostic in &self.reference.diagnostics {
                writeln!(formatter, "- {diagnostic}")?;
            }
        }
        if !self.candidate.diagnostics.is_empty() {
            writeln!(formatter, "candidate diagnostics:")?;
            for diagnostic in &self.candidate.diagnostics {
                writeln!(formatter, "- {diagnostic}")?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct ChildEnvironmentResult {
    environment: Vec<u8>,
    cleanup_success: bool,
    diagnostics: Vec<String>,
}

impl ChildEnvironmentResult {
    fn normalized_environment(&self, session_name: &str) -> String {
        normalize_child_environment(&self.environment, session_name)
    }
}

#[derive(Debug)]
struct RemoteStuffResult {
    completed: bool,
    output: Vec<u8>,
    diagnostics: Vec<String>,
}

#[derive(Debug)]
struct AttachedCreateCaseResult {
    output_has_ready: bool,
    status_success: bool,
    cleanup_success: bool,
    diagnostics: Vec<String>,
}

impl AttachedCreateCaseResult {
    fn new() -> Self {
        Self {
            output_has_ready: false,
            status_success: false,
            cleanup_success: false,
            diagnostics: Vec::new(),
        }
    }

    fn flags(&self) -> [bool; 3] {
        [
            self.output_has_ready,
            self.status_success,
            self.cleanup_success,
        ]
    }

    fn completed(&self) -> bool {
        self.flags().into_iter().all(|flag| flag)
    }
}

fn run_attached_create_case(
    implementation: Implementation,
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> AttachedCreateCaseResult {
    let mut result = AttachedCreateCaseResult::new();
    let screenrc_path = runtime.join("attached-screenrc");
    if let Err(error) = fs::write(&screenrc_path, "startup_message off\n") {
        result
            .diagnostics
            .push(format!("failed to write screenrc: {error}"));
        return result;
    }
    let envs = [
        (OsString::from("SCREENDIR"), runtime.as_os_str().to_owned()),
        (
            OsString::from("SCREENRC"),
            screenrc_path.as_os_str().to_owned(),
        ),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];
    let mut process = match PtyTestProcess::spawn_with_env(
        executable,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf attached-ready; sleep 1"),
        ],
        envs,
        PtySize::new(80, 24),
    ) {
        Ok(process) => process,
        Err(error) => {
            result
                .diagnostics
                .push(format!("attached create spawn failed: {error}"));
            return result;
        }
    };

    match process.read_until(b"attached-ready", Duration::from_secs(5)) {
        Ok(output) => result.output_has_ready = contains(&output, b"attached-ready"),
        Err(error) => result
            .diagnostics
            .push(format!("attached create did not produce ready: {error}")),
    }

    match process.wait_or_kill(Duration::from_secs(5)) {
        Ok(status) => result.status_success = status.success(),
        Err(error) => result
            .diagnostics
            .push(format!("attached create did not exit cleanly: {error}")),
    }

    let cleanup = wait_until_no_session(implementation, executable, runtime, session_name);
    result.cleanup_success = cleanup.success;
    if !cleanup.success {
        result.diagnostics.push(cleanup.diagnostic);
    }
    result
}

fn run_attach_or_create_create_case(
    implementation: Implementation,
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> AttachedCreateCaseResult {
    let mut result = AttachedCreateCaseResult::new();
    let screenrc_path = runtime.join("rr-screenrc");
    let shell_path = runtime.join("rr-shell");
    if let Err(error) = fs::write(&screenrc_path, "startup_message off\n") {
        result
            .diagnostics
            .push(format!("failed to write screenrc: {error}"));
        return result;
    }
    if let Err(error) = fs::write(&shell_path, "#!/bin/sh\nprintf rr-ready; sleep 1\n") {
        result
            .diagnostics
            .push(format!("failed to write shell: {error}"));
        return result;
    }
    if let Err(error) = fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o700)) {
        result
            .diagnostics
            .push(format!("failed to chmod shell: {error}"));
        return result;
    }
    let envs = [
        (OsString::from("SCREENDIR"), runtime.as_os_str().to_owned()),
        (
            OsString::from("SCREENRC"),
            screenrc_path.as_os_str().to_owned(),
        ),
        (OsString::from("SHELL"), shell_path.as_os_str().to_owned()),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];
    let mut process = match PtyTestProcess::spawn_with_env(
        executable,
        [OsStr::new("-R"), OsStr::new(session_name)],
        envs,
        PtySize::new(80, 24),
    ) {
        Ok(process) => process,
        Err(error) => {
            result
                .diagnostics
                .push(format!("attach-or-create spawn failed: {error}"));
            return result;
        }
    };

    match process.read_until(b"rr-ready", Duration::from_secs(5)) {
        Ok(output) => result.output_has_ready = contains(&output, b"rr-ready"),
        Err(error) => result
            .diagnostics
            .push(format!("attach-or-create did not produce ready: {error}")),
    }

    match process.wait_or_kill(Duration::from_secs(5)) {
        Ok(status) => result.status_success = status.success(),
        Err(error) => result
            .diagnostics
            .push(format!("attach-or-create did not exit cleanly: {error}")),
    }

    let cleanup = wait_until_no_session(implementation, executable, runtime, session_name);
    result.cleanup_success = cleanup.success;
    if !cleanup.success {
        result.diagnostics.push(cleanup.diagnostic);
    }
    result
}

fn run_remote_stuff_case(
    implementation: Implementation,
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> RemoteStuffResult {
    let mut result = RemoteStuffResult {
        completed: false,
        output: Vec::new(),
        diagnostics: Vec::new(),
    };
    let output_path = runtime.join("stuff.out");
    let ready_path = runtime.join("stuff.ready");
    let script = OsStr::new(
        ": > \"$2\"; IFS= read -r line; printf '%s\n' \"$line\" > \"$1\"; while :; do sleep 1; done",
    );
    let args = [
        OsStr::new("-S"),
        OsStr::new(session_name),
        OsStr::new("-d"),
        OsStr::new("-m"),
        OsStr::new("sh"),
        OsStr::new("-c"),
        script,
        OsStr::new("sh"),
        output_path.as_os_str(),
        ready_path.as_os_str(),
    ];

    match run_screen_null(executable, runtime, args, Duration::from_secs(5)) {
        Ok(status) if status.success() => {}
        Ok(status) => {
            result.diagnostics.push(format!("start: status {status}"));
            return result;
        }
        Err(error) => {
            result
                .diagnostics
                .push(format!("start command failed to spawn: {error}"));
            return result;
        }
    }

    if let Err(error) = wait_for_file(&ready_path, Duration::from_secs(5)) {
        result
            .diagnostics
            .push(format!("stuff child was not ready: {error}"));
        let _ = quit_session(implementation, executable, runtime, session_name);
        return result;
    }

    let stuff = run_screen(
        executable,
        runtime,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-p"),
            OsStr::new("0"),
            OsStr::new("-X"),
            OsStr::new("stuff"),
            OsStr::new("hello from stuff\r"),
        ],
        Duration::from_secs(5),
    );
    match stuff {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            result.diagnostics.push(format_output("stuff", &output));
            let _ = quit_session(implementation, executable, runtime, session_name);
            return result;
        }
        Err(error) => {
            result
                .diagnostics
                .push(format!("stuff command failed to spawn: {error}"));
            let _ = quit_session(implementation, executable, runtime, session_name);
            return result;
        }
    }

    match wait_for_file(&output_path, Duration::from_secs(5)) {
        Ok(output) => result.output = output,
        Err(error) => result
            .diagnostics
            .push(format!("stuff output file was not produced: {error}")),
    }

    let quit = quit_session(implementation, executable, runtime, session_name);
    match quit {
        Ok(output) if output.status.success() => {}
        Ok(output) => result.diagnostics.push(format_output("quit", &output)),
        Err(error) => result
            .diagnostics
            .push(format!("quit command failed to spawn: {error}")),
    }
    let cleanup = wait_until_no_session(implementation, executable, runtime, session_name);
    if !cleanup.success {
        result.diagnostics.push(cleanup.diagnostic);
    }
    result.completed = !result.output.is_empty() && cleanup.success;
    result
}

fn start_detached_loop(executable: &Path, runtime: &Path, session_name: &str) -> io::Result<()> {
    let status = run_screen_null(
        executable,
        runtime,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("while :; do sleep 1; done"),
        ],
        Duration::from_secs(5),
    )?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "detached start exited with {status}"
        )))
    }
}

fn run_shell_option_case(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> io::Result<Vec<u8>> {
    let shell_path = runtime.join("custom-shell");
    let output_path = runtime.join("custom-shell.out");
    let script = format!(
        "#!/bin/sh\nprintf 'custom-shell\\n' > '{}'\n",
        output_path.display()
    );
    fs::write(&shell_path, script)?;
    fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o700))?;

    let status = run_screen_null(
        executable,
        runtime,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-s"),
            shell_path.as_os_str(),
            OsStr::new("-d"),
            OsStr::new("-m"),
        ],
        Duration::from_secs(5),
    )?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "screen -s start exited with {status}"
        )));
    }

    let output = wait_for_file(&output_path, Duration::from_secs(5))?;
    let cleanup =
        wait_until_no_session(Implementation::Reference, executable, runtime, session_name);
    if !cleanup.success {
        return Err(io::Error::other(cleanup.diagnostic));
    }
    Ok(output)
}

fn run_compact_detached_create_case(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> io::Result<Vec<u8>> {
    let output_path = runtime.join("compact.out");

    let status = run_screen_null(
        executable,
        runtime,
        [
            OsStr::new("-dmS"),
            OsStr::new(session_name),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf 'compact-detached\n' > \"$1\""),
            OsStr::new("sh"),
            output_path.as_os_str(),
        ],
        Duration::from_secs(5),
    )?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "screen -dmS start exited with {status}"
        )));
    }

    let output = wait_for_file(&output_path, Duration::from_secs(5))?;
    let cleanup =
        wait_until_no_session(Implementation::Reference, executable, runtime, session_name);
    if !cleanup.success {
        return Err(io::Error::other(cleanup.diagnostic));
    }
    Ok(output)
}

fn run_config_shell_term_case(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> io::Result<Vec<u8>> {
    let shell_path = runtime.join("config-shell");
    let output_path = runtime.join("config-shell.out");
    let config_path = runtime.join("screenrc");
    let script = format!(
        "#!/bin/sh\nprintf 'TERM=%s\\n' \"$TERM\" > '{}'\n",
        output_path.display()
    );
    fs::write(&shell_path, script)?;
    fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o700))?;
    fs::write(
        &config_path,
        format!(
            "shell {}\nterm screen-256color\n",
            shell_path.to_string_lossy()
        ),
    )?;

    let status = run_screen_null(
        executable,
        runtime,
        [
            OsStr::new("-c"),
            config_path.as_os_str(),
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
        ],
        Duration::from_secs(5),
    )?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "screen -c start exited with {status}"
        )));
    }

    let output = wait_for_file(&output_path, Duration::from_secs(5))?;
    let cleanup =
        wait_until_no_session(Implementation::Reference, executable, runtime, session_name);
    if !cleanup.success {
        return Err(io::Error::other(cleanup.diagnostic));
    }
    Ok(output)
}

fn run_config_source_case(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> io::Result<Vec<u8>> {
    let shell_path = runtime.join("source-shell");
    let output_path = runtime.join("source-shell.out");
    let config_path = runtime.join("screenrc-source");
    let included_path = runtime.join("included-screenrc");
    let script = format!(
        "#!/bin/sh\nprintf 'TERM=%s\\n' \"$TERM\" > '{}'\n",
        output_path.display()
    );
    fs::write(&shell_path, script)?;
    fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o700))?;
    fs::write(
        &included_path,
        format!(
            "shell {}\nterm screen-256color\n",
            shell_path.to_string_lossy()
        ),
    )?;
    fs::write(
        &config_path,
        format!("source {}\n", included_path.to_string_lossy()),
    )?;

    let status = run_screen_null(
        executable,
        runtime,
        [
            OsStr::new("-c"),
            config_path.as_os_str(),
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
        ],
        Duration::from_secs(5),
    )?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "screen -c source start exited with {status}"
        )));
    }

    let output = wait_for_file(&output_path, Duration::from_secs(5))?;
    let cleanup =
        wait_until_no_session(Implementation::Reference, executable, runtime, session_name);
    if !cleanup.success {
        return Err(io::Error::other(cleanup.diagnostic));
    }
    Ok(output)
}

fn run_home_screenrc_case(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> io::Result<Vec<u8>> {
    let home_dir = runtime.join("home");
    fs::create_dir(&home_dir)?;
    let shell_path = runtime.join("home-screenrc-shell");
    let output_path = runtime.join("home-screenrc-shell.out");
    let screenrc_path = home_dir.join(".screenrc");
    let script = format!(
        "#!/bin/sh\nprintf 'TERM=%s\\n' \"$TERM\" > '{}'\n",
        output_path.display()
    );
    fs::write(&shell_path, script)?;
    fs::set_permissions(&shell_path, fs::Permissions::from_mode(0o700))?;
    fs::write(
        &screenrc_path,
        format!(
            "shell {}\nterm screen-256color\n",
            shell_path.to_string_lossy()
        ),
    )?;

    let status = run_screen_null_with_home(
        executable,
        runtime,
        &home_dir,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
        ],
        Duration::from_secs(5),
    )?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "home .screenrc start exited with {status}"
        )));
    }

    let output = wait_for_file(&output_path, Duration::from_secs(5))?;
    let cleanup =
        wait_until_no_session(Implementation::Reference, executable, runtime, session_name);
    if !cleanup.success {
        return Err(io::Error::other(cleanup.diagnostic));
    }
    Ok(output)
}

fn run_config_chdir_case(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> io::Result<Vec<u8>> {
    let work_dir = runtime.join("work");
    fs::create_dir(&work_dir)?;
    let output_path = runtime.join("pwd.out");
    let config_path = runtime.join("screenrc-chdir");
    fs::write(
        &config_path,
        format!("chdir {}\n", work_dir.to_string_lossy()),
    )?;

    let status = run_screen_null(
        executable,
        runtime,
        [
            OsStr::new("-c"),
            config_path.as_os_str(),
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("pwd > \"$1\""),
            OsStr::new("sh"),
            output_path.as_os_str(),
        ],
        Duration::from_secs(5),
    )?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "screen -c chdir start exited with {status}"
        )));
    }

    let output = wait_for_file(&output_path, Duration::from_secs(5))?;
    let cleanup =
        wait_until_no_session(Implementation::Reference, executable, runtime, session_name);
    if !cleanup.success {
        return Err(io::Error::other(cleanup.diagnostic));
    }
    Ok(output)
}

fn run_screenrc_logging_case(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> io::Result<Vec<u8>> {
    let log_path = runtime.join("configured-screenlog.out");
    let config_path = runtime.join("env-screenrc");
    let _ = fs::remove_file(&log_path);
    fs::write(
        &config_path,
        format!(
            "startup_message off\nlogfile {}\ndeflog on\n",
            log_path.to_string_lossy()
        ),
    )?;

    let status = run_screen_null_with_screenrc(
        executable,
        runtime,
        &config_path,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf 'screenrc-logline\n'; sleep 1"),
        ],
        Duration::from_secs(5),
    )?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "SCREENRC logging start exited with {status}"
        )));
    }

    let output = wait_for_nonempty_file(&log_path, Duration::from_secs(5))?;
    let cleanup =
        wait_until_no_session(Implementation::Reference, executable, runtime, session_name);
    if !cleanup.success {
        return Err(io::Error::other(cleanup.diagnostic));
    }
    Ok(output)
}

fn run_logging_case(executable: &Path, runtime: &Path, session_name: &str) -> io::Result<Vec<u8>> {
    let log_path = runtime.join("screenlog.0");
    let _ = fs::remove_file(&log_path);

    let status = run_screen_null_in_dir(
        executable,
        runtime,
        runtime,
        [
            OsStr::new("-L"),
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf 'logline\n'; sleep 1"),
        ],
        Duration::from_secs(5),
    )?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "screen -L start exited with {status}"
        )));
    }

    let output = wait_for_nonempty_file(&log_path, Duration::from_secs(5))?;
    let cleanup =
        wait_until_no_session(Implementation::Reference, executable, runtime, session_name);
    if !cleanup.success {
        return Err(io::Error::other(cleanup.diagnostic));
    }
    Ok(output)
}

fn run_child_environment_case(
    implementation: Implementation,
    executable: &Path,
    runtime: &Path,
    session_name: &str,
    terminal: Option<&OsStr>,
) -> ChildEnvironmentResult {
    let mut result = ChildEnvironmentResult {
        environment: Vec::new(),
        cleanup_success: false,
        diagnostics: Vec::new(),
    };
    let output_path = runtime.join("child-env.out");
    let script =
        OsStr::new("printf 'STY=%s\nWINDOW=%s\nTERM=%s\n' \"$STY\" \"$WINDOW\" \"$TERM\" > \"$1\"");
    let output_path_arg = output_path.as_os_str();
    let mut args = vec![OsStr::new("-S"), OsStr::new(session_name)];
    if let Some(terminal) = terminal {
        args.extend([OsStr::new("-T"), terminal]);
    }
    args.extend([
        OsStr::new("-d"),
        OsStr::new("-m"),
        OsStr::new("sh"),
        OsStr::new("-c"),
        script,
        OsStr::new("sh"),
        output_path_arg,
    ]);

    let start = run_screen_null(executable, runtime, args, Duration::from_secs(5));
    match start {
        Ok(status) if status.success() => {}
        Ok(status) => {
            result.diagnostics.push(format!("start: status {status}"));
            return result;
        }
        Err(error) => {
            result
                .diagnostics
                .push(format!("start command failed to spawn: {error}"));
            return result;
        }
    }

    match wait_for_file(&output_path, Duration::from_secs(5)) {
        Ok(environment) => result.environment = environment,
        Err(error) => {
            result
                .diagnostics
                .push(format!("environment file was not produced: {error}"));
        }
    }

    let cleanup = wait_until_no_session(implementation, executable, runtime, session_name);
    result.cleanup_success = cleanup.success;
    if !cleanup.success {
        result.diagnostics.push(cleanup.diagnostic);
    }
    result
}

fn run_lifecycle(
    implementation: Implementation,
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> LifecycleResult {
    eprintln!("{implementation:?}: starting lifecycle for {session_name}");
    let mut result = LifecycleResult::new(implementation);
    let _guard = SessionGuard {
        implementation,
        executable: executable.to_owned(),
        runtime: runtime.to_owned(),
        session_name: session_name.to_owned(),
    };

    eprintln!("{implementation:?}: start detached session");
    let start = run_screen_null(
        executable,
        runtime,
        [
            OsStr::new("-S"),
            OsStr::new(session_name),
            OsStr::new("-d"),
            OsStr::new("-m"),
            OsStr::new("sh"),
            OsStr::new("-c"),
            OsStr::new("printf ready; while :; do sleep 1; done"),
        ],
        Duration::from_secs(5),
    );
    let Ok(start) = start else {
        result
            .diagnostics
            .push("start command failed to spawn".to_owned());
        return result;
    };
    result.start_success = start.success();
    if !result.start_success {
        result.diagnostics.push(format!("start: status {start}"));
        return result;
    }

    eprintln!("{implementation:?}: list after start");
    let list = run_screen(
        executable,
        runtime,
        [OsStr::new("-ls")],
        Duration::from_secs(5),
    );
    if let Ok(list) = list {
        result.list_after_start_has_session = output_lists_session(&list, session_name);
        if !result.list_after_start_has_session {
            result
                .diagnostics
                .push(format_output("list_after_start", &list));
        }
    }

    eprintln!("{implementation:?}: attach and detach");
    let attach = attach_and_detach(executable, runtime, session_name, implementation);
    match attach {
        Ok(attach) => {
            result.attach_output_has_ready = attach.output_has_ready;
            result.detach_status_success = attach.status_success;
            if !result.attach_output_has_ready || !result.detach_status_success {
                result.diagnostics.push(attach.diagnostic);
            }
        }
        Err(error) => result.diagnostics.push(error),
    }

    eprintln!("{implementation:?}: list after detach");
    let list_after_detach = run_screen(
        executable,
        runtime,
        [OsStr::new("-ls")],
        Duration::from_secs(5),
    );
    if let Ok(list_after_detach) = list_after_detach {
        result.list_after_detach_has_session =
            output_lists_session(&list_after_detach, session_name);
        if !result.list_after_detach_has_session {
            result
                .diagnostics
                .push(format_output("list_after_detach", &list_after_detach));
        }
    }

    eprintln!("{implementation:?}: quit session");
    let quit = quit_session(implementation, executable, runtime, session_name);
    if let Ok(quit) = quit {
        result.quit_success = quit.status.success();
        if !result.quit_success {
            result.diagnostics.push(format_output("quit", &quit));
        }
    }

    eprintln!("{implementation:?}: wait for cleanup");
    let cleanup = wait_until_no_session(implementation, executable, runtime, session_name);
    result.cleanup_success = cleanup.success;
    if !cleanup.success {
        result.diagnostics.push(cleanup.diagnostic);
    }
    result.completed = result.flags().into_iter().all(|flag| flag);
    result
}

struct AttachResult {
    output_has_ready: bool,
    status_success: bool,
    diagnostic: String,
}

fn attach_and_detach(
    executable: &Path,
    runtime: &Path,
    session_name: &str,
    implementation: Implementation,
) -> Result<AttachResult, String> {
    eprintln!("{implementation:?}: spawn attach process");
    let args = [OsStr::new("-r"), OsStr::new(session_name)];
    let envs = [
        (OsString::from("SCREENDIR"), runtime.as_os_str().to_owned()),
        (OsString::from("SCREENRC"), OsString::from("/dev/null")),
        (OsString::from("TERM"), OsString::from("xterm-256color")),
        (OsString::from("LC_ALL"), OsString::from("C")),
    ];
    let mut process = PtyTestProcess::spawn_with_env(executable, args, envs, PtySize::new(80, 24))
        .map_err(|error| format!("attach spawn failed: {error}"))?;
    eprintln!("{implementation:?}: read attach output");
    let output = process
        .read_until(b"ready", Duration::from_secs(5))
        .map_err(|error| format!("attach did not observe ready: {error}"))?;
    thread::sleep(Duration::from_millis(100));
    eprintln!("{implementation:?}: request remote detach");
    let detach = detach_session(implementation, executable, runtime, session_name)
        .map_err(|error| format!("remote detach failed to run: {error}"))?;
    let detach_success = detach.status.success();
    eprintln!("{implementation:?}: wait for attach process");
    let attach_status = process.wait_or_kill(Duration::from_secs(1));
    match &attach_status {
        Ok(status) => eprintln!("{implementation:?}: attach process exited with {status}"),
        Err(error) => {
            eprintln!("{implementation:?}: attach cleanup after remote detach reported {error}");
        }
    }

    Ok(AttachResult {
        output_has_ready: contains(&output, b"ready"),
        status_success: detach_success,
        diagnostic: format!(
            "{}\nattach cleanup: {attach_status:?}; output:\n{}",
            format_output("remote_detach", &detach),
            String::from_utf8_lossy(&output)
        ),
    })
}

fn detach_session(
    implementation: Implementation,
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> io::Result<CommandOutput> {
    match implementation {
        Implementation::Reference | Implementation::Candidate => run_screen(
            executable,
            runtime,
            [
                OsStr::new("-S"),
                OsStr::new(session_name),
                OsStr::new("-X"),
                OsStr::new("detach"),
            ],
            Duration::from_secs(5),
        ),
    }
}

fn quit_session(
    implementation: Implementation,
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> io::Result<CommandOutput> {
    match implementation {
        Implementation::Reference | Implementation::Candidate => run_screen(
            executable,
            runtime,
            [
                OsStr::new("-S"),
                OsStr::new(session_name),
                OsStr::new("-X"),
                OsStr::new("quit"),
            ],
            Duration::from_secs(5),
        ),
    }
}

fn wait_until_no_session(
    implementation: Implementation,
    executable: &Path,
    runtime: &Path,
    session_name: &str,
) -> CleanupResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_diagnostic = "no list output captured".to_owned();
    loop {
        let list = run_screen(
            executable,
            runtime,
            [OsStr::new("-ls")],
            Duration::from_secs(2),
        );
        if let Ok(list) = list {
            let has_session = output_lists_session(&list, session_name);
            if !has_session {
                return CleanupResult::success();
            }
            last_diagnostic = format_output("cleanup_list", &list);
        } else if let Err(error) = list {
            last_diagnostic = format!("cleanup list failed: {error}");
        }

        if Instant::now() >= deadline {
            let _ = quit_session(implementation, executable, runtime, session_name);
            return CleanupResult {
                success: false,
                diagnostic: last_diagnostic,
            };
        }
        thread::sleep(Duration::from_millis(25));
    }
}

struct CleanupResult {
    success: bool,
    diagnostic: String,
}

impl CleanupResult {
    fn success() -> Self {
        Self {
            success: true,
            diagnostic: String::new(),
        }
    }
}

#[derive(Debug)]
struct CommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        // Keep path short to fit Unix socket paths within SUN_LEN (104 on macOS)
        let short = if name.len() > 8 { &name[..8] } else { name };
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let path = temp_base().join(format!("sd-{short}-{nanos}"));
        fs::create_dir(&path).unwrap_or_else(|error| {
            panic!(
                "failed to create temporary directory {}: {error}",
                path.display()
            )
        });
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap_or_else(|error| {
            panic!(
                "failed to chmod temporary directory {}: {error}",
                path.display()
            )
        });
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct SessionGuard {
    implementation: Implementation,
    executable: PathBuf,
    runtime: PathBuf,
    session_name: String,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = quit_session(
            self.implementation,
            &self.executable,
            &self.runtime,
            &self.session_name,
        );
    }
}

fn run_screen<'a>(
    executable: &Path,
    runtime: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    timeout: Duration,
) -> io::Result<CommandOutput> {
    let stdout_path = runtime.join(format!(
        "capture-{}-{}.stdout",
        std::process::id(),
        unique_nanos()
    ));
    let stderr_path = runtime.join(format!(
        "capture-{}-{}.stderr",
        std::process::id(),
        unique_nanos()
    ));
    let stdout_file = File::create(&stdout_path)?;
    let stderr_file = File::create(&stderr_path)?;

    let mut child = Command::new(executable)
        .args(args)
        .env("SCREENDIR", runtime)
        .env("SCREENRC", "/dev/null")
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file.try_clone()?))
        .stderr(Stdio::from(stderr_file.try_clone()?))
        .spawn()?;

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "command timed out after {timeout:?}: {}",
                executable.display()
            );
        }
        thread::sleep(Duration::from_millis(10));
    };

    drop(stdout_file);
    drop(stderr_file);
    let stdout = fs::read(&stdout_path)?;
    let stderr = fs::read(&stderr_path)?;
    let _ = fs::remove_file(stdout_path);
    let _ = fs::remove_file(stderr_path);
    Ok(CommandOutput {
        status,
        stdout,
        stderr,
    })
}

fn run_screen_null<'a>(
    executable: &Path,
    runtime: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    timeout: Duration,
) -> io::Result<ExitStatus> {
    run_screen_null_in_dir(executable, runtime, Path::new("."), args, timeout)
}

fn run_screen_null_with_screenrc<'a>(
    executable: &Path,
    runtime: &Path,
    screenrc: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    timeout: Duration,
) -> io::Result<ExitStatus> {
    run_screen_null_in_dir_with_screenrc(
        executable,
        runtime,
        Path::new("."),
        screenrc.as_os_str(),
        args,
        timeout,
    )
}

fn run_screen_null_with_home<'a>(
    executable: &Path,
    runtime: &Path,
    home: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    timeout: Duration,
) -> io::Result<ExitStatus> {
    let mut child = Command::new(executable)
        .args(args)
        .env("SCREENDIR", runtime)
        .env_remove("SCREENRC")
        .env("HOME", home)
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "command timed out after {timeout:?}: {}",
                executable.display()
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn run_screen_null_in_dir<'a>(
    executable: &Path,
    runtime: &Path,
    current_dir: &Path,
    args: impl IntoIterator<Item = &'a OsStr>,
    timeout: Duration,
) -> io::Result<ExitStatus> {
    run_screen_null_in_dir_with_screenrc(
        executable,
        runtime,
        current_dir,
        OsStr::new("/dev/null"),
        args,
        timeout,
    )
}

fn run_screen_null_in_dir_with_screenrc<'a>(
    executable: &Path,
    runtime: &Path,
    current_dir: &Path,
    screenrc: &OsStr,
    args: impl IntoIterator<Item = &'a OsStr>,
    timeout: Duration,
) -> io::Result<ExitStatus> {
    let mut child = Command::new(executable)
        .args(args)
        .current_dir(current_dir)
        .env("SCREENDIR", runtime)
        .env("SCREENRC", screenrc)
        .env("TERM", "xterm-256color")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "command timed out after {timeout:?}: {}",
                executable.display()
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn output_lists_session(output: &CommandOutput, session_name: &str) -> bool {
    lists_session_name(&output.stdout, session_name)
        || lists_session_name(&output.stderr, session_name)
}

fn wait_for_file(path: &Path, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    loop {
        match fs::read(path) {
            Ok(bytes) => return Ok(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("timed out waiting for {}", path.display()),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_nonempty_file(path: &Path, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    loop {
        match fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => return Ok(bytes),
            Ok(_empty) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("timed out waiting for non-empty {}", path.display()),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn normalize_child_environment(environment: &[u8], session_name: &str) -> String {
    let mut normalized = Vec::new();
    for line in String::from_utf8_lossy(environment).lines() {
        if let Some(sty) = line.strip_prefix("STY=") {
            let suffix = format!(".{session_name}");
            if let Some(pid) = sty.strip_suffix(&suffix)
                && !pid.is_empty()
                && pid.bytes().all(|byte| byte.is_ascii_digit())
            {
                normalized.push(format!("STY=<pid>{suffix}"));
                continue;
            }
        }
        normalized.push(line.to_owned());
    }
    normalized.join("\n")
}

fn lists_session_name(output: &[u8], session_name: &str) -> bool {
    let session = session_name.as_bytes();
    let dotted_session = format!(".{session_name}");
    output.split(|byte| *byte == b'\n').any(|line| {
        let trimmed = trim_ascii_start(line);
        trimmed.starts_with(session) || contains(trimmed, dotted_session.as_bytes())
    })
}

fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let first_non_space = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[first_non_space..]
}

fn unique_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn format_output(label: &str, output: &CommandOutput) -> String {
    format!(
        "{label}: status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn normalize_list_output(output: &CommandOutput, runtime: &Path, session_name: &str) -> String {
    format!(
        "status={}\nstdout={}\nstderr={}",
        output.status.code().unwrap_or(-1),
        normalize_list_stream(&output.stdout, runtime, session_name),
        normalize_list_stream(&output.stderr, runtime, session_name)
    )
}

fn normalize_runtime_path(bytes: &[u8], runtime: &Path) -> Vec<u8> {
    String::from_utf8_lossy(bytes)
        .replace(&runtime.display().to_string(), "<runtime>")
        .into_bytes()
}

fn normalize_list_stream(bytes: &[u8], runtime: &Path, session_name: &str) -> String {
    let runtime = runtime.display().to_string();
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace(&runtime, "<runtime>")
        .lines()
        .map(|line| normalize_list_line(line, session_name))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_list_line(line: &str, session_name: &str) -> String {
    let marker = format!(".{session_name}");
    let Some(marker_index) = line.find(&marker) else {
        return line.to_owned();
    };

    let before_marker = &line[..marker_index];
    let digits_start = before_marker
        .rfind(|character: char| !character.is_ascii_digit())
        .map(|index| index + 1)
        .unwrap_or(0);
    let pid = &before_marker[digits_start..];
    if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return line.to_owned();
    }

    format!("{}<pid>{}", &line[..digits_start], &line[marker_index..])
}

fn unix_socket_bind_allowed(runtime: &Path) -> bool {
    let socket_path = runtime.join("bind-probe");
    match UnixListener::bind(&socket_path) {
        Ok(listener) => {
            drop(listener);
            let _ = fs::remove_file(socket_path);
            true
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => false,
        Err(error) => panic!(
            "unexpected Unix socket bind error at {}: {error}",
            socket_path.display()
        ),
    }
}

fn temp_base() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/private/tmp")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::temp_dir()
    }
}
