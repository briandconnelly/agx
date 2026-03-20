//! agx — Agent Environment Manager library.
//!
//! This module re-exports the core library types so integration tests
//! and future consumers can access them.

pub mod backends;
pub mod cache;
pub mod config;
pub mod context;
pub mod hooks;
pub mod lockfile;
pub mod manifest;
pub mod resolver;
pub mod sources;
