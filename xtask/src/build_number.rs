//! `cargo xtask build-number [next | set <X.Y>]`: manage `.build_number`.

use std::path::Path;

/// Read current build number from `.build_number`, defaulting to `0.1`.
pub fn read(root: &Path) -> String {
    let path = root.join(".build_number");
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0.1".into())
}

/// Bump to next build number (e.g. 0.1 -> 0.2, 0.9 -> 0.10) and persist it.
pub fn bump(root: &Path) -> Result<String, String> {
    let path = root.join(".build_number");
    if !path.exists() {
        std::fs::write(&path, "0.1\n").map_err(|e| format!("write {}: {e}", path.display()))?;
        return Ok("0.1".into());
    }
    let cur = read(root);
    let (major, minor) = match cur.split_once('.') {
        Some((maj, min)) => (maj, min),
        None => ("0", cur.as_str()),
    };
    let min_num: u64 = minor.parse().unwrap_or(0);
    let next = format!("{major}.{}", min_num + 1);
    std::fs::write(&path, format!("{next}\n")).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(next)
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    match args {
        [] => {
            println!("{}", read(root));
            Ok(())
        }
        ["next"] => {
            let next = bump(root)?;
            println!("{next}");
            Ok(())
        }
        ["set", new] => {
            let path = root.join(".build_number");
            std::fs::write(&path, format!("{}\n", new.trim())).map_err(|e| format!("write {}: {e}", path.display()))?;
            println!("build number set to {new}");
            Ok(())
        }
        _ => Err("usage: cargo xtask build-number [next | set <X.Y>]".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_increments_minor() {
        let tmp = std::env::temp_dir().join(format!("test_build_num_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        // initial bump creates 0.1
        let v1 = bump(&tmp).unwrap();
        assert_eq!(v1, "0.1");

        // next bump becomes 0.2
        let v2 = bump(&tmp).unwrap();
        assert_eq!(v2, "0.2");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

