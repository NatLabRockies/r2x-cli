//! Entry point parsing for plugin discovery
//!
//! This module handles parsing of Python package entry points from:
//! - entry_points.txt files (PEP 566)
//! - pyproject.toml files (PEP 621)
//!
//! Entry points are the primary mechanism for discovering r2x plugins in installed packages.

mod parser;
mod pyproject;

pub use parser::{is_r2x_section, parse_all_entry_points, parse_entry_point_line};
pub use pyproject::{find_pyproject_toml_path, parse_pyproject_entry_points};
