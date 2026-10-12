//! Structural guard for the Windows 11 WM_DPICHANGED fix in the winit vendor.

#[test]
fn windows_11_uses_the_suggested_dpi_rectangle_and_windows_10_keeps_old_path() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let event_loop = std::fs::read_to_string(root.join("vendor/winit/src/platform_impl/windows/event_loop.rs")).unwrap();
    let dark_mode = std::fs::read_to_string(root.join("vendor/winit/src/platform_impl/windows/dark_mode.rs")).unwrap();

    assert!(dark_mode.contains("pub(crate) static WIN10_BUILD_VERSION"));
    assert!(event_loop.contains("if !WIN10_BUILD_VERSION.is_some_and(|build| build < 22000) {"));
    assert!(event_loop.contains("new_outer_rect = suggested_rect;"));
}
