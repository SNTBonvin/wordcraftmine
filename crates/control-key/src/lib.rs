//! The control channel's key file, one rule for the app (which writes it) and the MCP bridge
//! (which reads it): `<settings>/control-key.<instance>`. See `docs/control-protocol.md`, Keys.

use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
pub use live_auth::{fresh_nonce, live_auth_proof, verify_live_auth_proof};

/// Mutual authentication for live TCP only. Legacy TCP and private Unix sockets retain
/// their existing protocols. Neither role sends the installation's secret over TCP.
#[cfg(not(target_arch = "wasm32"))]
mod live_auth {
    use std::path::Path;

    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn decode(value: &str) -> Result<[u8; 32], String> {
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err("live authentication requires 32-byte lowercase hex values".into());
        }
        let mut decoded = [0u8; 32];
        for (byte, pair) in decoded.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
            let digits = std::str::from_utf8(pair).map_err(|_| "invalid live authentication hex")?;
            *byte = u8::from_str_radix(digits, 16).map_err(|_| "invalid live authentication hex")?;
        }
        Ok(decoded)
    }

    /// A cryptographically fresh 32-byte nonce, encoded as 64 lowercase hex characters.
    pub fn fresh_nonce() -> Result<String, String> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| format!("cannot create live authentication nonce: {e}"))?;
        Ok(hex(&bytes))
    }

    fn authenticator(key: &str, role: &str, session: &str, document: u64, client: &str, server: &str) -> Result<Hmac<Sha256>, String> {
        if !matches!(role, "server" | "client") || super::session_path(Path::new(""), session).is_none() {
            return Err("invalid live authentication role or session identity".into());
        }
        let key = decode(key)?;
        let client = decode(client)?;
        let server = decode(server)?;
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key).map_err(|_| "invalid live authentication key")?;
        mac.update(b"wordcraft/live-tcp/hmac-sha256-v1\0");
        // Length prefixes avoid ambiguous concatenation; the role prevents reflection of
        // a server proof as a client proof. Both nonces bind the proof to one connection.
        for part in [role.as_bytes(), session.as_bytes(), &document.to_be_bytes(), &client, &server] {
            mac.update(&(part.len() as u64).to_be_bytes());
            mac.update(part);
        }
        Ok(mac)
    }

    pub fn live_auth_proof(
        key: &str,
        role: &str,
        session_id: &str,
        document_id: u64,
        client_nonce: &str,
        server_nonce: &str,
    ) -> Result<String, String> {
        Ok(hex(&authenticator(key, role, session_id, document_id, client_nonce, server_nonce)?.finalize().into_bytes()))
    }

    /// Verify with the MAC implementation's constant-time comparison, never string equality.
    pub fn verify_live_auth_proof(
        key: &str,
        role: &str,
        session_id: &str,
        document_id: u64,
        client_nonce: &str,
        server_nonce: &str,
        proof: &str,
    ) -> Result<(), String> {
        let proof = decode(proof)?;
        authenticator(key, role, session_id, document_id, client_nonce, server_nonce)?
            .verify_slice(&proof)
            .map_err(|_| "live authentication proof is invalid".into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn proofs_bind_role_identity_and_both_nonces() {
            let key = "01".repeat(32);
            let client = "02".repeat(32);
            let server = "03".repeat(32);
            let proof = live_auth_proof(&key, "server", "session", 7, &client, &server).unwrap();
            // Independent reference vector for the length-prefixed protocol transcript.
            assert_eq!(proof, "d9a3ae9c9a719cb1b6eae06b8e06dde800e35e8439d94280ffbee5525a14dafb");
            verify_live_auth_proof(&key, "server", "session", 7, &client, &server, &proof).unwrap();
            for (role, session, document, c, s) in [
                ("client", "session", 7, client.clone(), server.clone()),
                ("server", "other", 7, client.clone(), server.clone()),
                ("server", "session", 8, client.clone(), server.clone()),
                ("server", "session", 7, "04".repeat(32), server.clone()),
                ("server", "session", 7, client.clone(), "05".repeat(32)),
            ] {
                assert!(verify_live_auth_proof(&key, role, session, document, &c, &s, &proof).is_err());
            }
            assert!(verify_live_auth_proof(&"06".repeat(32), "server", "session", 7, &client, &server, &proof).is_err());
        }

        #[test]
        fn malformed_values_are_rejected_and_nonces_are_fresh() {
            let nonce = fresh_nonce().unwrap();
            assert_eq!(nonce.len(), 64);
            assert_ne!(nonce, fresh_nonce().unwrap());
            for bad in [String::new(), "a".repeat(63), "A".repeat(64), "界".repeat(32), "g".repeat(64)] {
                assert!(live_auth_proof(&bad, "server", "session", 7, &nonce, &nonce).is_err());
                assert!(live_auth_proof(&nonce, "server", "session", 7, &bad, &nonce).is_err());
                assert!(verify_live_auth_proof(&nonce, "server", "session", 7, &nonce, &nonce, &bad).is_err());
            }
            assert!(live_auth_proof(&nonce, "other", "session", 7, &nonce, &nonce).is_err());
            assert!(live_auth_proof(&nonce, "server", "../session", 7, &nonce, &nonce).is_err());
        }
    }
}

/// The app's settings folder (preferences, logs, key files): macOS
/// `~/Library/Application Support/WordCraft`, Windows `%APPDATA%\WordCraft`, otherwise
/// `$XDG_CONFIG_HOME/wordcraft` or `~/.config/wordcraft`.
pub fn settings_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/WordCraft"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("WordCraft"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("wordcraft"))
    }
}

/// The `instance-id=` value of a `/.flatpak-info` file, only when it is `[A-Za-z0-9_-]+` (it
/// becomes part of a file name).
pub fn parse_instance_id(info: &str) -> Option<String> {
    let v = info.lines().find_map(|l| l.strip_prefix("instance-id="))?.trim();
    (!v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')).then(|| v.to_string())
}

/// The Flatpak instance id, or `None` outside Flatpak.
pub fn instance_id() -> Option<String> {
    parse_instance_id(&std::fs::read_to_string("/.flatpak-info").ok()?)
}

/// `<settings>/control-key.<instance>`, with `<instance>` the Flatpak instance id or
/// `local-<port>`.
pub fn key_path(settings: &Path, port: u16) -> PathBuf {
    key_path_for(settings, instance_id().as_deref(), port)
}

fn key_path_for(settings: &Path, instance: Option<&str>, port: u16) -> PathBuf {
    match instance {
        Some(id) => settings.join(format!("control-key.{id}")),
        None => settings.join(format!("control-key.local-{port}")),
    }
}

/// Live document discovery: one descriptor for each enabled desktop window.
pub fn sessions_dir(settings: &Path) -> PathBuf {
    settings.join("agents/sessions")
}

/// Safe session identifiers can be used as descriptor file names.
pub fn session_path(settings: &Path, session_id: &str) -> Option<PathBuf> {
    (!session_id.is_empty() && session_id.len() <= 128 && session_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))
        .then(|| sessions_dir(settings).join(format!("{session_id}.json")))
}

/// Settings locations visible to a host MCP bridge, including installed Flatpak windows.
pub fn discovery_settings_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<_> = settings_dir().into_iter().collect();
    if cfg!(target_os = "linux")
        && let Some(home) = std::env::var_os("HOME")
    {
        let flatpak = PathBuf::from(home).join(".var/app/ai.storyteller.wordcraft/config/wordcraft");
        if !dirs.contains(&flatpak) {
            dirs.push(flatpak);
        }
    }
    dirs
}

/// Runtime agent ports are unique and can be reached across Flatpak instances.
/// Keep their keys separate from legacy per-instance automation ports.
pub fn live_key_path(settings: &Path, port: u16) -> PathBuf {
    settings.join(format!("control-key.live-{port}"))
}

/// Unix live sockets are shared across a Flatpak app's instances without networking grants.
/// Each enabled window creates its own exclusively owned 0700 child directory here.
#[cfg(unix)]
pub fn live_runtime_base() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute()) {
        if std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").is_file() {
            return runtime.join("app/ai.storyteller.wordcraft");
        }
        return runtime;
    }
    // Short enough for sockaddr_un on macOS. The parent is shared, while each unpredictably
    // named child is created exclusively with 0700 before any socket exists inside it.
    PathBuf::from("/tmp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn instance_id_only_safe_values() {
        assert_eq!(parse_instance_id("[Instance]\ninstance-id=abc_123-X\n").as_deref(), Some("abc_123-X"));
        assert_eq!(parse_instance_id("instance-id= 42 \n").as_deref(), Some("42"));
        // It ends up in a file name.
        assert_eq!(parse_instance_id("instance-id=../etc\n"), None);
        assert_eq!(parse_instance_id("instance-id=a/b\n"), None);
        assert_eq!(parse_instance_id("instance-id=a b\n"), None);
        assert_eq!(parse_instance_id("instance-id=..\n"), None);
        assert_eq!(parse_instance_id("instance-id=\n"), None);
        assert_eq!(parse_instance_id("app=ai.storyteller.wordcraft\n"), None);
    }

    #[test]
    fn key_file_per_instance() {
        let dir = Path::new("/settings/wordcraft");
        assert_eq!(key_path_for(dir, Some("1234"), 7981), dir.join("control-key.1234"));
        assert_eq!(key_path_for(dir, None, 7981), dir.join("control-key.local-7981"));
        assert_eq!(key_path_for(dir, None, 7982), dir.join("control-key.local-7982"));
        if instance_id().is_none() {
            assert_eq!(key_path(dir, 7981), dir.join("control-key.local-7981"));
        }
    }

    #[test]
    fn settings_dir_is_the_apps_folder() {
        if let Some(d) = settings_dir() {
            let name = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
            assert!(name == "wordcraft" || name == "WordCraft", "{}", d.display());
        }
    }
}
