//! R2X library - expose modules for testing
//!
//! This library exposes core modules needed for testing and integration.

pub mod commands;
pub mod common;
pub mod errors;
pub(crate) mod help;
mod install_source;
pub mod manifest_lookup;
pub mod package_verification;
pub mod pipeline_config;
pub mod plugins;
mod uv;

#[cfg(test)]
pub(crate) mod test_support;
