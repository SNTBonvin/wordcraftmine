//! Android shared-library entry point for WordCraft.
#![allow(unsafe_code)]

// Keep the app implementation in a module so its inner docs and attributes remain valid.
#[path = "main.rs"]
mod app;

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app_handle: winit::platform::android::activity::AndroidApp) {
    if let Err(error) = app::run_app(Some(app_handle)) {
        log::error!("WordCraft failed to start on Android: {error}");
    }
}
