# WordCraft GitHub Issues Analysis

This document provides a detailed analysis of open GitHub issues for WordCraft, including root cause analysis and planned fixes.


## Issue 1246: Web Save in cross-origin iframe
- **Summary**: Save does nothing in a cross-origin iframe on Chromium.
- **Root Cause**: In `apps/wordcraft-web/src/web.rs`, `trigger_save_file_picker` returns `Ok(())` because the `showSaveFilePicker` call returns a promise. However, in cross-origin iframes, Chromium rejects this promise asynchronously with a `SecurityError`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Modify the `spawn_local` block in `trigger_save_file_picker` to treat `SecurityError` (and other non-`AbortError` rejections) as a failure that triggers `trigger_anchor_download`.
- **Risk**: Low.
## Issue 1246: Web Save in cross-origin iframe
- **Summary**: Save does nothing in a cross-origin iframe on Chromium.
- **Root Cause**: In `apps/wordcraft-web/src/web.rs`, `trigger_save_file_picker` returns `Ok(())` because the `showSaveFilePicker` call returns a promise. However, in cross-origin iframes, Chromium rejects this promise asynchronously with a `SecurityError`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Modify the `spawn_local` block in `trigger_save_file_picker` to treat `SecurityError` (and other non-`AbortError` rejections) as a failure that triggers `trigger_anchor_download`.
- **Risk**: Low.

## Issue 1243: Copy rich text formatting
- **Summary**: Copying formatted text loses bold/italics when pasted into other apps.
- **Root Cause**: The `edit.copy` command in `crates/engine/src/cmd/edit.rs` only updates internal state and doesn't push rich text formats (HTML/RTF) to the system clipboard.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Implement system clipboard write for rich text (HTML/RTF) using OS-specific APIs.
- **Risk**: Medium.

## Issue 1242: Table Height/Width changes apply only to last row/column
- **Summary**: Height/Width changes apply only to the last row/column when the entire table is selected.
- **Root Cause**: `table.rowHeight` and `table.columnWidth` in `crates/engine/src/cmd/table.rs` only apply changes to the caret cell's row/column.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Update these commands to iterate over the current selection and apply changes to all selected rows/columns.
- **Risk**: Low.

## Issue 1241: RTL text direction only works for numbers
- **Summary**: Right-to-left text direction only seems to work for numbers, not letters.
- **Root Cause**: RTL support in `crates/layout/src/para.rs` and `crates/layout/src/display.rs` is partial and lacks full bidi reordering for letters.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Review `unicode-bidi` integration and ensure visual ordering of letters in RTL paragraphs is correctly applied.
- **Risk**: High.

## Issue 1240: Save dialog window too tall on XWayland/niri
- **Summary**: Save dialog window is taller than screen when running via XWayland on niri.
- **Root Cause**: External issue with `xdg-desktop-portal-gtk` and XWayland on niri.
- **Status**: `NOT REPRODUCIBLE / EXTERNAL`
- **Planned Fix**: No internal fix planned.
- **Risk**: N/A.

## Issue 1238: Styles selector empty with sample docx
- **Summary**: Styles selector becomes empty when opening a specific sample docx file.
- **Root Cause**: Likely failure in styles resolution or filtering in `crates/ui-egui/src/panes.rs`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Investigate styles resolution in `crates/doc` and filtering in the Styles pane.
- **Risk**: Low.

## Issue 1237: Ctrl+ / Ctrl- for zooming
- **Summary**: Feature request for Ctrl+ / Ctrl- keyboard shortcuts for zooming.
- **Root Cause**: Support for mouse-wheel zoom exists, but keyboard shortcuts are missing in `crates/ui-egui/src/keys.rs`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Add keyboard event handlers for `Ctrl+` and `Ctrl-` in `crates/ui-egui/src/keys.rs`.
- **Risk**: Low.

## Issue 1236: Bookmark Delete doesn't exist
- **Summary**: Feature request: Bookmark > Delete does not exist.
- **Root Cause**: Command not implemented in `crates/engine/src/cmd/`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Implement `bookmark.delete` command.
- **Risk**: Low.

## Issue 1234: Link > Place in This Document (does not exist)
- **Summary**: Feature request: Link to "Place in This Document" does not exist.
- **Root Cause**: Command not implemented.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Implement `insert.linkInternal` command.
- **Risk**: Low.

## Issue 1233: Installing error
- **Summary**: User reports an installing error (images only).
- **Root Cause**: Undetermined.
- **Status**: `NOT REPRODUCIBLE`
- **Planned Fix**: None.
- **Risk**: N/A.
## Issue 1232: WordCraft quits when attempting to display a large document's outline
- **Summary**: App crashes with a `wgpu` Validation Error when opening the Outline view of a large document.
- **Root Cause**: The Outline view likely attempts to render a single large texture for the entire outline, exceeding GPU limits.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Implement virtualized rendering or texture tiling for the Outline view.
- **Risk**: Medium.

## Issue 1230: Object names and certain automatic numbering features are missing
- **Summary**: Missing "Insert Caption" feature and advanced page numbering options.
- **Root Cause**: Features not yet implemented in the engine or UI layers.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Add `insert.caption` command and implement automatic counter logic in the document model.
- **Risk**: Low.

## Issue 1229: Text Box Feature bug view - [Mac OS]
- **Summary**: Outlines and text disappear in text boxes on macOS.
- **Status**: `OBSOLETE`
- **Reason**: macOS-specific issue.

## Issue 1228: Tab row: Comments / Editing / Share buttons overlap ribbon tabs
- **Summary**: When the window is narrow, the right-side action buttons overlap the ribbon tabs.
- **Root Cause**: Tab row layout in `crates/ui-egui/src/ribbon.rs` doesn't account for the width of action buttons.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Calculate action button width first and implement a degradation strategy (icon-only -> trimmed labels -> overflow menu).
- **Risk**: Low.

## Issue 1227: Feature request: Create with templates
- **Summary**: Request for more common templates (MLA, APA, etc.) and searchability.
- **Root Cause**: Only a few templates exist in `crates/engine/src/sample.rs`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Add more template definitions and implement a search filter in the templates dialog.
- **Risk**: Low.

## Issue 1225: Proofing: help users get a Hunspell dictionary
- **Summary**: Users struggle to install Hunspell dictionaries.
- **Root Cause**: No easy way to populate the local dictionary folder.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Add "Get dictionary..." and "Open dictionaries folder" buttons to the Proofing options dialog.
- **Risk**: Low.

## Issue 1223: [Bug] Zoom via ctrl+middle mouse causes scroll and zoom at the same time
- **Summary**: Zooming with Ctrl+Middle Mouse also triggers scrolling.
- **Root Cause**: Middle mouse events handled by both zoom and scroll logic in `crates/ui-egui/src/canvas.rs` or `crates/ui-egui/src/scroll.rs`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Ensure Ctrl+middle-mouse events are captured exclusively by the zoom handler.
- **Risk**: Low.

## Issue 1218: Ctrl+Q clears paragraph formatting but also opens a save dialog
- **Summary**: Ctrl+Q is bound to "Clear Paragraph Formatting" but also triggers the app's Quit action.
- **Root Cause**: Shortcut conflict between `para.reset` and the framework's default Quit action.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Override the global Quit shortcut or ensure `para.reset` consumes the event.
- **Risk**: Low.

## Issue 1215: Bug in scrollbar when dragged
- **Summary**: Scrollbar sticks when the cursor hits the edge of the screen.
- **Root Cause**: Likely an issue with how `egui` or the wrapper handles mouse drag events when they leave window bounds.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Ensure dragging continues as long as the mouse button is held.
- **Risk**: Low.

## Issue 1213: Can't type anything under Xwayland in GNOME 50 + arboard not supporting wayland
- **Summary**: Users on Wayland cannot type due to `arboard` clipboard initialization failure.
- **Root Cause**: `arboard` has limited Wayland support, which may be blocking input.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Improve fallback mechanism in `apps/wordcraft/src/clipboard.rs` or implement a native Wayland bridge.
- **Risk**: Medium.
## Issue 1212: Cannot drag n drop picture
- **Summary**: User reports inability to drag and drop files from file manager to WordCraft on Ubuntu.
- **Root Cause**: Codebase implements dropped file handling in `crates/ui-egui/src/lib.rs`, but may have platform-specific gaps.
- **Status**: `ALREADY FIXED`
- **Planned Fix**: None needed.
- **Risk**: None.

## Issue 1211: Print Option Doesn't show my printers
- **Summary**: Native spooling not showing printers on Linux.
- **Root Cause**: `craft_print` discovery may be failing or UI doesn't distinguish native vs PDF.
- **Status**: `NEEDS FIX / NEEDS DECISION`
- **Planned Fix**: Verify `craft_print` discovery on Linux and clarify UI options.
- **Risk**: Low.

## Issue 1208: Laggy Cursor
- **Summary**: Cursor trails behind typing.
- **Root Cause**: Caret gliding in `crates/ui-egui/src/caret_glide.rs` may be too slow.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Review and potentially reduce `GLIDE_SECS` or make it configurable.
- **Risk**: Low.

## Issue 1207: Document Tabs & importing doc with tabs fails
- **Summary**: Google Docs "Tabs" are not supported in import or model.
- **Root Cause**: Multi-tab support is not in `crates/doc` or OOXML reader.
- **Status**: `NEEDS FIX (Import Bug) / NEEDS DECISION (Feature)`
- **Planned Fix**: Investigate Google Docs tab export and evaluate architectural impact of adding tabs to `Session`.
- **Risk**: Medium.

## Issue 1206: Margins, Orientation, Size, and Columns dialog missing icon and active setting
- **Summary**: UI menus lack checkmarks and icons for active settings.
- **Root Cause**: `mi` helper used instead of `mi_check` in `crates/ui-egui/src/ribbon.rs`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Replace `mi` with `mi_check` and add icons from `crates/ui-egui/src/icons.rs`.
- **Risk**: Low.

## Issues 1201, 1200, 1199, 1198, 1197
- **Status**: `NOT ANALYZED`
- **Reason**: GitHub API rate limits.
## Issue 1196: Provide pre-built Docker images on GHCR or Docker Hub
- **Summary**: Request for pre-built Docker images for easier deployment.
- **Root Cause**: Infrastructure gap; images must be built manually by users.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Set up a GitHub Action to build and push the WordCraft Docker image to GHCR.
- **Risk**: Low.

## Issue 1192: Proposal: LawCraft, legal drafting tools for WordCraft
- **Summary**: Proposal for a specialized "LawCraft" toolset for legal drafting.
- **Root Cause**: Feature request for specialized legal tools.
- **Status**: `NEEDS DECISION`
- **Planned Fix**: Decide if this should be an official extension or a community project.
- **Risk**: Low.

## Issue 1189: Ten catalogues are ~397 keys behind the leaders
- **Summary**: Localization gap in several language catalogues (e.g., pt-BR).
- **Root Cause**: Missing translation keys in specific catalogues.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Coordinate translations and update i18n tests to enforce catalogue synchronization.
- **Risk**: Low.

## Issue 1188: Bug Ctrl+A
- **Summary**: `Ctrl+A` (Select All) automatically scrolls to the bottom of the page.
- **Root Cause**: UI layer likely forces viewport scroll to the focus (end of document) during selection.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Ensure `select.all` does not force viewport scroll if the cursor was elsewhere.
- **Risk**: Low.

## Issue 1186: Flickering after update to 0.6.1
- **Summary**: Vague report of "flickering" after update.
- **Root Cause**: Undetermined; lack of details on OS/Hardware.
- **Status**: `NOT REPRODUCIBLE`
- **Planned Fix**: Request more information from the user.
- **Risk**: N/A.

## Issue 1183: Feature Request: KeyTips for Table Design and Table Format Ribbon
- **Summary**: Request for MS Word-style Alt-key navigation (KeyTips) for Table ribbons.
- **Root Cause**: KeyTips for Table ribbons not implemented in `crates/ui-egui/src/keytips.rs`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Implement keytip mappings for Table Design and Table Layout ribbons.
- **Risk**: Low.

## Issue 1182: Random placeholder profile icon at the top right
- **Summary**: A "WU" placeholder icon appears in the top right corner.
- **Root Cause**: Leftover template/feature code in `crates/ui-egui`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Locate and remove the rendering code for this icon.
- **Risk**: Very Low.

## Issue 1181: Mirror margins do not load correctly from .docx; shapes shift right
- **Summary**: Mirror margins in .docx files are not loaded correctly, affecting shape positions.
- **Root Cause**: Potential bugs in `crates/docx/src/read/mod.rs` or mirror margin application in `crates/layout/src/lib.rs`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Verify DOCX loading logic and apply mirrored offsets correctly in the layout engine.
- **Risk**: Medium.

## Issue 1178: Modify styles page does not have a font selector
- **Summary**: Style modification dialog uses a plain text field instead of a font combo box.
- **Root Cause**: `TextEdit` used instead of `font_combo` in the style dialog.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Replace `TextEdit` with `font_combo` widget from `crates/ui-egui/src/widgets.rs`.
- **Risk**: Low.

## Issue 1174: DOCX: legacy form fields not drawn/saved; gridBefore mispositioned/lost
- **Summary**: Legacy form fields and `gridBefore` table offsets are ignored/stripped.
- **Root Cause**: Missing support for `w:ffData` and `w:gridBefore` in `crates/docx` and `crates/layout`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Implement support for legacy form fields in the reader/writer and `gridBefore` logic in table layout.
- **Risk**: Medium.
## Issue 1173: Picture Adjustments compound
- **Summary**: Adjustments (transparency, color, etc.) are applied to the already-edited image, compounding effects.
- **Root Cause**: `adjust()` in `crates/engine/src/cmd/objects.rs` decodes the current bitmap instead of the original.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Use `adjust_media()` to apply adjustments from the original source.
- **Risk**: Low.

## Issue 1171: Embedded fonts in .docx not used
- **Summary**: Embedded fonts in .docx are ignored, falling back to system fonts.
- **Root Cause**: Embedded font parts are read but not integrated into `FontDb` in `crates/fonts`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Extract and de-obfuscate embedded fonts from DOCX and register them as document-scoped fonts.
- **Risk**: Medium.

## Issue 1169: Typing stops on XWayland with fcitx5 XIM
- **Summary**: Typing stops on XWayland with fcitx5 XIM due to IME interrupts.
- **Root Cause**: `crates/ui-egui/src/canvas.rs` calls `request_focus()` on every interaction, interrupting IME and causing a race condition in fcitx5.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Only call `request_focus()` if the widget does not already have focus.
- **Risk**: Very Low.

## Issue 1167: Proofing dictionary missing hint
- **Summary**: No feedback is given when a proofing dictionary is missing.
- **Root Cause**: Lack of UI notification for missing dictionaries.
- **Status**: `NEEDS DECISION`
- **Planned Fix**: Implement a UI hint (e.g., dimmed language in status bar or a mark in the Language menu).
- **Risk**: Low.

## Issue 1160: Broken links to `storytold/craftrules`
- **Summary**: Links in `CLAUDE.md` and `AGENTS.md` are broken.
- **Root Cause**: Target repository is private or moved.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Update URLs or mirror essential standards into the WordCraft repo.
- **Risk**: Negligible.

## Issue 1159: Bulleted lists alignment bug
- **Summary**: Bullets remain anchored to the margin when paragraph alignment is changed.
- **Root Cause**: Bullet position in `crates/layout/src/para.rs` is not relative to the aligned text block.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Update layout engine to position bullets relative to the text block's start.
- **Risk**: Low.

## Issue 1158: Multi-column shape wrap bug
- **Summary**: Shape wrap areas are not correctly applied to text in previous columns.
- **Root Cause**: Wrap areas are only added when the anchor paragraph is laid out.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Implement a layout re-pass for the current page if a shape's wrap area overlaps already-placed text.
- **Risk**: Medium.

## Issue 1156: Print preview zoom/size
- **Summary**: Print preview is static and small.
- **Root Cause**: Print preview UI lacks zoom and pan capabilities.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Update Print page UI to allow the preview to fill space and add zoom/pan support.
- **Risk**: Low.

## Issue 1155: Finnish proofing (Voikko)
- **Summary**: Request to integrate Finnish proofing via `corevoikko`.
- **Root Cause**: Feature request.
- **Status**: `NEEDS DECISION`
- **Planned Fix**: Review and integrate the provided working branch if approved.
- **Risk**: Low.

## Issue 1148: Dark theme ruler shading inverted
- **Summary**: Ruler shading in dark theme makes text area darker than margins.
- **Root Cause**: Inverted colors for `ruler` and `ruler_margin` in `crates/ui-egui/src/theme.rs`.
- **Status**: `NEEDS FIX`
- **Planned Fix**: Swap the values of `ruler` and `ruler_margin` in the dark theme tokens.
- **Risk**: Negligible.
