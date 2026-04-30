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
