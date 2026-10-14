//! Native print onboarding preserves engine PDF generation and submits the selected settings.
use serde_json::json;
use std::{cell::RefCell, rc::Rc};
use wordcraft_engine::Session;
use wordcraft_ui_egui::printing::{Duplex, Feedback, Printer};
use wordcraft_ui_egui::{Services, WordApp};

#[test]
fn control_print_discovery_reports_metadata_without_host_side_effects() {
    use std::cell::Cell;
    use wordcraft_ui_egui::control::{ControlRequest, Outcome, handle};
    let discoveries = Rc::new(Cell::new(0));
    let submissions = Rc::new(Cell::new(0));
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    let observed = discoveries.clone();
    app.services.printers = Some(Box::new(move || {
        observed.set(observed.get() + 1);
        let (_, receiver) = std::sync::mpsc::channel();
        receiver
    }));
    let observed = submissions.clone();
    app.services.print_native = Some(Box::new(move |_, _, _| {
        observed.set(observed.get() + 1);
        let (_, receiver) = std::sync::mpsc::channel();
        receiver
    }));
    let ctx = egui::Context::default();
    let (request, _) = ControlRequest::new("engine.commands", json!({"search":"print"}));
    let Outcome::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("metadata replies immediately") };
    assert_eq!(reply["ok"], true);
    let commands = reply["result"].as_array().unwrap();
    assert!(commands.iter().any(|command| command["id"] == "file.print"), "engine print commands remain visible");
    let settings = commands.iter().find(|command| command["id"] == "ui.printSettings").unwrap();
    assert_eq!(settings["enabled"], true, "settings/status inspection is available");
    for field in ["printer", "copies", "duplex", "grayscale", "refresh", "status", "detail"] {
        assert!(settings["params"].as_str().unwrap().contains(field), "documented setting: {field}");
    }
    for id in ["ui.printSubmit", "ui.print"] {
        let command = commands.iter().find(|command| command["id"] == id).unwrap();
        assert_eq!(command["enabled"], false, "a printer must be selected before submission");
        for field in ["range", "pages", "parity", "copies", "collate"] {
            assert!(command["params"].as_str().unwrap().contains(field), "{id}: {field}");
        }
    }
    app.printing.printers.push(Printer { name: "Queue".into(), default: true });
    app.run("ui.printSettings", json!({"printer":"Queue"})).unwrap();
    let Outcome::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("metadata replies immediately") };
    for id in ["ui.printSubmit", "ui.print"] {
        let command = reply["result"].as_array().unwrap().iter().find(|command| command["id"] == id).unwrap();
        assert_eq!(command["enabled"], true, "the selected queue enables {id}");
    }
    assert_eq!(discoveries.get(), 0, "metadata never starts printer discovery");
    assert_eq!(submissions.get(), 0, "metadata never submits");
    assert!(app.dialog.is_none() && !app.ui.backstage, "metadata never opens a dialog or Print page");
    assert!(!app.printing.busy() && !app.session.dirty);
}

#[test]
fn native_print_uses_selected_settings_and_reports_spool_errors() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.run("text.insert", json!({"text": "A document to print"})).unwrap();
    let jobs = Rc::new(RefCell::new(Vec::new()));
    let observed = jobs.clone();
    app.services.print_native = Some(Box::new(move |bytes, settings, collate| {
        observed.borrow_mut().push((bytes, settings, collate));
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Err("printer is offline".into())).unwrap();
        receiver
    }));
    assert!(app.run("ui.print", json!({})).is_err());
    assert!(jobs.borrow().is_empty(), "opening settings/exporting does not spool");
    app.printing.printers.push(Printer { name: "Chosen queue".into(), default: false });
    let configured = app.run("ui.printSettings", json!({"printer":"Chosen queue", "copies":3,"duplex":"shortEdge","grayscale":true})).unwrap();
    assert_eq!(configured["ready"], true);
    assert_eq!(configured["printers"][0]["name"], "Chosen queue");
    app.run("ui.print", json!({})).unwrap();
    assert!(app.printing.busy());
    assert!(!app.close_requested(), "a pending print job cannot be interrupted by normal close");
    let jobs = jobs.borrow();
    assert_eq!(jobs.len(), 1);
    assert!(jobs[0].0.starts_with(b"%PDF"));
    assert_eq!(jobs[0].1.printer, "Chosen queue");
    assert_eq!(jobs[0].1.copies, 3);
    assert_eq!(jobs[0].1.duplex, Some(Duplex::ShortEdge));
    assert!(jobs[0].1.grayscale);
    assert!(jobs[0].2, "copies are collated by default");
    app.poll_printing(&egui::Context::default());
    assert_eq!(app.printing.message, Some(Feedback::Failed("printer is offline".into())));
    assert!(app.status_msg.as_ref().unwrap().0.contains("printer is offline"));
}

#[test]
fn print_settings_are_visible_in_a_narrow_window() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.run("text.insert", json!({"text": "A document to print"})).unwrap();
    app.services.printers = Some(Box::new(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(vec![Printer { name: "Test queue".into(), default: true }]).unwrap();
        receiver
    }));
    app.services.print_native = Some(Box::new(|_, _, _| {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Ok("queued".into())).unwrap();
        receiver
    }));
    app.ui.backstage = true;
    app.ui.backstage_page = "print".into();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 700.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(5);
    assert!(!harness.get_by_label("Refresh printers").accesskit_node().is_disabled());
    assert!(!harness.get_by_label("Grayscale").accesskit_node().is_disabled());
    // The settings column scrolls on its own; Grayscale is reached by scrolling it.
    harness.get_by_label("Grayscale").scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label("Grayscale").click();
    harness.run_steps(2);
    assert!(harness.state().printing.settings.grayscale);
    assert!(harness.state().printing.ready());
}

#[test]
fn invalid_print_settings_are_rejected_without_partial_changes() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.printing.printers.push(Printer { name: "Queue".into(), default: true });
    let before = app.run("ui.printSettings", json!({})).unwrap();
    for params in [
        json!({"printer":"Missing"}),
        json!({"copies":0}),
        json!({"copies":1000}),
        json!({"copies":1.5}),
        json!({"copies":-1}),
        json!({"copies":"3"}),
        json!({"duplex":"maybe"}),
        json!({"grayscale":"yes"}),
        json!({"refresh":"yes"}),
        json!({"copies":3,"duplex":"invalid"}),
        json!([]),
    ] {
        assert!(app.run("ui.printSettings", params.clone()).is_err(), "{params}");
        assert_eq!(app.run("ui.printSettings", json!({})).unwrap(), before, "no partial update: {params}");
    }
}

#[test]
fn repeated_fast_jobs_each_surface_an_identical_receipt() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.printing.printers.push(Printer { name: "Queue".into(), default: true });
    app.run("ui.printSettings", json!({"printer":"Queue"})).unwrap();
    app.services.print_native = Some(Box::new(|_, _, _| {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Ok("same receipt".into())).unwrap();
        receiver
    }));
    for _ in 0..2 {
        app.status_msg = None;
        app.run("ui.printSubmit", json!({})).unwrap();
        // Completion may be consumed by the shared widget before the app's logic pass.
        app.printing.poll(&egui::Context::default());
        app.poll_printing(&egui::Context::default());
        assert!(app.status_msg.as_ref().unwrap().0.contains("same receipt"));
        app.status_msg = None;
        app.poll_printing(&egui::Context::default());
        assert!(app.status_msg.is_none());
    }
}

#[test]
fn native_ranges_and_parity_do_not_duplicate_spooler_copies() {
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    for page in 1..=3 {
        app.run("text.insert", json!({"text":format!("Distinct page {page}")})).unwrap();
        if page < 3 {
            app.run("insert.pageBreak", json!({})).unwrap();
        }
    }
    assert_eq!(app.session.export_layout().pages.len(), 3);
    app.printing.printers.push(Printer { name: "Queue".into(), default: true });
    app.run("ui.printSettings", json!({"printer":"Queue","copies":3})).unwrap();
    let jobs = Rc::new(RefCell::new(Vec::new()));
    let observed = jobs.clone();
    app.services.print_native = Some(Box::new(move |bytes, settings, collate| {
        observed.borrow_mut().push((bytes, settings, collate));
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Ok("queued".into())).unwrap();
        receiver
    }));
    for bad in [json!({"pages":"8"}), json!({"parity":"bad"}), json!({"copies":0}), json!({"collate":"yes"})] {
        assert!(app.run("ui.printSubmit", bad).is_err());
    }
    assert!(jobs.borrow().is_empty(), "invalid settings do not send any job");
    let params = json!({"pages":"1-3","parity":"odd","collate":false});
    let (expected, plan) = app.session.print_pdf(&params).unwrap();
    assert_eq!(plan.pages, vec![0, 2]);
    app.run("ui.printSubmit", params).unwrap();
    let jobs = jobs.borrow();
    assert_eq!(jobs[0].0, expected, "the PDF contains only the two selected pages, once each");
    assert_eq!(jobs[0].1.copies, 3, "copies go to the spooler, not the PDF");
    assert!(!jobs[0].2, "uncollated copies reach the spooler");
}

/// A system print dialog that records what it was given and answers with `answer`.
fn fake_dialog(
    available: Option<bool>,
    answer: Result<wordcraft_ui_egui::printing::DialogOutcome, String>,
) -> (wordcraft_ui_egui::printing::SystemDialog, Rc<RefCell<Vec<(Vec<u8>, wordcraft_ui_egui::printing::DialogJob)>>>) {
    let shown = Rc::new(RefCell::new(Vec::new()));
    let observed = shown.clone();
    let dialog = wordcraft_ui_egui::printing::SystemDialog {
        available: Box::new(move || available),
        print: Box::new(move |bytes, job| {
            observed.borrow_mut().push((bytes, job));
            let (sender, receiver) = std::sync::mpsc::channel();
            sender.send(answer.clone()).unwrap();
            receiver
        }),
    };
    (dialog, shown)
}

#[test]
fn the_print_button_opens_the_system_dialog_with_the_chosen_pages_and_copies() {
    use wordcraft_ui_egui::printing::{DialogOutcome, dialog_available, print_with_dialog};
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.run("text.insert", json!({"text": "A document to print"})).unwrap();
    let (dialog, shown) = fake_dialog(Some(true), Ok(DialogOutcome::Printed("Office laser".into())));
    app.services.print_dialog = Some(dialog);
    app.ui.print.copies = 4;
    app.ui.print.collate = false;
    assert!(dialog_available(&app));
    print_with_dialog(&mut app).unwrap();
    assert!(app.print_dialog_open());
    assert!(!app.close_requested(), "the window stays open while the print dialog is");
    {
        let shown = shown.borrow();
        assert_eq!(shown.len(), 1);
        assert!(shown[0].0.starts_with(b"%PDF"));
        assert_eq!((shown[0].1.copies, shown[0].1.collate), (4, false), "the dialog opens with the Print page's copies");
        assert_eq!(shown[0].1.accept_label, "Print");
    }
    assert!(print_with_dialog(&mut app).is_err(), "one dialog at a time");
    app.poll_printing(&egui::Context::default());
    assert!(!app.print_dialog_open());
    assert!(app.status_msg.as_ref().unwrap().0.contains("Sent to Office laser"));
}

#[test]
fn cancelling_or_failing_in_the_system_dialog_says_so() {
    use wordcraft_ui_egui::printing::{DialogOutcome, print_with_dialog};
    for (answer, says) in [(Ok(DialogOutcome::Cancelled), "Printing cancelled"), (Err("no paper".to_owned()), "no paper")] {
        let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
        app.run("text.insert", json!({"text": "x"})).unwrap();
        app.services.print_dialog = Some(fake_dialog(Some(true), answer).0);
        print_with_dialog(&mut app).unwrap();
        app.poll_printing(&egui::Context::default());
        assert!(app.status_msg.as_ref().unwrap().0.contains(says), "{says}");
        assert!(!app.print_dialog_open(), "nothing holds the window once the dialog is closed");
    }
}

#[test]
fn commands_never_open_the_system_print_dialog() {
    use wordcraft_ui_egui::printing::{DialogOutcome, dialog_available, print_with_dialog};
    let mut app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    app.run("text.insert", json!({"text": "x"})).unwrap();
    let (dialog, shown) = fake_dialog(Some(true), Ok(DialogOutcome::Cancelled));
    app.services.print_dialog = Some(dialog);
    let _ = app.run("ui.print", json!({}));
    let _ = app.run("ui.printSubmit", json!({}));
    assert!(shown.borrow().is_empty(), "programmatic printing goes to the spooler, never a dialog");
    // While the host is still finding out, or where there is no portal, the Print page keeps
    // its own printer list.
    for available in [None, Some(false)] {
        app.services.print_dialog = Some(fake_dialog(available, Ok(DialogOutcome::Cancelled)).0);
        assert!(!dialog_available(&app));
        assert!(print_with_dialog(&mut app).is_err());
    }
}
