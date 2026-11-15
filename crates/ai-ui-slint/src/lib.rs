//! Build-time location of the Slint source library and bundled assets.
//!
//! This crate contains no renderer or conversation runtime. Register [`ui_dir`]
//! in `slint_build::CompilerConfiguration::with_library_paths`; see the README.
#![forbid(unsafe_code)]

use std::path::PathBuf;

/// Directory containing `lib.slint` and its relative font, icon and license files.
///
/// Use this path during the consuming application's build, not at application
/// runtime. Cargo unpacks package sources before compiling build dependencies.
pub fn ui_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui")
}
