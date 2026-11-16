//! The Rust core for the iOS app, through `UniFFI`: fetching a spot through the Swift host, the
//! view and the settings.

uniffi::setup_scaffolding!();

mod fetch;
mod settings;
mod view;
