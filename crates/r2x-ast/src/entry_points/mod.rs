//! Entry point parsing for plugin discovery
//!
//! This module handles parsing of Python package entry points from:
//! - entry_points.txt files (PEP 566)
//! - pyproject.toml files (PEP 621)
//!
//! Entry points are the primary mechanism for discovering r2x plugins in installed packages.

pub mod parser;
pub mod pyproject;
