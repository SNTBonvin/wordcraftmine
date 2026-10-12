# Hidden maximized Windows startup

This patch adapts source PR [pdfcraft #674](https://github.com/storytold/pdfcraft/pull/674)
and upstream [winit #4587](https://github.com/rust-windowing/winit/pull/4587) to the
vendored winit 0.30.13 baseline. A hidden maximized window is shown with one
`SW_MAXIMIZE`, avoiding a visible unpainted frame before the first application
frame. The guard ports an exhaustive model of the four state flags and checks
all 64 visible-to-visible transitions against the old `ShowWindow` sequence,
hidden-target calls, and the one-call hidden-to-visible-maximized path. The
source evidence and known edge case are documented in [PdfCraft #674](https://github.com/storytold/pdfcraft/pull/674).

Guard: `apps/wordcraft/tests/winit_windows_hidden_maximized.rs`.
