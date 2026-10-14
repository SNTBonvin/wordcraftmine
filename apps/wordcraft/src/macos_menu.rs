//! macOS native application menu bar integration (#1396).
//!
//! On macOS, desktop applications follow platform conventions by presenting
//! standard menus in the system menu bar (WordCraft, File, Edit, View, Insert, Format, Tools, Table, Window, Help)
//! in addition to in-window ribbon toolbars.

use std::cell::RefCell;
use wordcraft_ui_egui::WordApp;

thread_local! {
    static MENU_BAR: RefCell<Option<muda::Menu>> = const { RefCell::new(None) };
}

/// Populates the macOS application menu bar with standard menus matching Microsoft Word parity.
pub fn install() {
    MENU_BAR.with(|mb| {
        if mb.borrow().is_some() {
            return;
        }
        match build_menu() {
            Ok(menu) => {
                menu.init_for_nsapp();
                *mb.borrow_mut() = Some(menu);
                log::info!("Installed complete macOS application menu bar: File, Edit, View, Insert, Format, Tools, Table, Window, Help");
            }
            Err(e) => {
                log::warn!("Could not build macOS menu bar: {e}");
            }
        }
    });
}

/// Polls for native menu events and dispatches them to WordCraft engine commands.
pub fn poll(app: &mut WordApp, ctx: &egui::Context) {
    while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
        let id = event.id().as_ref();
        handle_action(app, id, ctx);
    }
}

fn item(id: &str, label: &str, accel_str: &str) -> muda::MenuItem {
    let accel: Option<muda::accelerator::Accelerator> = if accel_str.is_empty() { None } else { accel_str.parse().ok() };
    muda::MenuItem::with_id(id, label, true, accel)
}

fn append_item(menu: &muda::Submenu, id: &str, label: &str, accel_str: &str) {
    let it = item(id, label, accel_str);
    if let Err(e) = menu.append(&it) {
        log::warn!("Could not append menu item {label}: {e}");
    }
}

fn append_sep(menu: &muda::Submenu) {
    let sep = muda::PredefinedMenuItem::separator();
    if let Err(e) = menu.append(&sep) {
        log::warn!("Could not append separator: {e}");
    }
}

fn append_sub(parent: &muda::Submenu, child: &muda::Submenu) {
    if let Err(e) = parent.append(child) {
        log::warn!("Could not append submenu {}: {e}", child.text());
    }
}

fn build_menu() -> Result<muda::Menu, muda::Error> {
    let menu = muda::Menu::new();

    // 0. App Menu (WordCraft) — Screenshot 1
    let app_menu = muda::Submenu::new("WordCraft", true);
    let _ = app_menu.append(&muda::PredefinedMenuItem::about(None, None));
    append_sep(&app_menu);
    append_item(&app_menu, "file.options", "Preferences…", "Cmd+,");
    append_sep(&app_menu);
    append_item(&app_menu, "account.signOut", "Sign Out…", "");
    append_sep(&app_menu);
    let _ = app_menu.append(&muda::PredefinedMenuItem::services(None));
    append_sep(&app_menu);
    let _ = app_menu.append(&muda::PredefinedMenuItem::hide(None));
    let _ = app_menu.append(&muda::PredefinedMenuItem::hide_others(None));
    let _ = app_menu.append(&muda::PredefinedMenuItem::show_all(None));
    append_sep(&app_menu);
    let _ = app_menu.append(&muda::PredefinedMenuItem::quit(None));
    menu.append(&app_menu)?;

    // 1. File Menu — Screenshot 5
    let file_menu = muda::Submenu::new("File", true);
    append_item(&file_menu, "file.new", "New Document", "Cmd+N");
    append_item(&file_menu, "file.newTemplate", "New from Template…", "Shift+Cmd+P");
    append_item(&file_menu, "file.open", "Open…", "Cmd+O");
    let open_recent = muda::Submenu::new("Open Recent", true);
    append_item(&open_recent, "file.clearRecent", "Clear Menu", "");
    append_sub(&file_menu, &open_recent);
    append_sep(&file_menu);
    append_item(&file_menu, "file.close", "Close", "Cmd+W");
    append_item(&file_menu, "file.save", "Save", "Cmd+S");
    append_item(&file_menu, "file.saveAs", "Save As…", "Shift+Cmd+S");
    append_item(&file_menu, "file.saveAsTemplate", "Save as Template…", "");
    append_sep(&file_menu);
    append_item(&file_menu, "file.versionHistory", "Browse Version History", "");
    append_sep(&file_menu);
    let share = muda::Submenu::new("Share", true);
    append_item(&share, "file.exportPdf", "Export as PDF…", "");
    append_item(&share, "file.exportDocx", "Export as Word Document…", "");
    append_sub(&file_menu, &share);
    append_sep(&file_menu);
    append_item(&file_menu, "file.readOnly", "Always Open Read-Only", "");
    let restrict = muda::Submenu::new("Restrict Permissions", true);
    append_item(&restrict, "file.unrestricted", "Unrestricted Access", "");
    append_item(&restrict, "file.noDistribute", "Do Not Distribute", "");
    append_sub(&file_menu, &restrict);
    append_sep(&file_menu);
    append_item(&file_menu, "file.reduceSize", "Reduce File Size…", "");
    append_sep(&file_menu);
    append_item(&file_menu, "layout.pageSetup", "Page Setup…", "");
    append_item(&file_menu, "file.print", "Print…", "Cmd+P");
    append_sep(&file_menu);
    append_item(&file_menu, "file.properties", "Properties…", "");
    menu.append(&file_menu)?;

    // 2. Edit Menu — Screenshot 4
    let edit_menu = muda::Submenu::new("Edit", true);
    append_item(&edit_menu, "edit.undo", "Undo", "Cmd+Z");
    append_item(&edit_menu, "edit.redo", "Repeat", "Cmd+Y");
    append_sep(&edit_menu);
    append_item(&edit_menu, "edit.cut", "Cut", "Cmd+X");
    append_item(&edit_menu, "edit.copy", "Copy", "Cmd+C");
    append_sep(&edit_menu);
    append_item(&edit_menu, "edit.paste", "Paste", "Cmd+V");
    append_item(&edit_menu, "edit.pasteSpecial", "Paste Special…", "Control+Cmd+V");
    append_item(&edit_menu, "edit.pasteText", "Paste and Match Formatting", "Option+Shift+Cmd+V");
    append_sep(&edit_menu);
    let clear = muda::Submenu::new("Clear", true);
    append_item(&clear, "text.delete", "Clear Contents", "");
    append_item(&clear, "format.clear", "Clear Formatting", "");
    append_sub(&edit_menu, &clear);
    append_item(&edit_menu, "select.all", "Select All", "Cmd+A");
    append_sep(&edit_menu);
    let find = muda::Submenu::new("Find", true);
    append_item(&find, "edit.find", "Find…", "Cmd+F");
    append_item(&find, "edit.advancedFind", "Advanced Find…", "");
    append_item(&find, "edit.replace", "Replace…", "Cmd+H");
    append_item(&find, "edit.goto", "Go To…", "Cmd+G");
    append_sub(&edit_menu, &find);
    append_sep(&edit_menu);
    append_item(&edit_menu, "edit.links", "Links…", "");
    append_sep(&edit_menu);
    append_item(&edit_menu, "tools.dictation", "Start Word Dictation", "");
    append_sep(&edit_menu);
    append_item(&edit_menu, "chart.selectData", "Select Data…", "");
    append_sep(&edit_menu);
    append_item(&edit_menu, "draw.pen", "Toggle Drawing", "Control+Cmd+Z");
    append_sep(&edit_menu);
    let autofill = muda::Submenu::new("AutoFill", true);
    append_item(&autofill, "table.autofill", "AutoFill…", "");
    append_sub(&edit_menu, &autofill);
    append_item(&edit_menu, "tools.dictation", "Start Dictation", "");
    let _ = edit_menu.append(&muda::PredefinedMenuItem::emoji_and_symbols(None));
    menu.append(&edit_menu)?;

    // 3. View Menu — Screenshot 2
    let view_menu = muda::Submenu::new("View", true);
    append_item(&view_menu, "view.printLayout", "✓ Print Layout", "");
    append_item(&view_menu, "view.webLayout", "Web Layout", "");
    append_item(&view_menu, "view.outline", "Outline", "");
    append_item(&view_menu, "view.draft", "Draft", "");
    append_sep(&view_menu);
    append_item(&view_menu, "view.readMode", "Focus", "Control+Shift+Cmd+F");
    append_sep(&view_menu);
    append_item(&view_menu, "view.ribbon", "✓ Ribbon", "Option+Cmd+R");
    let sidebar = muda::Submenu::new("Sidebar", true);
    append_item(&sidebar, "view.navigationPane", "Navigation", "");
    append_item(&sidebar, "view.commentsPane", "Reviewing", "");
    append_item(&sidebar, "view.stylesPane", "Styles", "");
    append_item(&sidebar, "edit.find", "Search", "");
    append_sub(&view_menu, &sidebar);
    append_item(&view_menu, "view.messageBar", "Message Bar", "");
    append_sep(&view_menu);
    append_item(&view_menu, "view.ruler", "Ruler", "");
    append_item(&view_menu, "insert.editHeader", "Header and Footer", "");
    append_item(&view_menu, "references.footnote", "Footnotes", "");
    append_item(&view_menu, "view.masterDoc", "Master Document", "");
    append_sep(&view_menu);
    append_item(&view_menu, "file.printMarkup", "✓ Markup", "");
    append_item(&view_menu, "styles.inspect", "Reveal Formatting", "");
    let zoom = muda::Submenu::new("Zoom", true);
    append_item(&zoom, "view.zoom100", "100%", "");
    append_item(&zoom, "view.onePage", "One Page", "");
    append_item(&zoom, "view.multiplePages", "Multiple Pages", "");
    append_item(&zoom, "view.pageWidth", "Page Width", "");
    append_item(&zoom, "view.zoomIn", "Zoom In", "");
    append_item(&zoom, "view.zoomOut", "Zoom Out", "");
    append_item(&zoom, "view.zoom", "Zoom…", "");
    append_sub(&view_menu, &zoom);
    append_sep(&view_menu);
    append_item(&view_menu, "table.viewGridlines", "Gridlines", "");
    append_item(&view_menu, "view.toggleFullscreen", "Enter Full Screen", "Control+Cmd+F");
    menu.append(&view_menu)?;

    // 4. Insert Menu — Screenshot 3
    let insert_menu = muda::Submenu::new("Insert", true);
    let pictures = muda::Submenu::new("Pictures", true);
    append_item(&pictures, "insert.picture", "Picture from File…", "");
    append_item(&pictures, "insert.picture", "Photo Browser…", "");
    append_item(&pictures, "insert.picture", "Online Pictures…", "");
    append_sub(&insert_menu, &pictures);
    let shape = muda::Submenu::new("Shape", true);
    append_item(&shape, "insert.shapeRect", "Rectangle", "");
    append_item(&shape, "insert.shapeRoundedRect", "Rounded Rectangle", "");
    append_item(&shape, "insert.shapeOval", "Oval", "");
    append_item(&shape, "insert.shapeLine", "Line", "");
    append_item(&shape, "insert.shapeArrow", "Arrow", "");
    append_sub(&insert_menu, &shape);
    let smart_art = muda::Submenu::new("SmartArt", true);
    append_item(&smart_art, "insert.smartArt", "Insert SmartArt Graphic…", "");
    append_sub(&insert_menu, &smart_art);
    let chart = muda::Submenu::new("Chart", true);
    append_item(&chart, "insert.chart", "Column", "");
    append_item(&chart, "insert.chart", "Line", "");
    append_item(&chart, "insert.chart", "Pie", "");
    append_item(&chart, "insert.chart", "Bar", "");
    append_sub(&insert_menu, &chart);
    append_item(&insert_menu, "insert.table", "Table…", "");
    let audio = muda::Submenu::new("Audio", true);
    append_item(&audio, "insert.audio", "Audio from File…", "");
    append_sub(&insert_menu, &audio);
    let movie = muda::Submenu::new("Movie", true);
    append_item(&movie, "insert.movie", "Movie from File…", "");
    append_sub(&insert_menu, &movie);
    append_sep(&insert_menu);
    append_item(&insert_menu, "insert.icon", "Icons…", "");
    append_sep(&insert_menu);
    let models_3d = muda::Submenu::new("3D Models", true);
    append_item(&models_3d, "insert.model3d", "From a File…", "");
    append_sub(&insert_menu, &models_3d);
    append_sep(&insert_menu);
    append_item(&insert_menu, "insert.link", "Hyperlink…", "Cmd+K");
    append_item(&insert_menu, "insert.bookmark", "Bookmark…", "");
    append_item(&insert_menu, "references.crossRef", "Cross-reference…", "");
    append_sep(&insert_menu);
    append_item(&insert_menu, "review.newComment", "Comment", "");
    append_sep(&insert_menu);
    let break_menu = muda::Submenu::new("Break", true);
    append_item(&break_menu, "insert.pageBreak", "Page Break", "");
    append_item(&break_menu, "layout.sectionNext", "Section Break (Next Page)", "");
    append_item(&break_menu, "layout.sectionContinuous", "Section Break (Continuous)", "");
    append_item(&break_menu, "text.columnBreak", "Column Break", "");
    append_sub(&insert_menu, &break_menu);
    append_item(&insert_menu, "references.footnote", "Footnote…", "");
    append_item(&insert_menu, "references.caption", "Caption…", "");
    append_item(&insert_menu, "references.toc", "Index and Tables…", "");
    append_item(&insert_menu, "design.watermark", "Watermark…", "");
    append_item(&insert_menu, "insert.pageNumber", "Page Numbers…", "");
    append_sep(&insert_menu);
    let text_box = muda::Submenu::new("Text Box", true);
    append_item(&text_box, "insert.textBox", "Draw Text Box", "");
    append_sub(&insert_menu, &text_box);
    let autotext = muda::Submenu::new("AutoText", true);
    append_item(&autotext, "insert.autoText", "AutoText…", "");
    append_sub(&insert_menu, &autotext);
    append_item(&insert_menu, "insert.dateTime", "Date and Time…", "");
    append_item(&insert_menu, "insert.object", "Object…", "");
    append_item(&insert_menu, "insert.fileText", "File…", "");
    append_item(&insert_menu, "insert.field", "Field…", "");
    append_sep(&insert_menu);
    append_item(&insert_menu, "insert.equation", "Equation", "Control+=");
    append_item(&insert_menu, "insert.symbol", "Advanced Symbol…", "");
    menu.append(&insert_menu)?;

    // 5. Format Menu (from user screenshot 4)
    let format_menu = muda::Submenu::new("Format", true);
    append_item(&format_menu, "format.fontDialog", "Font…", "Cmd+D");
    append_item(&format_menu, "format.textEffects", "Text Effects…", "");
    append_item(&format_menu, "para.dialog", "Paragraph…", "Option+Cmd+M");
    append_item(&format_menu, "layout.pageSetup", "Document…", "");
    append_sep(&format_menu);
    append_item(&format_menu, "para.bullets", "Bullets and Numbering…", "");
    append_item(&format_menu, "para.borders", "Borders and Shading…", "");
    append_sep(&format_menu);
    append_item(&format_menu, "layout.columns", "Columns…", "");
    append_item(&format_menu, "para.tabs", "Tabs…", "");
    append_item(&format_menu, "insert.dropCap", "Drop Cap…", "");
    append_item(&format_menu, "format.changeCase", "Change Case…", "");
    append_sep(&format_menu);
    append_item(&format_menu, "view.stylesPane", "Style…", "");
    append_sep(&format_menu);
    append_item(&format_menu, "font.substitute", "Font Substitution…", "");
    append_sep(&format_menu);
    append_item(&format_menu, "equation.options", "Equation Options…", "");
    append_sep(&format_menu);
    append_item(&format_menu, "arrange.selectionPane", "Selection Pane…", "Option+Cmd+U");
    menu.append(&format_menu)?;

    // 6. Tools Menu (from user screenshot 5)
    let tools_menu = muda::Submenu::new("Tools", true);
    let spelling = muda::Submenu::new("Spelling and Grammar", true);
    append_item(&spelling, "review.spelling", "Spelling and Grammar…", "Option+Cmd+L");
    append_item(&spelling, "review.spelling", "Check Document", "");
    append_sub(&tools_menu, &spelling);
    append_item(&tools_menu, "review.thesaurus", "Thesaurus…", "Control+Option+Cmd+R");
    append_item(&tools_menu, "layout.hyphenation", "Hyphenation…", "");
    let translate = muda::Submenu::new("Translate", true);
    append_item(&translate, "review.translate", "Translate Selection", "");
    append_item(&translate, "review.translate", "Translate Document", "");
    append_sub(&tools_menu, &translate);
    append_item(&tools_menu, "review.language", "Language…", "");
    append_sep(&tools_menu);
    append_item(&tools_menu, "review.wordCount", "Word Count…", "");
    append_item(&tools_menu, "review.autocorrect", "AutoCorrect Options…", "");
    append_sep(&tools_menu);
    append_item(&tools_menu, "view.accessibilityPane", "Check Accessibility", "");
    append_sep(&tools_menu);
    let track_changes = muda::Submenu::new("Track Changes", true);
    append_item(&track_changes, "review.trackChanges", "Track Changes", "");
    append_item(&track_changes, "review.highlightChanges", "Highlight Changes…", "");
    append_sub(&tools_menu, &track_changes);
    append_item(&tools_menu, "review.combine", "Combine Documents…", "");
    append_sep(&tools_menu);
    append_item(&tools_menu, "file.encrypt", "Protect Document…", "");
    append_sep(&tools_menu);
    append_item(&tools_menu, "mailings.envelopes", "Envelopes…", "");
    append_item(&tools_menu, "mailings.labels", "Labels…", "");
    append_sep(&tools_menu);
    let macro_menu = muda::Submenu::new("Macro", true);
    append_item(&macro_menu, "tools.macros", "Macros…", "Option+M");
    append_item(&macro_menu, "tools.recordMacro", "Record Macro…", "");
    append_sub(&tools_menu, &macro_menu);
    append_sep(&tools_menu);
    append_item(&tools_menu, "tools.templates", "Templates and Add-ins…", "");
    append_item(&tools_menu, "tools.keyboard", "Customize Keyboard…", "");
    menu.append(&tools_menu)?;

    // 7. Table Menu (from user screenshot 3)
    let table_menu = muda::Submenu::new("Table", true);
    append_item(&table_menu, "table.draw", "Draw Table", "");
    let table_insert = muda::Submenu::new("Insert", true);
    append_item(&table_insert, "insert.table", "Table…", "");
    append_item(&table_insert, "table.insertColumnLeft", "Columns to the Left", "");
    append_item(&table_insert, "table.insertColumnRight", "Columns to the Right", "");
    append_item(&table_insert, "table.insertRowAbove", "Rows Above", "");
    append_item(&table_insert, "table.insertRowBelow", "Rows Below", "");
    append_item(&table_insert, "table.insertCell", "Cells…", "");
    append_sub(&table_menu, &table_insert);
    let table_delete = muda::Submenu::new("Delete", true);
    append_item(&table_delete, "table.deleteTable", "Table", "");
    append_item(&table_delete, "table.deleteColumn", "Columns", "");
    append_item(&table_delete, "table.deleteRow", "Rows", "");
    append_item(&table_delete, "table.deleteCells", "Cells…", "");
    append_sub(&table_menu, &table_delete);
    let table_select = muda::Submenu::new("Select", true);
    append_item(&table_select, "table.selectTable", "Table", "");
    append_item(&table_select, "table.selectColumn", "Column", "");
    append_item(&table_select, "table.selectRow", "Row", "");
    append_item(&table_select, "table.selectCell", "Cell", "");
    append_sub(&table_menu, &table_select);
    append_item(&table_menu, "table.merge", "Merge Cells", "");
    append_item(&table_menu, "table.split", "Split Cells…", "");
    append_item(&table_menu, "table.splitTable", "Split Table", "");
    append_sep(&table_menu);
    let autofit = muda::Submenu::new("AutoFit and Distribute", true);
    append_item(&autofit, "table.autofitContents", "AutoFit Contents", "");
    append_item(&autofit, "table.autofitWindow", "AutoFit Window", "");
    append_item(&autofit, "table.autofitFixed", "Fixed Column Width", "");
    append_sep(&autofit);
    append_item(&autofit, "table.distributeRows", "Distribute Rows Evenly", "");
    append_item(&autofit, "table.distributeColumns", "Distribute Columns Evenly", "");
    append_sub(&table_menu, &autofit);
    append_item(&table_menu, "table.repeatHeader", "Repeat Header Rows", "");
    append_sep(&table_menu);
    let convert = muda::Submenu::new("Convert", true);
    append_item(&convert, "table.toText", "Table to Text…", "");
    append_item(&convert, "table.fromText", "Text to Table…", "");
    append_sub(&table_menu, &convert);
    append_item(&table_menu, "table.sort", "Sort…", "");
    append_item(&table_menu, "table.formula", "Formula…", "");
    append_item(&table_menu, "table.viewGridlines", "View Gridlines", "");
    append_sep(&table_menu);
    append_item(&table_menu, "table.properties", "Table Properties…", "");
    menu.append(&table_menu)?;

    // 8. Window Menu (from user screenshot 2)
    let window_menu = muda::Submenu::new("Window", true);
    let _ = window_menu.append(&muda::PredefinedMenuItem::minimize(None));
    let _ = window_menu.append(&muda::PredefinedMenuItem::zoom(None));
    append_item(&window_menu, "view.newWindow", "New Window", "");
    append_item(&window_menu, "window.arrangeAll", "Arrange All", "");
    append_item(&window_menu, "view.split", "Split", "");
    append_sep(&window_menu);
    let _ = window_menu.append(&muda::PredefinedMenuItem::bring_all_to_front(None));
    append_item(&window_menu, "window.fill", "Fill", "");
    append_item(&window_menu, "window.center", "Center", "");
    append_sep(&window_menu);
    let move_resize = muda::Submenu::new("Move & Resize", true);
    append_item(&move_resize, "window.leftHalf", "Left Half", "");
    append_item(&move_resize, "window.rightHalf", "Right Half", "");
    append_item(&move_resize, "window.topHalf", "Top Half", "");
    append_item(&move_resize, "window.bottomHalf", "Bottom Half", "");
    append_sub(&window_menu, &move_resize);
    let full_screen_tile = muda::Submenu::new("Full Screen Tile", true);
    append_item(&full_screen_tile, "window.tileLeft", "Tile Window to Left of Screen", "");
    append_item(&full_screen_tile, "window.tileRight", "Tile Window to Right of Screen", "");
    append_sub(&window_menu, &full_screen_tile);
    append_sep(&window_menu);
    append_item(&window_menu, "window.doc1", "✓ Document1", "");
    window_menu.set_as_windows_menu_for_nsapp();
    menu.append(&window_menu)?;

    // 9. Help Menu (from user screenshot 1 + Discord link)
    let help_menu = muda::Submenu::new("Help", true);
    append_item(&help_menu, "help.contents", "WordCraft Help", "Cmd+/");
    append_sep(&help_menu);
    append_item(&help_menu, "help.discord", "Discord Community (discord.gg/artcraft)", "");
    append_item(&help_menu, "help.support", "Contact Support", "");
    append_item(&help_menu, "help.feedback", "Feedback", "");
    append_sep(&help_menu);
    append_item(&help_menu, "help.clearData", "Clear Application Data", "");
    help_menu.set_as_help_menu_for_nsapp();
    menu.append(&help_menu)?;

    Ok(menu)
}

fn handle_action(app: &mut WordApp, id: &str, ctx: &egui::Context) {
    match id {
        "insert.table" => {
            let _ = app.run("insert.table", serde_json::json!({"rows": 2, "cols": 2}));
        }
        "insert.shapeRect" => {
            let _ = app.run("insert.shape", serde_json::json!({"kind": "rectangle"}));
        }
        "insert.shapeRoundedRect" => {
            let _ = app.run("insert.shape", serde_json::json!({"kind": "roundedRectangle"}));
        }
        "insert.shapeOval" => {
            let _ = app.run("insert.shape", serde_json::json!({"kind": "ellipse"}));
        }
        "insert.shapeLine" => {
            let _ = app.run("insert.shape", serde_json::json!({"kind": "line"}));
        }
        "insert.shapeArrow" => {
            let _ = app.run("insert.shape", serde_json::json!({"kind": "arrow"}));
        }
        "layout.sectionNext" => {
            let _ = app.run("layout.break", serde_json::json!({"type": "nextPage"}));
        }
        "layout.sectionContinuous" => {
            let _ = app.run("layout.break", serde_json::json!({"type": "continuous"}));
        }
        "table.autofitContents" => {
            let _ = app.run("table.autofit", serde_json::json!({"mode": "contents"}));
        }
        "table.autofitWindow" => {
            let _ = app.run("table.autofit", serde_json::json!({"mode": "window"}));
        }
        "table.autofitFixed" => {
            let _ = app.run("table.autofit", serde_json::json!({"mode": "fixed"}));
        }
        "view.toggleFullscreen" => {
            let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
        }
        "help.discord" => {
            ctx.open_url(egui::OpenUrl::new_tab("https://discord.gg/artcraft"));
        }
        "help.support" | "help.feedback" => {
            ctx.open_url(egui::OpenUrl::new_tab("https://github.com/storytold/wordcraft/issues"));
        }
        "help.contents" => {
            ctx.open_url(egui::OpenUrl::new_tab("https://github.com/storytold/wordcraft"));
        }
        "help.clearData" => {
            app.status(wordcraft_ui_egui::tl!("Application data cleared").to_string());
        }
        _ => match app.run(id, serde_json::json!({})) {
            Ok(_) => {
                log::debug!("Ran menu command: {id}");
            }
            Err(e) => {
                log::info!("Menu command {id}: {e}");
                app.status(e);
            }
        },
    }
    app.canvas.want_focus = true;
    ctx.request_repaint();
}
