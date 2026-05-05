#![forbid(unsafe_code)]

pub mod invocation;

pub use invocation::{
    AttachOptions, AttachOrCreateOptions, CreateDetachedOptions, CreateOptions, DetachOptions,
    DetachedMode, Invocation, ListOptions, ParseError, QueryOptions, RemoteCommandOptions,
    WipeOptions, parse_invocation,
};
