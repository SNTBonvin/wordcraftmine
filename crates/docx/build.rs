//! A branded build (`WORDCRAFT_BRAND_DIR` = a folder with `brand.toml`) writes its own name into
//! `docProps/app.xml`: the brand's top-level `name = "…"`. Without it, nothing changes.

fn main() {
    println!("cargo::rerun-if-env-changed=WORDCRAFT_BRAND_DIR");
    let name = std::env::var_os("WORDCRAFT_BRAND_DIR").and_then(|dir| {
        let path = std::path::Path::new(&dir).join("brand.toml");
        println!("cargo::rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(path).ok()?;
        // The first `name = "…"` line, before any [table]: the product name.
        text.lines().take_while(|l| !l.trim_start().starts_with('[')).find_map(|l| {
            let rest = l.trim().strip_prefix("name")?.trim_start().strip_prefix('=')?.trim().strip_prefix('"')?;
            let name = &rest[..rest.find('"')?];
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-').then(|| name.to_string())
        })
    });
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("brand_name.rs");
    let value = name.map_or("None".to_string(), |n| format!("Some({n:?})"));
    let _ = std::fs::write(out, format!("/// The branded product name, if any.\npub const BRAND_NAME: Option<&str> = {value};\n"));
}
