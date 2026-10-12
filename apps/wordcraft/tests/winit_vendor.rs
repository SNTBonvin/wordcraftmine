//! Structural guard for the checksum-verified local winit source override.

#[test]
fn winit_resolves_to_the_pristine_vendor_baseline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let winit = lock.split("[[package]]").find(|package| package.contains("\nname = \"winit\"\n")).expect("winit is in Cargo.lock");

    assert!(winit.contains("\nversion = \"0.30.13\"\n"));
    assert!(!winit.contains("\nsource = "), "winit must resolve to vendor/winit");

    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(
        manifest.contains("[patch.crates-io]\nwinit = { path = \"vendor/winit\" }"),
        "Cargo no longer patches crates.io winit to the local vendor"
    );
}
