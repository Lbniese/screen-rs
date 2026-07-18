use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use screen_testkit::{
    ScreenExecutable, TestEnvironment, compare_command_results, default_reference_path,
};

#[test]
fn first_cli_differential_cases_produce_structured_reports() {
    let candidate_path = std::env::var_os("SCREEN_CANDIDATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_screen-rs")));
    let candidate = ScreenExecutable::new(candidate_path);
    let reference = ScreenExecutable::new(default_reference_path());
    let environment =
        TestEnvironment::create("screen-rs-cli-diff").expect("test environment should be created");

    let cases: Vec<(&str, Vec<OsString>)> = vec![
        ("no_arguments", vec![]),
        ("help", vec![OsString::from("--help")]),
        ("version", vec![OsString::from("--version")]),
        (
            "invalid_option",
            vec![OsString::from("--screen-rs-invalid")],
        ),
        ("list_no_sessions", vec![OsString::from("-ls")]),
        ("wipe_no_sessions", vec![OsString::from("-wipe")]),
        // New CLI options - these should at least parse without errors
        (
            "quiet_mode",
            vec![OsString::from("-q"), OsString::from("--version")],
        ),
        (
            "flow_control_on",
            vec![OsString::from("-f"), OsString::from("--version")],
        ),
        (
            "flow_control_off",
            vec![OsString::from("-fn"), OsString::from("--version")],
        ),
        (
            "flow_control_auto",
            vec![OsString::from("-fa"), OsString::from("--version")],
        ),
        (
            "interrupt_sooner",
            vec![OsString::from("-i"), OsString::from("--version")],
        ),
        (
            "optimal_output",
            vec![OsString::from("-O"), OsString::from("--version")],
        ),
        (
            "utf8_mode",
            vec![OsString::from("-U"), OsString::from("--version")],
        ),
        (
            "force_all_capabilities",
            vec![OsString::from("-a"), OsString::from("--version")],
        ),
        (
            "adapt_all_windows",
            vec![OsString::from("-A"), OsString::from("--version")],
        ),
    ];

    let mut reports = Vec::new();

    for (case_name, args) in cases {
        let reference_result = reference
            .run_with_timeout(args.iter(), &environment, Duration::from_secs(2))
            .expect("reference command should complete or fail cleanly");
        let candidate_result = candidate
            .run_with_timeout(args.iter(), &environment, Duration::from_secs(2))
            .expect("candidate command should complete");

        let report = compare_command_results(case_name, &reference_result, &candidate_result);
        assert_eq!(report.case_name(), case_name);
        reports.push(report);
    }

    assert_eq!(reports.len(), 15);

    for report in reports {
        if !report.is_match() {
            eprintln!("{report}");
        }
    }
}
