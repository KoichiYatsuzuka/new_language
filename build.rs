// build.rs — auto-configures LLVM_SYS_NNN_PREFIX from ar_config.json.
//
// When the `llvm` feature is enabled, this script:
//   1. Reads ar_config.json from the project root.
//   2. Extracts llvm.path and llvm.version (e.g. "22.1.6").
//   3. Falls back to running `{path}/bin/llvm-config --version` if version is absent.
//   4. Writes LLVM_SYS_{MAJOR}{MINOR}_PREFIX = "{path}" into .cargo/config.toml.
//
// First run: .cargo/config.toml is written; re-run `cargo build --features llvm`.
// Second run: cargo reads config.toml before build scripts → LLVM found automatically.

fn main() {
    if std::env::var("CARGO_FEATURE_LLVM").is_err() {
        return;
    }

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let root     = std::path::Path::new(&manifest);

    println!("cargo:rerun-if-changed={}", root.join("ar_config.json").display());
    println!("cargo:rerun-if-changed={}", root.join(".cargo").join("config.toml").display());

    let config_path = root.join("ar_config.json");
    if !config_path.exists() {
        println!(
            "cargo:warning=ar_config.json not found. \
             Add {{\"llvm\":{{\"path\":\"C:/Program Files/LLVM\"}}}} or set LLVM_SYS_NNN_PREFIX manually."
        );
        return;
    }

    let json = match std::fs::read_to_string(&config_path) {
        Ok(s)  => s,
        Err(e) => { println!("cargo:warning=Cannot read ar_config.json: {e}"); return; }
    };

    let llvm_path = match json_str(&json, "llvm", "path") {
        Some(p) => p,
        None    => { println!("cargo:warning=ar_config.json: missing llvm.path"); return; }
    };

    // ── Determine LLVM version ────────────────────────────────────────────────
    // Prefer the explicit "version" field; fall back to running llvm-config.
    let version_str = if let Some(v) = json_str(&json, "llvm", "version") {
        v
    } else {
        let llvm_config = std::path::Path::new(&llvm_path)
            .join("bin")
            .join(if cfg!(windows) { "llvm-config.exe" } else { "llvm-config" });
        match std::process::Command::new(&llvm_config).arg("--version").output() {
            Ok(o)  => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(e) => {
                println!(
                    "cargo:warning=llvm-config not found at {} and no version in ar_config.json: {e}",
                    llvm_config.display()
                );
                return;
            }
        }
    };

    // "22.1.6" → major=22, minor=1 → env var suffix "221"
    let parts: Vec<&str> = version_str.split('.').collect();
    let major = parts.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    let suffix    = format!("{major}{minor}");
    let env_key   = format!("LLVM_SYS_{suffix}_PREFIX");
    let llvm_fwd  = llvm_path.replace('\\', "/");

    // ── Write .cargo/config.toml ─────────────────────────────────────────────
    let dot_cargo  = root.join(".cargo");
    let _          = std::fs::create_dir_all(&dot_cargo);
    let toml_path  = dot_cargo.join("config.toml");
    let existing   = std::fs::read_to_string(&toml_path).unwrap_or_default();

    if existing.contains(&env_key) {
        // Already configured for this LLVM version — nothing to do.
        return;
    }

    // Build the new [env] lines: the version-specific key + strict versioning off
    // (LLVM_SYS_STRICT_VERSIONING=0 allows inkwell built against one LLVM API
    // to link against a different minor version without aborting the build).
    let new_lines = format!(
        "{env_key} = \"{llvm_fwd}\"\nLLVM_SYS_STRICT_VERSIONING = \"0\"\n"
    );

    let new_content = if existing.contains("[env]") {
        existing.replacen("[env]\n", &format!("[env]\n{new_lines}"), 1)
    } else {
        format!("{}\n[env]\n{new_lines}", existing.trim_end())
    };

    match std::fs::write(&toml_path, &new_content) {
        Ok(()) => println!(
            "cargo:warning=.cargo/config.toml updated: {env_key}={llvm_fwd} (LLVM {version_str}). \
             Re-run `cargo build --features llvm`."
        ),
        Err(e) => println!("cargo:warning=Could not write .cargo/config.toml: {e}"),
    }
}

/// Minimal JSON string extractor for `"outer": { "inner": "VALUE" }`.
/// No external dependencies — uses only std.
fn json_str(json: &str, outer: &str, inner: &str) -> Option<String> {
    let outer_key  = format!("\"{outer}\"");
    let pos        = json.find(&outer_key)?;
    let after_key  = &json[pos + outer_key.len()..];
    let brace_pos  = after_key.find('{')?;
    let block      = &after_key[brace_pos..];
    let close      = block.find('}')?;
    let block      = &block[..=close];

    let inner_key  = format!("\"{inner}\"");
    let ki         = block.find(&inner_key)? + inner_key.len();
    let colon      = block[ki..].find(':')? + ki + 1;
    let value_part = block[colon..].trim_start();
    if !value_part.starts_with('"') { return None; }
    let inner_str  = &value_part[1..];
    let end        = inner_str.find('"')?;
    Some(inner_str[..end].to_string())
}
