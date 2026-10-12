//! Android print integration is not wired yet; fail gracefully instead of invoking desktop applications.
pub type Hook = Box<dyn Fn(&[u8]) -> Result<(), String>>;
pub fn hook() -> Hook {
    Box::new(|_bytes| Err("Printing is not yet supported on Android".to_owned()))
}
