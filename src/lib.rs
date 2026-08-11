//! repomonk library surface for the binary and integration tests.

pub mod app;
pub mod cli;
pub mod config;
pub mod domain;
pub mod error;
pub mod scan;
pub mod source;
pub mod store;
pub mod ui;

pub use error::{Error, Result};
