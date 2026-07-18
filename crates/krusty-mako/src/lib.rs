//! Independently supervised Mako daemon transport foundation.
//!
//! The HTTP server consumes only the shared protocol as an authenticated Unix
//! client. The daemon binary reuses `krusty-server`'s agent execution host so
//! providers, tools, hooks, and Mako prompt behavior have one implementation;
//! no HTTP listener or route is started in this process.

#[cfg(unix)]
mod config;
#[cfg(unix)]
mod executor;
#[cfg(unix)]
mod handler;
#[cfg(unix)]
mod runtime;
#[cfg(unix)]
mod server;

#[cfg(unix)]
pub use config::{MakoDaemonConfig, MakoPaths};
#[cfg(unix)]
pub use executor::KrustyExecutionBackend;
#[cfg(unix)]
pub use handler::{
    CommandContext, CommandHandler, HandlerReply, HandlerResult, UnavailableCommandHandler,
};
#[cfg(unix)]
pub use runtime::{
    start_runtime, DurableMakoCommandHandler, ExecutionBackend, ExecutionControl, ExecutionEvent,
    ExecutionEventSendError, ExecutionEventSink, ExecutionOutcome, ExecutionRequest,
    MakoRuntimeConfig, MakoRuntimeHandle, UnavailableExecutionBackend,
};
#[cfg(unix)]
pub use server::{DaemonInfo, DaemonServer, DaemonServerHandle};

pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
