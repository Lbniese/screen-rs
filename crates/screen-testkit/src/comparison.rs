use std::fmt;
use std::process::ExitStatus;

use crate::CommandResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandComparison {
    case_name: String,
    reference: CommandResultSummary,
    candidate: CommandResultSummary,
    mismatches: Vec<Mismatch>,
}

impl CommandComparison {
    pub fn case_name(&self) -> &str {
        &self.case_name
    }

    pub fn reference(&self) -> &CommandResultSummary {
        &self.reference
    }

    pub fn candidate(&self) -> &CommandResultSummary {
        &self.candidate
    }

    pub fn mismatches(&self) -> &[Mismatch] {
        &self.mismatches
    }

    pub fn is_match(&self) -> bool {
        self.mismatches.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResultSummary {
    pub status_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    pub field: &'static str,
    pub reference: MismatchValue,
    pub candidate: MismatchValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MismatchValue {
    Status(Option<i32>),
    Bytes(Vec<u8>),
}

pub fn compare_command_results(
    case_name: impl Into<String>,
    reference: &CommandResult,
    candidate: &CommandResult,
) -> CommandComparison {
    let reference = summarize(reference);
    let candidate = summarize(candidate);
    let mut mismatches = Vec::new();

    if reference.status_code != candidate.status_code {
        mismatches.push(Mismatch {
            field: "status",
            reference: MismatchValue::Status(reference.status_code),
            candidate: MismatchValue::Status(candidate.status_code),
        });
    }
    if reference.stdout != candidate.stdout {
        mismatches.push(Mismatch {
            field: "stdout",
            reference: MismatchValue::Bytes(reference.stdout.clone()),
            candidate: MismatchValue::Bytes(candidate.stdout.clone()),
        });
    }
    if reference.stderr != candidate.stderr {
        mismatches.push(Mismatch {
            field: "stderr",
            reference: MismatchValue::Bytes(reference.stderr.clone()),
            candidate: MismatchValue::Bytes(candidate.stderr.clone()),
        });
    }

    CommandComparison {
        case_name: case_name.into(),
        reference,
        candidate,
        mismatches,
    }
}

impl fmt::Display for CommandComparison {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "case: {}", self.case_name)?;
        if self.mismatches.is_empty() {
            return writeln!(formatter, "result: match");
        }

        writeln!(formatter, "result: mismatch")?;
        for mismatch in &self.mismatches {
            writeln!(formatter, "- field: {}", mismatch.field)?;
            writeln!(formatter, "  reference: {}", mismatch.reference)?;
            writeln!(formatter, "  candidate: {}", mismatch.candidate)?;
        }
        Ok(())
    }
}

impl fmt::Display for MismatchValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(status) => write!(formatter, "{status:?}"),
            Self::Bytes(bytes) => write!(formatter, "{:?}", String::from_utf8_lossy(bytes)),
        }
    }
}

fn summarize(result: &CommandResult) -> CommandResultSummary {
    CommandResultSummary {
        status_code: status_code(result.status),
        stdout: result.stdout.clone(),
        stderr: result.stderr.clone(),
    }
}

fn status_code(status: ExitStatus) -> Option<i32> {
    status.code()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    fn make_result(status_code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> CommandResult {
        // ExitStatus::from_raw takes the raw wait status.
        // For a normal exit, this is (exit_code << 8). The code() method
        // extracts the exit code by shifting right.
        let raw = status_code.unwrap_or(0) << 8;
        CommandResult {
            status: ExitStatus::from_raw(raw),
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        }
    }

    #[test]
    fn test_matching_results() {
        let ref_result = make_result(Some(0), b"output", b"");
        let cand_result = make_result(Some(0), b"output", b"");
        let comparison = compare_command_results("match_test", &ref_result, &cand_result);
        assert!(comparison.is_match());
        assert_eq!(comparison.case_name(), "match_test");
        assert!(comparison.mismatches().is_empty());
    }

    #[test]
    fn test_mismatched_status() {
        let ref_result = make_result(Some(0), b"same", b"");
        let cand_result = make_result(Some(1), b"same", b"");
        let comparison = compare_command_results("mismatch_status", &ref_result, &cand_result);
        assert!(!comparison.is_match());
        assert_eq!(comparison.mismatches().len(), 1);
        assert_eq!(comparison.mismatches()[0].field, "status");
    }

    #[test]
    fn test_mismatched_stdout() {
        let ref_result = make_result(Some(0), b"reference output", b"");
        let cand_result = make_result(Some(0), b"candidate output", b"");
        let comparison = compare_command_results("mismatch_stdout", &ref_result, &cand_result);
        assert!(!comparison.is_match());
        assert_eq!(comparison.mismatches().len(), 1);
        assert_eq!(comparison.mismatches()[0].field, "stdout");
    }

    #[test]
    fn test_mismatched_stderr() {
        let ref_result = make_result(Some(0), b"", b"error ref");
        let cand_result = make_result(Some(0), b"", b"error cand");
        let comparison = compare_command_results("mismatch_stderr", &ref_result, &cand_result);
        assert!(!comparison.is_match());
        assert_eq!(comparison.mismatches().len(), 1);
        assert_eq!(comparison.mismatches()[0].field, "stderr");
    }

    #[test]
    fn test_multiple_mismatches() {
        let ref_result = make_result(Some(0), b"out1", b"err1");
        let cand_result = make_result(Some(1), b"out2", b"err2");
        let comparison = compare_command_results("multi_mismatch", &ref_result, &cand_result);
        assert!(!comparison.is_match());
        assert_eq!(comparison.mismatches().len(), 3);
    }

    #[test]
    fn test_comparison_display_match() {
        let result = make_result(Some(0), b"hello", b"");
        let comparison = compare_command_results("display_match", &result, &result);
        let display = comparison.to_string();
        assert!(display.contains("display_match"), "display={display}");
        assert!(display.contains("match"), "display={display}");
    }

    #[test]
    fn test_comparison_display_mismatch() {
        let ref_result = make_result(Some(0), b"hello", b"");
        let cand_result = make_result(Some(1), b"world", b"");
        let comparison = compare_command_results("display_mismatch", &ref_result, &cand_result);
        let display = comparison.to_string();
        assert!(display.contains("mismatch"), "display={display}");
        assert!(display.contains("status"), "display={display}");
        assert!(display.contains("stdout"), "display={display}");
    }

    #[test]
    fn test_command_comparison_api() {
        let ref_result = make_result(Some(0), b"ref data", b"");
        let cand_result = make_result(Some(1), b"cand data", b"");
        let comparison = compare_command_results("api_test", &ref_result, &cand_result);

        assert_eq!(comparison.case_name(), "api_test");
        assert_eq!(comparison.reference().status_code, Some(0));
        assert_eq!(comparison.reference().stdout, b"ref data");
        assert_eq!(comparison.candidate().status_code, Some(1));
        assert_eq!(comparison.candidate().stdout, b"cand data");
    }

    #[test]
    fn test_mismatch_value_display_status() {
        let val = MismatchValue::Status(Some(0));
        assert_eq!(val.to_string(), "Some(0)");
    }

    #[test]
    fn test_mismatch_value_display_status_none() {
        let val = MismatchValue::Status(None);
        assert_eq!(val.to_string(), "None");
    }

    #[test]
    fn test_mismatch_value_display_bytes() {
        let val = MismatchValue::Bytes(b"hello world".to_vec());
        let s = val.to_string();
        assert!(s.contains("hello"), "s={s}");
    }

    #[test]
    fn test_mismatch_value_display_empty_bytes() {
        let val = MismatchValue::Bytes(b"".to_vec());
        let s = val.to_string();
        assert!(!s.is_empty(), "empty bytes should still display");
    }

    #[test]
    fn test_command_result_summary_equality() {
        let a = CommandResultSummary {
            status_code: Some(0),
            stdout: b"data".to_vec(),
            stderr: b"".to_vec(),
        };
        let b = CommandResultSummary {
            status_code: Some(0),
            stdout: b"data".to_vec(),
            stderr: b"".to_vec(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn test_command_result_summary_inequality() {
        let a = CommandResultSummary {
            status_code: Some(0),
            stdout: b"data".to_vec(),
            stderr: b"".to_vec(),
        };
        let b = CommandResultSummary {
            status_code: Some(1),
            stdout: b"data".to_vec(),
            stderr: b"".to_vec(),
        };
        assert_ne!(a, b);
    }

    #[test]
    fn test_no_panic_on_null_bytes() {
        let ref_result = make_result(Some(0), b"\0data\0", b"");
        let cand_result = make_result(Some(0), b"\0data\0", b"");
        // Should not panic
        let comparison = compare_command_results("null_test", &ref_result, &cand_result);
        assert!(comparison.is_match());
    }
}
