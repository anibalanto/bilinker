use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

// Serialize all tests that read/write the shared Persona.java fixture.
static PERSONA_LOCK: Mutex<()> = Mutex::new(());

// Path to the compiled binary — Cargo sets this env var during test runs.
fn bilinker() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bilinker"))
}

// Workspace root (where .bilinker.toml lives).
fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
}

fn persona_java() -> PathBuf {
    workspace()
        .join("../../expancode/tests/fixtures/java-app/src/main/java/ar/example/demo/persona/Persona.java")
}

fn run(args: &[&str]) -> (String, String, bool) {
    let out = Command::new(bilinker())
        .current_dir(workspace())
        .args(args)
        .output()
        .expect("failed to run bilinker");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

// ─── 1. capture ────────────────────────────────────────────────────────────

#[test]
fn capture_identifies_workspace_file_and_method() {
    let _guard = PERSONA_LOCK.lock().unwrap();
    let (stdout, stderr, ok) = run(&[
        "capture", "java-demo",
        "src/main/java/ar/example/demo/persona/Persona.java",
        "10:5", "12:5",
    ]);
    assert!(ok, "capture failed:\n{stderr}");
    assert!(stdout.contains("java-demo"),   "missing workspace in link");
    assert!(stdout.contains("Persona.java"), "missing file in link");
    assert!(stdout.contains("Persona"),     "missing class name in query");
    assert!(stdout.contains("vote"),        "missing method name in query");
    assert!(stderr.contains("hash:"),       "missing hash in stderr");
}

// ─── 2. get matches sed ────────────────────────────────────────────────────

#[test]
fn get_content_matches_sed_selection() {
    let _guard = PERSONA_LOCK.lock().unwrap();
    let (get_out, stderr, ok) = run(&["get", "persona-voting-impl", "0"]);
    assert!(ok, "get failed:\n{stderr}");

    // Equivalent of: sed -n '10,12p' Persona.java
    let full = fs::read_to_string(persona_java()).expect("read Persona.java");
    let sed: String = full.lines()
        .skip(9)   // 0-based: skip lines 1–9
        .take(3)   // lines 10, 11, 12
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        get_out.trim(),
        sed.trim(),
        "get output differs from sed selection"
    );
}

// ─── 3. hash is a valid sha256 and is deterministic ───────────────────────

#[test]
fn capture_hash_is_valid_sha256_and_stable() {
    let _guard = PERSONA_LOCK.lock().unwrap();
    let args = &[
        "capture", "java-demo",
        "src/main/java/ar/example/demo/persona/Persona.java",
        "10:5", "12:5",
    ];

    let (_, stderr1, ok1) = run(args);
    assert!(ok1, "first capture failed:\n{stderr1}");
    let hash1 = extract_hash(&stderr1);

    assert_eq!(hash1.len(), 64, "hash must be 64 hex chars (SHA-256)");
    assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()), "hash must be hex");

    // Same file, same selection → same hash every time.
    let (_, stderr2, ok2) = run(args);
    assert!(ok2, "second capture failed:\n{stderr2}");
    assert_eq!(hash1, extract_hash(&stderr2), "hash must be deterministic");
}

// ─── 4. drift detection: changing the file changes the hash ────────────────

#[test]
fn drift_changes_hash() {
    let _guard = PERSONA_LOCK.lock().unwrap();
    let path = persona_java();
    let original = fs::read_to_string(&path).expect("read Persona.java");

    // Hash before modification.
    let (_, stderr1, ok1) = run(&[
        "capture", "java-demo",
        "src/main/java/ar/example/demo/persona/Persona.java",
        "10:5", "12:5",
    ]);
    assert!(ok1, "capture (before) failed:\n{stderr1}");
    let hash_before = extract_hash(&stderr1);

    // Modify the file.
    let modified = original.replace(
        "System.out.println(name + \" votes for \" + candidate);",
        "System.out.println(name + \" voted for \" + candidate); // drift",
    );
    fs::write(&path, &modified).expect("write modified");

    // Hash after modification.
    let (_, stderr2, ok2) = run(&[
        "capture", "java-demo",
        "src/main/java/ar/example/demo/persona/Persona.java",
        "10:5", "12:5",
    ]);

    // Always restore before asserting.
    fs::write(&path, &original).expect("restore original");

    assert!(ok2, "capture (after) failed:\n{stderr2}");
    let hash_after = extract_hash(&stderr2);

    assert_ne!(hash_before, hash_after, "hash should change when code drifts");
}

// ─── helpers ───────────────────────────────────────────────────────────────

fn extract_hash(stderr: &str) -> String {
    stderr
        .lines()
        .find(|l| l.starts_with("hash:"))
        .expect("no hash line")
        .trim_start_matches("hash:")
        .trim()
        .to_string()
}

