#![forbid(unsafe_code)]

mod comparison;
mod environment;
mod executable;
mod pty;

pub use comparison::{
    CommandComparison, CommandResultSummary, Mismatch, MismatchValue, compare_command_results,
};
pub use environment::TestEnvironment;
pub use executable::{CommandResult, ScreenExecutable, TestError, default_reference_path};
pub use pty::PtyTestProcess;
pub use screen_pty::{PtyError, PtySize};
