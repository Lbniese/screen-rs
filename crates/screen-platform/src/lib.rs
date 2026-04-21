#[cfg(not(unix))]
compile_error!("screen-rs currently supports Unix targets only");

pub mod runtime;

pub use runtime::{
    RuntimeDirectory, RuntimeDirectoryError, SessionNameError, SocketPathStatus,
    current_effective_uid,
};
