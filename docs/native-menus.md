# Native macOS menus

WordCraft's desktop app installs a native AppKit menu bar alongside the ribbon. The scope is
access to existing common commands, not a replacement interface or a new editing feature set.
Windows, Linux/BSD and the web continue to use the existing ribbon.

| Menu | Existing actions exposed |
|---|---|
| WordCraft | About, Settings, Services, Hide, Hide Others, Show All, Quit |
| File | New, Open, Open Recent, Close, Save, Save As, Export PDF, Print |
| Edit | Undo/Redo, Cut/Copy/Paste/Paste Special, Select All, Find/Find Next/Find Previous/Replace |
| Insert | Page Break, Blank Page, Table dialog, Pictures, Link, Bookmark, Edit Header/Footer, Page Number, Comment, Equation, Symbol |
| Format | Font/Paragraph dialogs, Bold/Italic/Underline/Strikethrough/Subscript/Superscript, font size steps, Clear Formatting, alignment, bullets/numbering, indent/outdent |
| View | Document views, Read Mode, ruler/gridlines/marks, navigation/styles/comments panes, zoom |
| Window | Minimize, Zoom, Bring All to Front; registered with AppKit for its window list |
| Help | Command search; registered with AppKit for native menu search |

## Command routing

`crates/ui-egui/src/native_menu.rs` owns a platform-free menu model. The desktop host injects
`apps/wordcraft/src/mac_menu.rs`, a macOS-only `muda` backend. The engine and other platforms have
no AppKit dependency. Labels come from WordCraft's current interface language and command
registry. View and formatting checkmarks read the same state as the ribbon.

Menu actions use `WordApp::run`, preserving user dialogs and system clipboard handling.
Quit calls the same `file.close` flow as the window's close button: Save, Don't Save or Cancel
for dirty work, and the existing deferred AppKit termination only after the close is accepted.
The native backend does not install a predefined Quit selector that bypasses this flow.

Dialog-open state disables document actions and is checked again when queued actions run.
Read Mode disables insertion and formatting. Focused egui text fields receive their own
copy/cut/paste/select-all/undo/redo events, including in dialogs; these do not edit the document.
Asynchronous native file dialogs block document actions while their answer is pending. Their
Edit items use AppKit's responder-chain selectors so filename/search fields keep native editing;
the egui command items are restored when the dialog closes.

Key equivalents use the effective command keymap, including user assignments and removals.
Only supported command/control shortcuts and function keys can be native equivalents; ordinary typing
and Option characters are left to the editor. Command-H (Replace) and Command-M (Indent) keep
their existing meaning: Hide and Minimize try Control-Command-H/M instead. Quit (Command-Q),
Settings (Command-comma) and the other system fallbacks are only assigned when the keymap leaves
those combinations free. One equivalent is displayed per menu row.

The backend distinguishes key equivalents from mouse/Return activation and preserves the
modifiers actually pressed. It restores swallowed key-downs ahead of their real key-ups,
without inventing key-up events. Clipboard actions go through the focused text field or the
existing UI paste path; a later Command-V release cannot paste the same picture again.

Enabled state, item labels and checkmarks update in place, including a submenu's parent row.
The menu tree is rebuilt when its shape, language or shortcuts change. The ribbon is retained
even if native menu installation fails.

## Provenance and validation

The backend and integration design are adapted from
[PhotoCraft at 67e4b8a](https://github.com/storytold/photocraft/tree/67e4b8a499b55ef728b1da700005382c45e4677d),
including its modal, key-up and text-field clipboard fixes after the original menu PR.
The copied portions and reused translations are covered by the MIT notice in `NOTICE`.

Run `cargo xtask ci` for the repository gates. Focused checks are:

```sh
cargo test -p wordcraft-ui-egui --lib native_menu
cargo test -p wordcraft --test mac_menu_appkit
```

The UI tests cover menu structure, existing command/dialog routing, selection state, effective
shortcuts, modal races, text fields, text/picture paste and saving/cancelling Quit. The AppKit
test runs on the main thread without simulated desktop keystrokes; it reads real NSMenu state,
performs key equivalents and click/Return actions, checks submenu restoration, language changes
and save/cancel, and verifies native editing selectors during file dialogs. It is a no-op on other
platforms. `ui_shot` can separately check that the ribbon
still renders; an offscreen screenshot does not verify the system menu bar.

The menu's new strings have entries in the existing language catalogs, but native-speaker
review is still needed. VoiceOver, Services providers and a second physical Mac/keyboard layout
are not covered by these automated tests. Overall Word parity and readiness estimates are
unchanged.
