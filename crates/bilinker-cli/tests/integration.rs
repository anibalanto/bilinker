use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

static PERSONA_LOCK: Mutex<()> = Mutex::new(());

fn bilinker() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bilinker"))
}

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
}

fn persona_java() -> PathBuf {
    workspace()
        .join("tests/fixtures/java-app/src/main/java/ar/example/demo/persona/Persona.java")
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
        "capture",
        "tests/fixtures/java-app/src/main/java/ar/example/demo/persona/Persona.java",
        "10:5", "12:5",
    ]);
    assert!(ok, "capture failed:\n{stderr}");
    assert!(stdout.contains("Persona.java"), "missing file in link");
    assert!(stdout.contains("Persona"),      "missing class name in query");
    assert!(stdout.contains("vote"),         "missing method name in query");
    assert!(stderr.contains("hash:"),        "missing hash in stderr");
}

// ─── 2. get matches sed ────────────────────────────────────────────────────

#[test]
fn get_content_matches_sed_selection() {
    let _guard = PERSONA_LOCK.lock().unwrap();
    let (get_out, stderr, ok) = run(&["get", "persona-voting-impl.0"]);
    assert!(ok, "get failed:\n{stderr}");

    let full = fs::read_to_string(persona_java()).expect("read Persona.java");
    let sed: String = full.lines()
        .skip(9)
        .take(3)
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
        "capture",
        "tests/fixtures/java-app/src/main/java/ar/example/demo/persona/Persona.java",
        "10:5", "12:5",
    ];

    let (_, stderr1, ok1) = run(args);
    assert!(ok1, "first capture failed:\n{stderr1}");
    let hash1 = extract_hash(&stderr1);

    assert_eq!(hash1.len(), 64, "hash must be 64 hex chars (SHA-256)");
    assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()), "hash must be hex");

    let (_, stderr2, ok2) = run(args);
    assert!(ok2, "second capture failed:\n{stderr2}");
    assert_eq!(hash1, extract_hash(&stderr2), "hash must be deterministic");
}

// ─── 4. drift detection ────────────────────────────────────────────────────

#[test]
fn drift_changes_hash() {
    let _guard = PERSONA_LOCK.lock().unwrap();
    let path = persona_java();
    let original = fs::read_to_string(&path).expect("read Persona.java");

    let (_, stderr1, ok1) = run(&[
        "capture",
        "tests/fixtures/java-app/src/main/java/ar/example/demo/persona/Persona.java",
        "10:5", "12:5",
    ]);
    assert!(ok1, "capture (before) failed:\n{stderr1}");
    let hash_before = extract_hash(&stderr1);

    let modified = original.replace(
        "System.out.println(name + \" votes for \" + candidate);",
        "System.out.println(name + \" voted for \" + candidate); // drift",
    );
    fs::write(&path, &modified).expect("write modified");

    let (_, stderr2, ok2) = run(&[
        "capture",
        "tests/fixtures/java-app/src/main/java/ar/example/demo/persona/Persona.java",
        "10:5", "12:5",
    ]);

    fs::write(&path, &original).expect("restore original");

    assert!(ok2, "capture (after) failed:\n{stderr2}");
    let hash_after = extract_hash(&stderr2);

    assert_ne!(hash_before, hash_after, "hash should change when code drifts");
}

// ─── 5. chain new ──────────────────────────────────────────────────────────

fn isolated_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();

    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/spec.md"), "# Spec\n\nSome spec content.\n").unwrap();
    fs::write(root.join("src/Service.java"),
        "public class Service {\n    public void run() {}\n}\n").unwrap();

    (tmp, root)
}

fn run_in(root: &std::path::Path, args: &[&str]) -> (String, String, bool) {
    let out = std::process::Command::new(bilinker())
        .current_dir(root)
        .args(args)
        .output()
        .expect("failed to run bilinker");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn chain_new_direct_link_creates_single_file() {
    let (_tmp, root) = isolated_workspace();

    let (stdout, stderr, ok) = run_in(&root, &[
        "chain", "new",
        "--tip", ".", "docs/spec.md",
        "--tip", ".", "src/Service.java",
    ]);
    assert!(ok, "chain new failed:\n{stderr}");
    assert!(stdout.contains("Created chain:"), "missing uuid in output");

    let bilink_dir = root.join(".bilink");
    let files: Vec<_> = std::fs::read_dir(&bilink_dir).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("bilink"))
        .collect();
    assert_eq!(files.len(), 1, "direct link should create exactly one file");
}

#[test]
fn chain_new_two_layers_creates_two_files() {
    let (_tmp, root) = isolated_workspace();
    std::fs::create_dir_all(root.join(".estrato/impl")).unwrap();

    let (_, stderr, ok) = run_in(&root, &[
        "chain", "new",
        "--tip", ".", "docs/spec.md",
        "--tip", ".estrato/impl", "src/Service.java",
    ]);
    assert!(ok, "chain new failed:\n{stderr}");

    let count_bilinks = |dir: &std::path::Path| -> usize {
        std::fs::read_dir(dir).map(|rd| rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("bilink"))
            .count()
        ).unwrap_or(0)
    };

    assert_eq!(count_bilinks(&root.join(".bilink")), 1, "tip at root");
    assert_eq!(count_bilinks(&root.join(".estrato/impl/.bilink")), 1, "tip at impl");
}

// ─── 6. check ─────────────────────────────────────────────────────────────

#[test]
fn check_marks_whole_file_endpoint_ok() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", ".", "docs/spec.md",
        "--tip", ".", "src/Service.java",
    ]);

    let (_, stderr, ok) = run_in(&root, &["check", "."]);
    assert!(ok, "check failed with non-clean state:\n{stderr}");
    assert!(stderr.contains("all clean"), "expected 'all clean' message");
}

#[test]
fn check_marks_altered_after_file_change() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", ".", "docs/spec.md",
        "--tip", ".", "src/Service.java",
    ]);

    run_in(&root, &["check", "."]);

    std::fs::write(root.join("docs/spec.md"), "# Modified\n\nDifferent content.\n").unwrap();

    let (stdout, _, ok) = run_in(&root, &["check", "."]);
    assert!(!ok, "check should exit 1 when state is ALTERED");
    assert!(stdout.contains("ALTERED"), "expected ALTERED in output:\n{stdout}");
}

// ─── 7. chain list / chain status ──────────────────────────────────────────

#[test]
fn chain_list_shows_created_chain() {
    let (_tmp, root) = isolated_workspace();

    let (create_out, _, _) = run_in(&root, &[
        "chain", "new",
        "--tip", ".", "docs/spec.md",
        "--tip", ".", "src/Service.java",
    ]);

    let uuid = create_out.lines()
        .find(|l| l.starts_with("Created chain:"))
        .and_then(|l| l.split_whitespace().nth(2))
        .expect("no UUID in output");

    let (list_out, _, ok) = run_in(&root, &["chain", "list"]);
    assert!(ok, "chain list failed");
    assert!(list_out.contains(&uuid[..8]), "UUID prefix not in chain list:\n{list_out}");
}

#[test]
fn chain_status_shows_nodes() {
    let (_tmp, root) = isolated_workspace();
    std::fs::create_dir_all(root.join(".estrato/impl")).unwrap();

    let (create_out, _, ok) = run_in(&root, &[
        "chain", "new",
        "--tip", ".", "docs/spec.md",
        "--tip", ".estrato/impl", "src/Service.java",
    ]);
    assert!(ok);

    let uuid = create_out.lines()
        .find(|l| l.starts_with("Created chain:"))
        .and_then(|l| l.split_whitespace().nth(2))
        .expect("no UUID in output");

    let (status_out, _, ok) = run_in(&root, &["chain", "status", uuid]);
    assert!(ok, "chain status failed");
    assert!(status_out.contains("Chain:"), "expected chain header:\n{status_out}");
    assert!(status_out.contains("link.0"), "expected link.0 in output");
    assert!(status_out.contains("link.1"), "expected link.1 in output");
}

// ─── 8. get by file ────────────────────────────────────────────────────────

#[test]
fn get_by_file_returns_bilink_after_check() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", ".", "docs/spec.md",
        "--tip", ".", "src/Service.java",
    ]);

    run_in(&root, &["check", "."]);

    let (out, _, ok) = run_in(&root, &["get", "docs/spec.md"]);
    assert!(ok, "get by file failed");
    assert!(!out.is_empty(), "expected at least one bilink in output:\n{out}");
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
