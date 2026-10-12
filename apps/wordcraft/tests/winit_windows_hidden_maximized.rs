//! Structural and exhaustive state-model guard for the hidden-maximized winit fix.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct State {
    visible: bool,
    maximized: bool,
    minimized: bool,
    activate: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShowCall {
    Show,
    ShowNoActivate,
    Maximize,
    Restore,
    Minimize,
    Hide,
}

fn states() -> impl Iterator<Item = State> {
    (0..16).map(|bits| State { visible: bits & 1 != 0, maximized: bits & 2 != 0, minimized: bits & 4 != 0, activate: bits & 8 != 0 })
}

fn old_show_calls(before: State, after: State) -> Vec<ShowCall> {
    if before == after {
        return Vec::new();
    }

    let mut calls = Vec::new();
    if after.visible {
        calls.push(if before.activate { ShowCall::Show } else { ShowCall::ShowNoActivate });
    }
    if before.maximized != after.maximized || after.maximized {
        calls.push(if after.maximized { ShowCall::Maximize } else { ShowCall::Restore });
    }
    if before.minimized != after.minimized {
        calls.push(if after.minimized { ShowCall::Minimize } else { ShowCall::Restore });
    }
    if !after.visible {
        calls.push(ShowCall::Hide);
    }
    calls
}

fn patched_show_calls(before: State, after: State) -> Vec<ShowCall> {
    if before == after {
        return Vec::new();
    }

    let visibility_changed = before.visible != after.visible;
    let mut calls = Vec::new();
    if after.visible {
        if after.maximized && visibility_changed {
            calls.push(ShowCall::Maximize);
        } else {
            calls.push(if before.activate { ShowCall::Show } else { ShowCall::ShowNoActivate });
        }
    }
    if (before.maximized != after.maximized || after.maximized) && after.visible && !visibility_changed {
        calls.push(if after.maximized { ShowCall::Maximize } else { ShowCall::Restore });
    }
    if before.minimized != after.minimized {
        calls.push(if after.minimized { ShowCall::Minimize } else { ShowCall::Restore });
    }
    if !after.visible {
        calls.push(ShowCall::Hide);
    }
    calls
}

#[test]
fn source_keeps_maximized_show_in_one_call_and_defers_hidden_state() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = std::fs::read_to_string(root.join("vendor/winit/src/platform_impl/windows/window_state.rs")).unwrap();
    assert!(source.contains("if new.contains(WindowFlags::MAXIMIZED) && diff.contains(WindowFlags::VISIBLE) {"));
    assert!(source.contains("&& new.contains(WindowFlags::VISIBLE)\n            && !diff.contains(WindowFlags::VISIBLE)"));
}

#[test]
fn every_visible_to_visible_transition_keeps_the_old_showwindow_sequence() {
    let mut transitions = 0;
    for before in states().filter(|state| state.visible) {
        for after in states().filter(|state| state.visible) {
            assert_eq!(old_show_calls(before, after), patched_show_calls(before, after), "visible transition changed: {before:?} -> {after:?}");
            transitions += 1;
        }
    }
    assert_eq!(transitions, 64);
}

#[test]
fn hidden_targets_only_use_restore_on_the_minimize_path() {
    for before in states() {
        for after in states().filter(|state| !state.visible) {
            let calls = patched_show_calls(before, after);
            assert!(!calls.contains(&ShowCall::Show));
            assert!(!calls.contains(&ShowCall::ShowNoActivate));
            assert!(!calls.contains(&ShowCall::Maximize));
            if before.minimized == after.minimized {
                assert!(!calls.contains(&ShowCall::Restore));
            }
        }
    }
}

#[test]
fn hidden_to_visible_maximized_uses_a_single_maximize_call() {
    for before in states().filter(|state| !state.visible && !state.maximized && !state.minimized) {
        for after in states().filter(|state| state.visible && state.maximized && !state.minimized) {
            assert_eq!(patched_show_calls(before, after), [ShowCall::Maximize]);
        }
    }
}
