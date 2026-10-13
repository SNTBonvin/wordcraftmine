//! Windows owns the caption buttons. Mirror only the non-client area, leaving
//! the GPU surface, input coordinates and document direction unchanged.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[derive(Default)]
pub(crate) struct State {
    attempted: Option<(isize, bool)>,
}

impl State {
    pub(crate) fn sync(&mut self, window: &impl HasWindowHandle, rtl: bool) {
        let Ok(handle) = window.window_handle() else { return };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else { return };
        self.update(handle.hwnd.get(), rtl, || apply(handle.hwnd.get(), rtl));
    }

    fn update(&mut self, hwnd: isize, rtl: bool, apply: impl FnOnce() -> Result<(), String>) {
        let desired = (hwnd, rtl);
        if self.attempted == Some(desired) {
            return;
        }
        // A new LTR window already has native defaults. Preserve them instead of
        // touching DWM every frame. Direction changes also retry a previous failure.
        let initial_ltr = self.attempted.is_none_or(|(old, _)| old != hwnd) && !rtl;
        self.attempted = Some(desired);
        if !initial_ltr && let Err(error) = apply() {
            log::warn!("native caption direction: {error}");
        }
    }
}

fn apply(target: isize, rtl: bool) -> Result<(), String> {
    // WinSafe's raw HWND constructor is unsafe. Instead obtain a typed HWND from
    // the OS on the owning UI thread and match the exact handle supplied by eframe.
    // Never select by title or foreground window, or retain a native handle.
    let mut outcome = None;
    winsafe::EnumThreadWindows(winsafe::GetCurrentThreadId(), |hwnd| {
        if hwnd.ptr() as isize == target {
            outcome = Some(hwnd.DwmSetWindowAttribute(winsafe::DwmAttr::NonClientRtlLayout(rtl)).map_err(|e| e.to_string()));
        }
        true
    })
    .map_err(|e| e.to_string())?;
    outcome.unwrap_or_else(|| Err("the app window was not found on its UI thread".into()))
}

#[cfg(test)]
mod tests {
    use super::State;
    use std::cell::RefCell;

    #[test]
    fn native_calls_follow_direction_changes_without_repeating_failures() {
        let mut state = State::default();
        let calls = RefCell::new(Vec::new());
        let mut update = |hwnd, rtl, success| {
            state.update(hwnd, rtl, || {
                calls.borrow_mut().push((hwnd, rtl));
                if success { Ok(()) } else { Err("DWM unavailable".into()) }
            });
        };
        update(1, false, true);
        update(1, false, true);
        assert!(calls.borrow().is_empty(), "initial LTR preserves Windows defaults");
        update(1, true, true);
        update(1, true, true);
        update(1, false, true);
        update(1, true, false);
        update(1, true, false);
        update(1, false, true);
        update(2, true, true);
        assert_eq!(*calls.borrow(), [(1, true), (1, false), (1, true), (1, false), (2, true)]);
    }

    #[test]
    fn unknown_window_returns_an_error_without_changing_other_windows() {
        assert!(super::apply(0, true).is_err());
    }
}
