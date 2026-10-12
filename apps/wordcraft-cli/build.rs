fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WORDCRAFT_BUILD_NUMBER");
    println!("cargo:rerun-if-changed=../../.build_number");

    let build_num = std::env::var("WORDCRAFT_BUILD_NUMBER").ok().or_else(|| {
        std::fs::read_to_string("../../.build_number").ok().map(|s| s.trim().to_string())
    });
    if let Some(num) = build_num.filter(|s| !s.is_empty()) {
        println!("cargo:rustc-env=WORDCRAFT_BUILD_NUMBER={num}");
    }
}

