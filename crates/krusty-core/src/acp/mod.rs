//! ACP (Agent Client Protocol) server implementation
//!
//! This module implements the Agent Client Protocol, enabling Krusty to act as
//! an ACP-compatible agent that can integrate with any ACP-supporting editor
//! (Zed, Neovim, JetBrains, Marimo).
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────┐     JSON-RPC 2.0      ┌─────────────────┐
//! │     Editor      │◄────── stdio ────────►│     Krusty      │
//! │  (ACP Client)   │                       │   (ACP Agent)   │
//! │                 │   initialize          │                 │
//! │  - Zed          │   session/new         │  KrustyAgent    │
//! │  - Neovim       │   session/prompt      │  SessionManager │
//! │  - JetBrains    │   session/update      │  Tool Bridge    │
//! └─────────────────┘                       └─────────────────┘
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use krusty_core::acp::AcpServer;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let server = AcpServer::new()?;
//!     server.run().await
//! }
//! ```

mod agent;
mod error;
mod processor;
mod server;
mod session;
mod tools;
mod updates;

pub use agent::KrustyAgent;
pub use error::AcpError;
pub use server::AcpServer;
pub use session::{SessionManager, SessionState};
