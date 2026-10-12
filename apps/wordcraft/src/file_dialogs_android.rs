//! Android placeholder for native desktop file dialogs. Android document picker integration is a follow-up.
use std::sync::mpsc::{Receiver, channel};
use wordcraft_ui_egui::FileDialogRequest;
pub type Hook = Box<dyn Fn(FileDialogRequest) -> Receiver<Option<String>>>;
pub struct Launcher;
pub fn hook(_ctx: &egui::Context) -> (Hook, Launcher) {
    let hook: Hook = Box::new(|_req| {
        let (_tx, rx) = channel();
        rx
    });
    (hook, Launcher)
}
impl Launcher {
    pub fn show(&self, _parent: &eframe::Frame) {}
}
