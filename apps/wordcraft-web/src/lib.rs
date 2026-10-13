//! WordCraft in the browser: the full app (the `wordcraft-web` binary, [`start`]) and the editor
//! embedded in another web app ([`embed::WordcraftEmbed`], built as a library with
//! `wasm-bindgen --target web`).
//!
//! The app runs the same [`wordcraft_ui_egui::WordApp`] as the desktop app through eframe's web
//! runner (wgpu: WebGPU where available, WebGL2 otherwise). Build it with `trunk build --release`
//! from this directory (output in `dist/web`).
//!
//! Differences from the desktop app: no TCP control channel; Open and Insert › Pictures use the
//! browser file picker (bytes arrive through `Services::inbox`); Save and Export download.
//! URL flags: `?webgl` forces WebGL2; `?sample` opens the sample document.
//!
//! The embedded editor: `cargo build --release --target wasm32-unknown-unknown -p wordcraft-web
//! --lib`, then `wasm-bindgen --target web` on `wordcraft_web.wasm`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_arch = "wasm32")]
pub mod embed;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
pub use web::start;
