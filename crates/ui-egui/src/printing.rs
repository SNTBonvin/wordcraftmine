//! Localized print-page wording around the shared, backend-independent controls.
use craft_ui::printing::Labels;
pub use craft_ui::printing::{Discover, Duplex, Feedback, Printer, Settings, State};

/// Native submission adds WordCraft's PDF-page collation setting to the shared spooler settings.
pub type Submit = Box<dyn Fn(Vec<u8>, Settings, bool) -> std::sync::mpsc::Receiver<Result<String, String>>>;

/// What the system print dialog opens with. The printer, two-sided printing, paper and the rest
/// are chosen in the dialog itself.
#[derive(Clone, Debug, PartialEq)]
pub struct DialogJob {
    /// The job's name in the print queue.
    pub title: String,
    pub copies: u32,
    pub collate: bool,
    /// The dialog's confirm button, in the interface language.
    pub accept_label: String,
}

/// How the system print dialog ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogOutcome {
    /// Sent to the printer the dialog names (empty when it didn't say).
    Printed(String),
    Cancelled,
}

/// The answer once the dialog closes and the job is handed to the spooler.
pub type DialogAnswer = std::sync::mpsc::Receiver<Result<DialogOutcome, String>>;

/// Desktop: the system's own print dialog (on Linux and the BSDs, the desktop's through the XDG
/// print portal). Only the Print button opens it; commands from control, MCP or macros print
/// through [`Submit`] instead and never show a dialog.
pub struct SystemDialog {
    /// Whether the dialog can be shown here; `None` while the host is still finding out.
    pub available: Box<dyn Fn() -> Option<bool>>,
    /// Show the dialog for this PDF, then print it as chosen there. Never blocks the UI.
    pub print: Box<dyn Fn(Vec<u8>, DialogJob) -> DialogAnswer>,
}

/// Desktop: open the chosen printer's driver preferences (Windows' Printer Properties).
pub type PrinterProperties = Box<dyn Fn(&str) -> Result<(), String>>;

/// Metadata for the app-owned commands. Inspecting it never discovers printers or submits.
pub fn commands(app: &crate::WordApp) -> Vec<serde_json::Value> {
    use serde_json::json;
    let enabled = if app.services.print_native.is_some() { app.printing.ready() } else { app.services.print.is_some() };
    let submit_params = "range?: all|current|selection|custom; pages?: string (1-3,5); parity?: all|odd|even; copies?: integer 1..999; collate?: bool. Native uses the selected printer, duplex and grayscale from ui.printSettings; omitted copies/collation use current settings. Browser printing opens its dialog. Submits only when explicitly executed.";
    vec![
        json!({"id":"ui.printSettings", "label":"Print settings", "location":"File › Print", "shortcut":null,
            "params":"printer?: discovered name; copies?: integer 1..999; duplex?: printerDefault|off|longEdge|shortEdge|null; grayscale?: bool; refresh?: bool. Empty params inspect printer/copies/duplex/grayscale, printers, busy/discovering/ready and status/detail. Updates are atomic; refresh discovers only on native hosts. Never submits or opens a dialog.",
            "enabled":true}),
        json!({"id":"ui.printSubmit", "label":"Submit print job", "location":"File › Print", "shortcut":null, "params":submit_params, "enabled":enabled}),
        json!({"id":"ui.print", "label":"Print (alias of ui.printSubmit)", "location":"File › Print", "shortcut":null, "params":submit_params, "enabled":enabled}),
    ]
}

pub fn show(state: &mut State, ui: &mut egui::Ui, discover: &Discover, translate: impl Fn(&str) -> String) {
    let labels = Labels {
        printer: &translate("Printer"),
        select_printer: &translate("Select a printer"),
        refresh: &translate("Refresh printers"),
        copies: &translate("Copies"),
        duplex: &translate("Two-sided printing"),
        printer_default: &translate("Printer default"),
        one_sided: &translate("One-sided"),
        long_edge: &translate("Two-sided: long edge"),
        short_edge: &translate("Two-sided: short edge"),
        grayscale: &translate("Grayscale"),
    };
    state.show(ui, egui::Id::new("wordcraft_print_settings"), discover, &labels);
    if let Some(message) = &state.message {
        ui.label(feedback(message, translate));
    }
}

/// Read print settings/status, or validate an explicit update before changing anything.
/// `refresh: true` starts discovery only; this command never submits a job or opens a dialog.
pub fn settings(state: &mut State, params: &serde_json::Value, discover: Option<&Discover>) -> Result<serde_json::Value, String> {
    use serde_json::{Value, json};
    let object = params.as_object().ok_or("print settings must be an object")?;
    if object.keys().any(|key| !["printer", "copies", "duplex", "grayscale", "refresh"].contains(&key.as_str())) {
        return Err("unknown print setting; use printer, copies, duplex, grayscale or refresh".into());
    }
    if state.busy() && !object.is_empty() {
        return Err("wait for the active print submission before changing settings".into());
    }
    let mut next = state.settings.clone();
    if let Some(value) = object.get("printer") {
        let printer = value.as_str().ok_or("printer must be a name from the discovered list")?;
        if !state.printers.iter().any(|available| available.name == printer) {
            return Err("printer must be a name from the discovered list".into());
        }
        next.printer = printer.to_owned();
    }
    if let Some(value) = object.get("copies") {
        let copies = value.as_u64().filter(|copies| (1..=999).contains(copies)).ok_or("copies must be an integer from 1 to 999")?;
        next.copies = copies as u32;
    }
    if let Some(value) = object.get("duplex") {
        next.duplex = match value.as_str() {
            Some("printerDefault") => None,
            Some("off") => Some(Duplex::Off),
            Some("longEdge") => Some(Duplex::LongEdge),
            Some("shortEdge") => Some(Duplex::ShortEdge),
            None if value.is_null() => None,
            _ => return Err("duplex must be printerDefault, off, longEdge or shortEdge".into()),
        };
    }
    if let Some(value) = object.get("grayscale") {
        next.grayscale = value.as_bool().ok_or("grayscale must be a boolean")?;
    }
    let refresh = object.get("refresh").map(|value| value.as_bool().ok_or("refresh must be a boolean")).transpose()?.unwrap_or(false);
    if refresh {
        let discover = discover.ok_or("native printer discovery is unavailable here")?;
        if !state.refresh(discover) {
            return Err("printer discovery or submission is already active".into());
        }
    }
    state.settings = next;
    let (status, detail) = match state.message.as_ref() {
        None => ("idle", ""),
        Some(Feedback::FindingPrinters) => ("findingPrinters", ""),
        Some(Feedback::NoPrinters) => ("noPrinters", ""),
        Some(Feedback::DiscoveryFailed) => ("discoveryFailed", ""),
        Some(Feedback::Submitting) => ("submitting", ""),
        Some(Feedback::Submitted(receipt)) => ("submitted", receipt.as_str()),
        Some(Feedback::Failed(error)) => ("failed", error.as_str()),
        Some(Feedback::WorkerStopped) => ("workerStopped", ""),
    };
    Ok(json!({
        "printer": state.settings.printer, "copies": state.settings.copies,
        "duplex": match state.settings.duplex { None => "printerDefault", Some(Duplex::Off) => "off", Some(Duplex::LongEdge) => "longEdge", Some(Duplex::ShortEdge) => "shortEdge" },
        "grayscale": state.settings.grayscale,
        "printers": state.printers.iter().map(|printer| json!({"name": printer.name, "default": printer.default})).collect::<Vec<Value>>(),
        "busy": state.busy(), "discovering": state.discovering(), "ready": state.ready(), "status": status, "detail": detail,
    }))
}

pub fn feedback(message: &Feedback, translate: impl Fn(&str) -> String) -> String {
    match message {
        Feedback::FindingPrinters => translate("Finding printers…"),
        Feedback::NoPrinters => translate("No printers found. Configure a system printer or export a PDF."),
        Feedback::DiscoveryFailed => translate("Printer discovery failed."),
        Feedback::Submitting => translate("Submitting print job…"),
        Feedback::Submitted(receipt) if receipt.trim().is_empty() => translate("Print job submitted."),
        Feedback::Submitted(receipt) => translate("Print job submitted: {receipt}").replace("{receipt}", receipt),
        Feedback::Failed(error) => translate("Print failed: {error}").replace("{error}", error),
        Feedback::WorkerStopped => translate("Print failed: the printing worker stopped."),
    }
}

/// Build the upstream page-range/markup PDF and hand it to the chosen host print flow.
pub fn submit(app: &mut crate::WordApp, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::json;
    if app.services.print.is_none() && app.services.print_native.is_none() {
        return Err("printing to the system print dialog isn't available here; export a PDF instead".into());
    }
    let native = app.services.print_native.is_some();
    if native && !app.printing.ready() {
        return Err("Select an available printer in File > Print before submitting.".into());
    }
    let mut pdf_params = params.clone();
    let mut settings = app.printing.settings.clone();
    let collate = params.get("collate").map(|value| value.as_bool().ok_or("collate must be a boolean")).transpose()?.unwrap_or(app.ui.print.collate);
    if native {
        let object = pdf_params.as_object_mut().ok_or("print settings must be an object")?;
        if let Some(copies) = object.get("copies") {
            settings.copies = copies.as_u64().filter(|copies| (1..=999).contains(copies)).ok_or("copies must be an integer from 1 to 999")? as u32;
        }
        // Copies/collation belong to the spooler. Baking them into the PDF would multiply
        // the copies twice, and duplex could put the end of one copy behind the next.
        object.insert("copies".into(), json!(1));
    }
    let pdf = print_pdf(app, &pdf_params);
    let result = pdf.and_then(|(bytes, plan)| {
        if native && plan.chosen.checked_mul(settings.copies as usize).is_none_or(|pages| pages > wordcraft_engine::print::MAX_PRINT_PAGES) {
            return Err("the selected pages and copies exceed the print-job page limit".into());
        }
        let len = bytes.len();
        if let Some(submit) = &app.services.print_native {
            app.printing.begin(submit(bytes, settings, collate)).map_err(|_| "A print job is already being submitted.".to_owned())?;
            app.print_feedback_seen = None;
        } else if let Some(print) = &app.services.print {
            print(&bytes)?;
        }
        Ok(len)
    });
    match result {
        Ok(len) => {
            app.status(if native { tl!("Submitting print job…") } else { tl!("Opening print dialog…") });
            Ok(json!({"printing": true, "bytes": len, "pending": native}))
        }
        Err(error) => {
            log::error!("print failed: {error}");
            let msg = crate::i18n::fmt(tl!("Print failed: {error}"), &[("error", &error)]);
            app.status(msg.clone());
            Err(msg)
        }
    }
}

/// The print job's PDF for `params`, with fields updated first and the page colour left out
/// when File › Options says so.
fn print_pdf(app: &mut crate::WordApp, params: &serde_json::Value) -> Result<(Vec<u8>, wordcraft_engine::print::PrintPlan), String> {
    if app.ui.update_fields_before_print
        && let Err(error) = app.session.run("references.updateFields", &serde_json::json!({}))
    {
        log::warn!("updating fields before printing: {error}");
    }
    let colour = if app.ui.print_backgrounds { None } else { app.session.doc.settings.page_color.take() };
    let pdf = app.session.print_pdf(params).map_err(|error| error.to_string());
    if colour.is_some() {
        app.session.doc.settings.page_color = colour;
    }
    pdf
}

/// Whether File › Print's Print button opens the system print dialog here.
pub fn dialog_available(app: &crate::WordApp) -> bool {
    app.services.print_dialog.as_ref().and_then(|dialog| (dialog.available)()) == Some(true)
}

/// The Print button where the system has a print dialog: the pages chosen on the Print page go
/// to the dialog, where the user picks the printer and the rest. Not a command: only the button
/// calls this, so nothing programmatic opens a dialog.
pub fn print_with_dialog(app: &mut crate::WordApp) -> Result<(), String> {
    if !dialog_available(app) {
        return Err("the system print dialog isn't available here".into());
    }
    if app.print_dialog_open() || app.printing.busy() {
        return Err("A print job is already being submitted.".into());
    }
    let mut params = app.ui.print.params();
    // Copies and collation are chosen in the dialog and applied by the spooler.
    params["copies"] = serde_json::json!(1);
    let job = DialogJob {
        title: app.title_stem(),
        copies: app.ui.print.copies.clamp(1, wordcraft_engine::print::MAX_COPIES),
        collate: app.ui.print.collate,
        accept_label: crate::i18n::t("Print").to_owned(),
    };
    let result = print_pdf(app, &params).and_then(|(bytes, plan)| {
        if plan.chosen.checked_mul(job.copies as usize).is_none_or(|pages| pages > wordcraft_engine::print::MAX_PRINT_PAGES) {
            return Err("the selected pages and copies exceed the print-job page limit".to_owned());
        }
        let dialog = app.services.print_dialog.as_ref().ok_or("the system print dialog isn't available here")?;
        Ok((dialog.print)(bytes, job))
    });
    match result {
        Ok(answer) => {
            app.set_print_dialog(answer);
            app.status(tl!("Opening print dialog…"));
            Ok(())
        }
        Err(error) => {
            log::error!("print failed: {error}");
            let msg = crate::i18n::fmt(tl!("Print failed: {error}"), &[("error", &error)]);
            app.status(msg.clone());
            Err(msg)
        }
    }
}

/// The status-bar message for how the system print dialog ended.
pub fn dialog_feedback(answer: Result<DialogOutcome, String>, translate: impl Fn(&str) -> String) -> String {
    match answer {
        Ok(DialogOutcome::Printed(printer)) if printer.trim().is_empty() => translate("Print job submitted."),
        Ok(DialogOutcome::Printed(printer)) => translate("Sent to {printer}.").replace("{printer}", printer.trim()),
        Ok(DialogOutcome::Cancelled) => translate("Printing cancelled."),
        Err(error) => translate("Print failed: {error}").replace("{error}", &error),
    }
}
