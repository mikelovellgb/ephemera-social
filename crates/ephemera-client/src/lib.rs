//! Ephemera desktop client backend.
//!
//! This crate provides the desktop application for the Ephemera platform.
//! It embeds a full [`ephemera_node::EphemeraNode`] and serves a static
//! SolidJS frontend via an axum HTTP server on localhost.
//!
//! # Architecture
//!
//! ```text
//! Browser/Webview  <--HTTP-->  Axum Server  -->  JSON-RPC Router  -->  EphemeraNode
//!     (SolidJS)                 (localhost)        (dispatch)           (embedded)
//! ```
//!
//! The frontend communicates with the backend through a single `/rpc`
//! HTTP POST endpoint carrying JSON-RPC 2.0 messages. Static frontend
//! assets are embedded in the binary via `rust-embed`.

pub mod commands;
pub mod server;
pub mod state;
