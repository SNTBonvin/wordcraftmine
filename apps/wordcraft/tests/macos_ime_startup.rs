//! Run the native dependency's pure Korean-startup policy tests in workspace CI.
//! This module contains no platform APIs, so its negative controls run on every host.
#[path = "../../../vendor/winit/src/platform_impl/macos/ime_startup.rs"]
mod ime_startup;

#[test]
fn korean_ascii_korean_rearms_retry_without_a_space() {
    let mut policy = ime_startup::ImeStartup::default();
    let korean = "com.apple.inputmethod.Korean.2SetKorean";
    let id = policy.begin(korean, "ㅈ", true);
    policy.insert("ㅈ");
    assert!(policy.finish(id, true));
    for c in ["a", "s", "d", "f"] {
        let id = policy.begin("com.apple.keylayout.ABC", c, true);
        policy.insert(c);
        assert!(!policy.finish(id, true));
    }
    let id = policy.begin(korean, "ㄷ", true);
    policy.insert("ㄷ");
    assert!(policy.finish(id, true), "switching back to Korean rearms the one-shot native retry");
    let id = policy.begin(korean, "ㄷ", true);
    policy.insert("ㄷ");
    assert!(!policy.finish(id, true), "the same source gets no second retry");
}
