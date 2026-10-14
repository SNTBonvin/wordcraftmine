//! The system print dialog on Linux and the BSDs (`Services::print_dialog`).
//!
//! Shown through the XDG print portal (`org.freedesktop.portal.Print`), so the user gets their
//! desktop's own dialog (GNOME's, KDE Plasma's…, inside Flatpak too) and picks the printer,
//! copies, two-sided printing, paper and the rest there. The portal then sends
//! WordCraft's PDF of the chosen pages to that printer.
//!
//! The portal is asked once, at start-up, whether it prints; where it doesn't (a bare window
//! manager, no portal running), File › Print keeps WordCraft's own printer list (`print`).

/// The portal's settings for a job: WordCraft's copies and collation as GTK print settings,
/// which the dialog opens with (all GTK print settings are strings).
pub fn initial_settings(copies: u32, collate: bool) -> [(&'static str, String); 2] {
    [("n-copies", copies.clamp(1, 999).to_string()), ("collate", collate.to_string())]
}

/// The object path of a portal request made with `token` by the connection named `sender`
/// (`:1.42`), known before the call so its answer can't be missed.
pub fn request_path(sender: &str, token: &str) -> String {
    format!("/org/freedesktop/portal/desktop/request/{}/{token}", sender.trim_start_matches(':').replace('.', "_"))
}

/// What a portal request's answer code means: 0 done, 1 cancelled by the user, else failed.
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Done,
    Cancelled,
    Failed,
}

pub fn answer(code: u32) -> Answer {
    match code {
        0 => Answer::Done,
        1 => Answer::Cancelled,
        _ => Answer::Failed,
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
pub use portal::hook;

#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use std::collections::HashMap;
    use std::io::{Seek, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};

    use wordcraft_ui_egui::printing::{DialogJob, DialogOutcome, SystemDialog};
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

    use super::{Answer, answer, initial_settings, request_path};

    const DESTINATION: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const PRINT: &str = "org.freedesktop.portal.Print";
    const REQUEST: &str = "org.freedesktop.portal.Request";

    const CHECKING: u8 = 0;
    const YES: u8 = 1;
    const NO: u8 = 2;

    /// The print dialog service. Whether the portal prints is found out on a thread of its own,
    /// so start-up never waits for D-Bus.
    pub fn hook() -> SystemDialog {
        let known = Arc::new(AtomicU8::new(CHECKING));
        let shared = known.clone();
        let spawned = std::thread::Builder::new().name("print-portal".into()).spawn(move || {
            let prints = portal_prints();
            log::debug!("print portal available: {prints}");
            shared.store(if prints { YES } else { NO }, Ordering::Relaxed);
        });
        if let Err(error) = spawned {
            log::warn!("print portal check: {error}");
            known.store(NO, Ordering::Relaxed);
        }
        SystemDialog {
            available: Box::new(move || match known.load(Ordering::Relaxed) {
                YES => Some(true),
                NO => Some(false),
                _ => None,
            }),
            print: Box::new(|pdf, job| {
                let (sender, receiver) = std::sync::mpsc::channel();
                let worker = std::thread::Builder::new().name("print-dialog".into()).spawn(move || {
                    let _ = sender.send(show_and_print(&pdf, &job));
                });
                match worker {
                    Ok(worker) => crate::print::keep_until_exit(worker),
                    Err(error) => log::error!("print dialog: {error}"),
                }
                receiver
            }),
        }
    }

    fn portal_prints() -> bool {
        let Ok(connection) = Connection::session() else { return false };
        Proxy::new(&connection, DESTINATION, PATH, PRINT).and_then(|portal| portal.get_property::<u32>("version")).is_ok()
    }

    /// A request token, unique to this process and call.
    fn token(kind: &str) -> String {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        format!("wordcraft_{kind}_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// Make a portal request with `call` and wait for its answer. The user may keep the dialog
    /// open as long as they like; the wait is on this worker, never the UI thread.
    fn request(
        connection: &Connection,
        token: &str,
        call: impl FnOnce() -> zbus::Result<OwnedObjectPath>,
    ) -> Result<(u32, HashMap<String, OwnedValue>), String> {
        let sender = connection.unique_name().map(|name| name.as_str().to_owned()).ok_or("not connected to the session bus")?;
        let expected = request_path(&sender, token);
        let watch = Proxy::new(connection, DESTINATION, expected.as_str(), REQUEST).map_err(text)?;
        let mut responses = watch.receive_signal("Response").map_err(text)?;
        let handle = call().map_err(text)?;
        // Portals older than version 0.9 choose their own path; follow that one.
        let other;
        if handle.as_str() != expected {
            other = Proxy::new(connection, DESTINATION, handle.as_str().to_owned(), REQUEST).map_err(text)?;
            responses = other.receive_signal("Response").map_err(text)?;
        }
        let message = responses.next().ok_or("the print dialog closed without answering")?;
        message.body().deserialize::<(u32, HashMap<String, OwnedValue>)>().map_err(text)
    }

    fn show_and_print(pdf: &[u8], job: &DialogJob) -> Result<DialogOutcome, String> {
        let connection = Connection::session().map_err(text)?;
        let portal = Proxy::new(&connection, DESTINATION, PATH, PRINT).map_err(text)?;

        let wanted = initial_settings(job.copies, job.collate);
        let settings: HashMap<&str, Value> = wanted.iter().map(|(key, value)| (*key, Value::from(value.as_str()))).collect();
        let page_setup: HashMap<&str, Value> = HashMap::new();
        let prepare = token("prepare");
        let options: HashMap<&str, Value> = HashMap::from([
            ("handle_token", Value::from(prepare.as_str())),
            ("modal", Value::from(true)),
            ("accept_label", Value::from(job.accept_label.as_str())),
        ]);
        let (code, results) =
            request(&connection, &prepare, || portal.call("PreparePrint", &("", job.title.as_str(), settings, page_setup, options)))?;
        match answer(code) {
            Answer::Done => {}
            Answer::Cancelled => return Ok(DialogOutcome::Cancelled),
            Answer::Failed => return Err("the print dialog couldn't be shown".into()),
        }
        let ticket = results.get("token").and_then(|value| u32::try_from(value).ok()).ok_or("the print dialog gave no print job")?;
        let printer = results
            .get("settings")
            .and_then(|value| value.try_clone().ok())
            .and_then(|value| HashMap::<String, OwnedValue>::try_from(value).ok())
            .and_then(|chosen| chosen.get("printer").and_then(|name| String::try_from(name.try_clone().ok()?).ok()))
            .unwrap_or_default();

        let file = job_file(pdf)?;
        let send = token("print");
        let options: HashMap<&str, Value> =
            HashMap::from([("handle_token", Value::from(send.as_str())), ("token", Value::from(ticket)), ("modal", Value::from(true))]);
        let (code, _) = request(&connection, &send, || portal.call("Print", &("", job.title.as_str(), Fd::from(&file), options)))?;
        match answer(code) {
            Answer::Done => Ok(DialogOutcome::Printed(printer)),
            Answer::Cancelled => Ok(DialogOutcome::Cancelled),
            Answer::Failed => Err("the printer didn't accept the job".into()),
        }
    }

    /// The PDF in a file only this user can read. Its name is removed at once: the portal reads
    /// it through the open file, and nothing is left behind, even if WordCraft stops.
    fn job_file(pdf: &[u8]) -> Result<std::fs::File, String> {
        use std::os::unix::fs::OpenOptionsExt;
        let path = std::env::temp_dir().join(format!("{}.pdf", token("job")));
        let mut file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path).map_err(text)?;
        let _ = std::fs::remove_file(&path);
        file.write_all(pdf).and_then(|()| file.rewind()).map_err(text)?;
        Ok(file)
    }

    fn text(error: impl std::fmt::Display) -> String {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_watched_on_the_path_the_portal_will_use() {
        assert_eq!(request_path(":1.42", "wordcraft_prepare_7_0"), "/org/freedesktop/portal/desktop/request/1_42/wordcraft_prepare_7_0");
    }

    #[test]
    fn the_dialog_opens_with_the_print_page_copies() {
        assert_eq!(initial_settings(3, false), [("n-copies", "3".to_string()), ("collate", "false".to_string())]);
        assert_eq!(initial_settings(0, true)[0].1, "1", "copies are clamped");
        assert_eq!(initial_settings(5000, true)[0].1, "999");
    }

    #[test]
    fn cancelling_the_dialog_is_not_a_failure() {
        assert_eq!(answer(0), Answer::Done);
        assert_eq!(answer(1), Answer::Cancelled);
        assert_eq!(answer(2), Answer::Failed);
        assert_eq!(answer(u32::MAX), Answer::Failed);
    }
}
