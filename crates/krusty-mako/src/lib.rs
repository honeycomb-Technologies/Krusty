//! Independently supervised Mako daemon transport foundation.
//!
//! Runtime and scheduler implementations plug in through [`CommandHandler`].
//! This crate never depends on `krusty-server`; the server consumes the shared
//! protocol crate as an ordinary authenticated Unix client.

#[cfg(unix)]
mod config;
#[cfg(unix)]
mod handler;
#[cfg(unix)]
mod server;

#[cfg(unix)]
pub use config::{MakoDaemonConfig, MakoPaths};
#[cfg(unix)]
pub use handler::{
    CommandContext, CommandHandler, HandlerReply, HandlerResult, UnavailableCommandHandler,
};
#[cfg(unix)]
pub use server::{DaemonInfo, DaemonServer, DaemonServerHandle};

pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
