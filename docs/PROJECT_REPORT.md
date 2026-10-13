# WordCraft Project Report

## 1. Overview
WordCraft is an open-source, Rust-native word processor designed for high-performance document creation with a target of Microsoft Word parity. It is developed as part of the "ArtCraft" suite of creative tools.

- **Purpose**: To provide a clean-room, open-source alternative to proprietary word processors, focusing on speed, openness, and AI-agent controllability.
- **Target Users**: Writers, designers, and professionals who require a familiar Word-like workflow (ribbon, styles, track changes) but prefer open-source software.
- **Maturity**: Currently in **Alpha**. It implements approximately 88% of Word's ribbon features. The team estimates it is ~60% toward replacing Word for general professional work.

## 2. Tech Stack
- **Language**: Rust (Edition 2024, version 1.90).
- **UI Framework**: `egui` / `eframe` (GPU-accelerated via `wgpu`).
- **Rendering**: `vello_cpu` for rasterization, `krilla` for PDF export.
- **Fonts/Shaping**: `skrifa`, `harfrust`.
- **Formats**: OOXML (`docx`), legacy `.doc` (`docbin`), and others (Markdown, HTML, RTF, ODT, LaTeX) via a shared intermediate model.
- **Packaging**: `nfpm` (Linux .deb/.rpm), `WiX v5` (Windows .msi), `hdiutil` (macOS .dmg), `Trunk` (WASM/Web).
- **Toolchain**: Cargo, GitHub Actions for CI.

## 3. Architecture
### Directory Tree
- `apps/`: Binaries (`wordcraft` desktop, `wordcraft-cli`, `wordcraft-web`).
- `crates/`: Core logic divided into layers (L0-L6).
- `packaging/`: Platform-specific build scripts (linux, macos, windows, web).
- `xtask/`: Custom cargo tasks for parity, layers, and assets.
- `docs/`: Parity checklists, architecture, and roadmap.
- `assets/`: App icons, fonts, and linguistic data.

### Main Modules (Layers)
| Layer | Crate | Responsibility |
|---|---|---|
| L0 | `geom` | Basic units, rectangles, and measurement parsing. |
| L1 | `doc`, `fonts`, `proof` | Document model, font database/shaping, and linguistic proofing. |
| L2 | `layout`, `docx`, `formats` | Pagination, line breaking, OOXML I/O, and other format converters. |
| L3 | `render`, `pdf` | Rasterizing the display list to images or PDF. |
| L4 | `engine` | `Session` management, undo/redo, and the **Command Registry** (422 commands). |
| L5 | `mcp` | Model Context Protocol server for AI agent integration. |
| L6 | `ui-egui` | The visual front-end (ribbon, canvas, dialogs). |

### Data Flow & State
1. **I/O**: Files are read via `engine::io` $\rightarrow$ `doc::Document`.
2. **Execution**: Every action is a `CommandSpec` run through `Session::run`.
3. **Layout**: The `layout` crate incrementally produces a display list of positioned glyphs/boxes.
4. **Render**: The UI paints the display list using `render` on a GPU canvas.
5. **Persistence**: Saved back via `engine::io` using the appropriate writer.

## 4. Build and Packaging System
### Build Scripts
- **Linux**: `packaging/linux/package.sh` produces AppImages, .deb, .rpm, and .tar.gz.
- **macOS**: `packaging/macos/package.sh` produces signed and notarized .dmg and .zip.
- **Windows**: `packaging/windows/package.ps1` produces .msi (via WiX) and portable .zip.
- **Web**: `packaging/web/package.sh` uses `Trunk` to build WASM.

### CI/CD
- **Workflows**: `.github/workflows/` handles release builds, packaging linting, and cross-platform checks (FreeBSD, Windows ARM64).
- **Quality Gates**: `cargo xtask ci` runs fmt, clippy, tests, asset attribution, and layering checks.

### Release Process
- Artifacts are named `wordcraft-<version>-<platform>-<arch>.<ext>`.
- macOS/Windows builds include code-signing and (for macOS) notarization.

## 5. Testing
- **Unit/Integration**: ~821 `#[test]` functions.
- **Property Testing**: 10 blocks for hostile input and round-trips.
- **Typing Parity**: `crates/engine/src/tests_typing.rs` ensures typing behavior matches Word.
- **Stability**: `hostile_params_never_panic` tests every command with junk parameters to ensure no crashes.
- **UI Testing**: `ui_shot` example for headless screenshots.

## 6. Code Quality Findings
### High Risk: Never-Crash Violations
The project has a strict "Never Crash" policy (no `unwrap`, `expect`, `panic` in non-test code). However, analysis found numerous violations:
- **`crates/docx/src/read/math.rs`**: Multiple `panic!` calls during XML parsing of math nodes.
- **`crates/docx/src/read/diagram.rs`**: Multiple `panic!` calls in diagram parsing.
- **`crates/engine/src/cmd/ink_shape.rs`**: `panic!` when recognition fails.
- **`crates/ui-egui/src/lib.rs`**: Numerous `panic!` calls in dialog handling and canvas layout.
- **General**: Widespread use of `.unwrap()` in `crates/docx` (e.g., `package.rs`, `smart_art.rs`, `xml.rs`).

### Medium Risk: Security & Robustness
- **Hostile Input**: While `hostile_params_never_panic` exists, the presence of `unwrap()` in `docx` readers suggests potential crash vectors when opening malformed `.docx` files.
- **Dependency Audit**: `cargo audit` is not integrated into the local environment provided to the agent, but the `Cargo.toml` uses modern versions of key crates.

### Documentation Gaps
- Architecture is well-documented in `docs/architecture.md` and `CLAUDE.md`.
- Parity is tracked meticulously in `docs/parity-checklist.md`.

## 7. Dependency Audit
The project relies on a lean set of high-quality Rust crates:
- `egui`/`eframe` for UI.
- `vello_cpu` for rendering.
- `serde` for serialization.
- `thiserror` for error handling.
- `quick-xml` for OOXML processing.

## 8. Prioritized Recommendations

### High Priority
1. **Eliminate Panics**: Replace all `unwrap()`, `expect()`, and `panic!()` calls in `crates/` with `Result<T, E>` and proper error propagation. This is a core project requirement ("Never Crash").
2. **Harden Docx Reader**: Audit the `docx` and `docbin` crates specifically for boundary checks and safe XML parsing to prevent crashes on corrupted files.

### Medium Priority
3. **Expand Testing**: Increase property-test coverage for the intermediate document model to ensure stability across format conversions.
4. **CI Integration**: Integrate `cargo audit` directly into `cargo xtask ci` to catch vulnerable dependencies automatically.

### Low Priority
5. **Refine UI Error States**: Instead of panicking when a dialog is missing (as seen in `ui-egui`), implement graceful fallback or logging.
