#![allow(clippy::disallowed_methods)] // test code: #[tokio::test] expands to a runtime builder
//! Integration tests for hoocode-code-native: the tier C helper supervisor.
//!
//! `agents` and `resolve` run on any platform. Spawning helpers needs a Unix
//! socket and process groups, so `supervisor` is Unix-only.

mod agents;
mod common;
mod resolve;
#[cfg(unix)]
mod supervisor;
