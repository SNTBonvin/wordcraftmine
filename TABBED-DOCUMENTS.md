# WordCraft — Multi-document tabs and workspace restoration

This document describes WordCraft's multi-document tab functionality. It uses the existing
editor engine and WordCraft branding; each open file has an independent editing session.

## Features

- **Multiple open documents:** Open or create documents in tabs above the ribbon. Switching tabs
  retains each document's undo/redo history, canvas and view state, and AutoSave authority.
- **Tab organization:** Drag tabs to reorder them within their pinned or unpinned group, or use
  Move Tab Left/Right in the tab context menu. A horizontal scroll area accommodates long tab lists.
- **Pinning:** Pin or unpin a tab from its context menu. Pinned tabs are grouped on the left, and
  their order is saved for the next launch.
- **Tab context menu:** Pin/Unpin, Move Left/Right, Copy Full Path (when a file is saved), Close,
  Close Other Unpinned Tabs, Close All Tabs, and Reopen Closed Tab.
- **Recently closed files:** Reopen a file-backed tab from the context menu or with `Ctrl+Shift+T`.
  This reloads the file from disk, not discarded edits. Up to 20 closed file-backed tabs are
  remembered for the current application run.
- **Workspace restoration:** After a normal application exit, the next ordinary desktop launch
  restores saved file paths, tab order, pinning, and the selected tab from WordCraft's existing
  `ui.json` preferences. Missing or unreadable files are skipped.
- **Save Changes prompts:** Closing a dirty tab prompts before discarding it. On exit, WordCraft
  reviews dirty documents, and Cancel aborts the exit. A successful Save As updates the file path
  used for restoration.
- **Explicitly closing all tabs:** Closing the final tab exits WordCraft and clears the saved
  workspace, so the next ordinary launch starts with a blank document.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+Tab` | Select next document tab |
| `Ctrl+Shift+Tab` | Select previous document tab |
| `Ctrl+Shift+T` | Reopen most recently closed file-backed tab |

The existing New, Open, Save, and Close commands remain available. New and Open now add or select
separate tabs instead of replacing the active document.

## Build and validation

On Windows, use a supported Rust toolchain and Microsoft C++ Build Tools. From the repository
root (the folder containing `Cargo.toml`):

```powershell
cargo build --release --locked -p wordcraft
.\target\release\wordcraft.exe
```

To run the UI tests and workspace checks:

```powershell
cargo test --locked -p wordcraft-ui-egui
cargo xtask ci
```

For WebAssembly checks, install the Rust target first if needed:

```powershell
rustup target add wasm32-unknown-unknown
cargo xtask wasm
```

To test automatic restoration, launch without `--sample` or explicit document filenames.

## Scope and limitations

- Preferences persist **paths and blank-tab placeholders**, not document contents. Untitled tabs
  return as blank documents; unsaved content is not recoverable after exit. Save important edits
  to disk before closing.
- Workspace restoration is recorded on **normal application exit**, not continuously. A crash or
  forced termination may leave an older workspace arrangement.
- Launching with `--sample` or explicit filenames skips automatic workspace restoration.
- Undo history, selections, scroll positions, and AutoSave state are not reconstructed across
  application restarts; file-backed documents are freshly loaded from disk.
- Recently closed tab history is in memory only and resets on restart.
