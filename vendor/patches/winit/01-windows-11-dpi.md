# Windows 11 monitor transitions

When handling `WM_DPICHANGED`, Windows 11 and later use the rectangle suggested
by Windows. Older builds retain winit 0.30.13's existing monitor-adjustment
path. While a drag is still associated with the monitor being left, that old
adjustment can move the window back, trigger another DPI message, and repeat.

The condition and behavior follow [PdfCraft #544](https://github.com/storytold/pdfcraft/pull/544)
and [winit #4341](https://github.com/rust-windowing/winit/pull/4341), commit
`488c036a`. PdfCraft's measurements recorded 9 scale changes in 0.2 seconds
and growth from 1000×650 to 1572×1038 logical pixels for a normal unpatched
drag, versus one change per crossing with no growth after the fix. Maximized
drag changed 7 times in 40 ms versus once, and the corrected rectangle landed
on the target monitor for keyboard movement and snap operations.

Guard: `apps/wordcraft/tests/winit_windows_11_dpi.rs`. This patch changes only
the Windows `WM_DPICHANGED` path; application code remains unchanged.
