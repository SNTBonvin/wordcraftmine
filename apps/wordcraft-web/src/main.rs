//! WordCraft in the browser (see the library's docs).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_arch = "wasm32")]
fn main() {
    wordcraft_web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("wordcraft-web only runs in the browser: build it with `trunk build --release` in apps/wordcraft-web");
}
