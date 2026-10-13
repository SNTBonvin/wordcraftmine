//! Exercise the actual build script with WASM's environment, without requiring craft-fonts.
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[test]
fn wasm_build_embeds_chinese_and_checks_required_scripts() {
    let temp = std::env::temp_dir().join(format!("wordcraft-web-font-build-{}", std::process::id()));
    fs::create_dir_all(temp.join("fonts")).unwrap();
    fs::create_dir_all(temp.join("out")).unwrap();
    let script = temp.join(format!("font-build{}", std::env::consts::EXE_SUFFIX));
    let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--edition=2024")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs"))
        .arg("-o")
        .arg(&script)
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{}", String::from_utf8_lossy(&compiled.stderr));

    // The script embeds bytes; it doesn't parse them. Missing bold and Arabic files must never
    // be opened by the web selection, even though they exist in the full native manifest.
    fs::write(temp.join("fonts/japanese.ttf"), b"Japanese fixture").unwrap();
    fs::write(temp.join("fonts/chinese.otf"), b"Chinese fixture").unwrap();
    let japanese = "BIZ UDPGothic | Regular | fonts/japanese.ttf | Jpan,Latn";
    let chinese = "Noto Sans CJK SC | Regular | fonts/chinese.otf | Hans,Latn";
    fs::write(
        temp.join("fonts/manifest.txt"),
        format!("{japanese}\n{chinese}\nBIZ UDPGothic | Bold | fonts/missing-bold.ttf | Jpan\nNoto Sans Arabic | Regular | fonts/missing-arabic.ttf | Arab\n"),
    )
    .unwrap();
    let run = || -> Output {
        Command::new(&script)
            .env("CARGO_CFG_TARGET_ARCH", "wasm32")
            .env("CRAFT_FONTS_DIR", &temp)
            .env("CRAFT_FONTS_REQUIRED", "1")
            .env("OUT_DIR", temp.join("out"))
            .output()
            .unwrap()
    };
    let output = run();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("cargo::error="), "{stdout}");
    let generated = fs::read_to_string(temp.join("out/craft_fonts.rs")).unwrap();
    assert!(generated.contains("Noto Sans CJK SC"), "{generated}");
    assert!(generated.contains("BIZ UDPGothic"), "{generated}");
    assert_eq!(generated.matches("CraftFont {").count(), 2, "{generated}");

    // Cargo treats cargo::error as a failed build. An old font checkout must be rejected on
    // WASM too, instead of silently shipping Japanese-only fonts for the Chinese interface.
    fs::write(temp.join("fonts/manifest.txt"), japanese).unwrap();
    let missing = String::from_utf8(run().stdout).unwrap();
    assert!(missing.contains("cargo::error="), "{missing}");
    assert!(missing.contains("no Hans font"), "{missing}");
    fs::remove_dir_all(temp).unwrap();
}
