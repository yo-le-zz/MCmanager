//! MCManager core library.
//!
//! Split out of what used to be a single `main.rs` so the same server
//! management logic (process supervision, downloads, addon handling,
//! backups, crash diagnostics...) can be driven by two different front
//! ends: the `mcmanager` binary (the web UI + REST API most people use)
//! and `mcmanager-headless` (a CLI-only binary with no web server at all,
//! meant for running on a remote Ubuntu box with no browser access - see
//! `src/bin/headless.rs`).
//!
//! Both binaries share the exact same `AppState`/`ServerEntry` model and
//! the same on-disk format, so a server created from one is fully visible
//! and manageable from the other (just not *at the same time* - see
//! `state::acquire_instance_lock`).

pub mod ai;
pub mod api;
pub mod backup;
pub mod cli;
pub mod debug;
pub mod downloader;
pub mod error;
pub mod files;
pub mod history;
pub mod models;
pub mod modrinth;
pub mod ntfy;
pub mod playit;
pub mod presets;
pub mod process;
pub mod remote;
pub mod secrets;
pub mod sleeper;
pub mod state;
pub mod stats;
pub mod updater;
pub mod ws;
