# Desktop document windows

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (first implementation)

New and Open create another native window when a document is already open. A pristine start
window is reused. Finder Open With, Dock drops and multiple command-line files follow the same
path. Opening a file already visible focuses that window instead of opening another writable copy.

**View → New Window** opens another view of the same document. Edits, saved filename, dirty state,
Undo and Redo are shared; selection, zoom, scrolling, navigation and chat permission stay local.
Shared views have numbered captions. **Switch Windows** focuses a selected document and
**Arrange All** tiles the open windows. The desktop currently bounds the application to 16 windows.

Close/Cmd+W closes that view. Another view of the same document keeps it open without a save
prompt. The final view asks Save/Don't Save/Cancel if needed. Closing the main document while
other windows exist hides the host window; closing the last document returns to the start screen.
Quit/Cmd+Q checks each dirty document group before terminating. Cancelling Quit keeps all windows.
Native pickers in child windows float because eframe does not expose a safe child-window handle
for parenting an AppKit sheet. The root window keeps its normal native sheets.

Each window has its own live agent state and optional authenticated control port. A single
history writer serves the in-process windows; their library, recent-file metadata and favourites
are exchanged on the event-loop thread. Headless and web command clients retain the existing
single-document replacement semantics. Secondary-window UI capture is currently unsupported by the
eframe immediate renderer; `ui.render` still captures document pages in each window.

The Mac start screen uses the installed San Francisco variable face with explicit regular,
medium and semibold weights. It offers document search, sortable opening times, breadcrumb paths,
full-path tooltips, favourites and row menus (Open, Copy path, Remove from Recent). Opening times
record successful opens, never edits or filesystem modification times. Older preference records
without timestamps display an unknown time until reopened. Removing a Recent entry leaves its
file intact. The start screen follows the macOS accent setting available at launch.
Seven compact previews include original Notes and Calendar templates. Recent rows use file icons
provided by macOS, with an original vector fallback; no installed application assets are bundled.

Control diagnostics include `documentWindows` and `viewNumber`. Window commands are
`ui.newDocumentWindow`, `view.newWindow`, `view.arrangeAll`, `ui.focusDocumentWindow`, `ui.closeDocumentWindow` and
`ui.connectDocumentWindow` (the latter three take `{"id":number}`). A child connection chooses
the main control port plus its window ID by default, or the ordinary 7981 base when disconnected.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | minor | Native document/view windows, shared Undo, per-document close and Quit checks, San Francisco UI weights and Recent opening metadata |
