//! R2X Manifest Management
//!
//! This module handles all manifest types and operations for the r2x plugin system.
//! It provides the core types for managing plugin metadata from discovery and AST analysis.
//!
//! The manifest is stored in TOML format and contains comprehensive metadata about
//! installed plugins, their configurations, and config schemas.
//!
//! # Version 3.0 Format
//!
//! The new format uses memory-efficient types:
//! - `Arc<str>` for string interning
//! - `SmallVec` for inline small collections
//! - Pre-computed hashes for O(1) comparisons
//! - Indexed lookups for O(1) package/plugin access

pub mod errors;
pub mod manifest;
pub mod package_discovery;
pub mod runtime;
pub mod sync;
pub mod types;
