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
fn capture_writes_a_capture_with_the_stable_anchor() {
    let (_tmp, root) = isolated_git_workspace();

    let (stdout, stderr, ok) = run_in(&root, &["capture", "src/Service.java", "2:5", "2:24"]);
    assert!(ok, "capture failed:
{stderr}");

    let uuid = stdout.trim();
    assert!(!uuid.is_empty(), "capture debe imprimir el uuid por stdout");

    let cap = fs::read_to_string(root.join(format!(".bilink/capture/{uuid}.yaml")))
        .expect("capture no fue escrito");
    assert!(cap.contains("file: src/Service.java"), "falta el archivo:
{cap}");
    assert!(cap.contains("Service"), "falta la clase en la query:
{cap}");
    assert!(cap.contains("run"),     "falta el método en la query:
{cap}");
    assert!(!cap.contains("hash"),   "un capture no guarda hashes:
{cap}");
}

// ─── 2. get matches sed ────────────────────────────────────────────────────

#[test]
#[ignore = "requires fixture persona-voting-impl.bilink — create with: bilinker chain new"]
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
fn capturing_the_same_fragment_twice_reuses_the_capture() {
    let (_tmp, root) = isolated_git_workspace();
    let args = &["capture", "src/Service.java", "2:5", "2:24"];

    let (out1, err1, ok1) = run_in(&root, args);
    assert!(ok1, "primera captura falló:
{err1}");
    let (out2, err2, ok2) = run_in(&root, args);
    assert!(ok2, "segunda captura falló:
{err2}");

    assert_eq!(out1.trim(), out2.trim(), "la misma referencia debe reusar el capture");
    assert!(err2.contains("reusado"), "la segunda debería reportar reuso:
{err2}");

    let n = fs::read_dir(root.join(".bilink/capture")).unwrap().count();
    assert_eq!(n, 1, "no debería haber dos captures para la misma referencia");
}

// ─── 4. drift detection ────────────────────────────────────────────────────

#[test]
fn the_reference_survives_a_content_change() {
    let (_tmp, root) = isolated_git_workspace();
    let args = &["capture", "src/Service.java", "2:5", "2:24"];

    let (out1, err1, ok1) = run_in(&root, args);
    assert!(ok1, "captura inicial falló:
{err1}");

    // Cambia el cuerpo del método, no su firma: la referencia apunta al nodo AST,
    // así que debe seguir siendo la misma. Es la promesa central de la herramienta.
    fs::write(root.join("src/Service.java"),
        "public class Service {
    public void run() { log(); }
}
").unwrap();

    let (out2, err2, ok2) = run_in(&root, args);
    assert!(ok2, "captura posterior falló:
{err2}");
    assert_eq!(out1.trim(), out2.trim(),
               "cambiar el contenido no debería cambiar la referencia");
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

/// Como `isolated_workspace`, pero con los archivos commiteados.
///
/// `capture` exige historial git: un bilink solo se puede crear sobre algo que
/// git pueda rastrear.
fn isolated_git_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let (tmp, root) = isolated_workspace();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "init"],
    ] {
        std::process::Command::new("git")
            .current_dir(&root).args(&args).output().unwrap();
    }
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
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);
    assert!(ok, "chain new failed:\n{stderr}");
    assert!(stdout.contains("Created chain:"), "missing uuid in output");

    let bilink_dir = root.join(".bilink");
    let files: Vec<_> = std::fs::read_dir(&bilink_dir).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("yaml"))
        .collect();
    assert_eq!(files.len(), 1, "direct link should create exactly one file");
}

#[test]
fn chain_new_two_layers_creates_two_files() {
    let (_tmp, root) = isolated_workspace();
    std::fs::create_dir_all(root.join(".stratum/impl")).unwrap();

    let (_, stderr, ok) = run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", ">impl/src/Service.java",
    ]);
    assert!(ok, "chain new failed:\n{stderr}");

    let count_bilinks = |dir: &std::path::Path| -> usize {
        std::fs::read_dir(dir).map(|rd| rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("yaml"))
            .count()
        ).unwrap_or(0)
    };

    assert_eq!(count_bilinks(&root.join(".bilink")), 1, "tip at root");
    assert_eq!(count_bilinks(&root.join(".stratum/impl/.bilink")), 1, "tip at impl");
}

// ─── 6. check ─────────────────────────────────────────────────────────────

#[test]
fn check_marks_new_chain_as_pending() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);

    let (stdout, _, ok) = run_in(&root, &["check", "."]);
    // No accepted entries yet → PENDING
    assert!(!ok, "check should exit 1 on PENDING state");
    assert!(stdout.contains("PENDING"), "expected PENDING in output:\n{stdout}");
}

#[test]
fn check_marks_altered_after_accept_and_file_change() {
    let (_tmp, root) = isolated_git_workspace();

    run_in(&root, &["chain", "new", "--tip", "docs/spec.md:1:1", "--tip", "src/Service.java:2:5"]);
    run_in(&root, &["check", "."]);

    // Aceptar de verdad, no simularlo: es lo que escribe el bloque `accepted`.
    let (_, stderr, ok) = run_in(&root, &["accept", "."]);
    assert!(ok, "accept failed:\n{stderr}");

    let (stdout, _, ok) = run_in(&root, &["check", "."]);
    assert!(ok, "tras aceptar tiene que quedar limpio:\n{stdout}");

    // El contenido cambia **bajo el mismo anchor**: el heading sigue siendo el que
    // la query nombra, así que el capture resuelve y lo que difiere es el hash.
    // Cambiar el heading sería otra cosa —el anchor se fue— y daría UNRESOLVED.
    fs::write(root.join("docs/spec.md"), "# Spec\n\nContenido distinto.\n").unwrap();

    let (stdout, _, ok) = run_in(&root, &["check", "."]);
    assert!(!ok, "ALTERED tiene que salir con 1");
    assert!(stdout.contains("ALTERED"), "esperaba ALTERED:\n{stdout}");
}

// ─── 7. chain list / chain status ──────────────────────────────────────────

#[test]
fn chain_list_shows_created_chain() {
    let (_tmp, root) = isolated_workspace();

    let (create_out, _, _) = run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
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
    std::fs::create_dir_all(root.join(".stratum/impl")).unwrap();

    let (create_out, _, ok) = run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", ">impl/src/Service.java",
    ]);
    assert!(ok);

    let uuid = create_out.lines()
        .find(|l| l.starts_with("Created chain:"))
        .and_then(|l| l.split_whitespace().nth(2))
        .expect("no UUID in output");

    let (status_out, _, ok) = run_in(&root, &["chain", "status", uuid]);
    assert!(ok, "chain status failed");
    assert!(status_out.contains("Cadena:"), "falta el encabezado:\n{status_out}");
    assert!(status_out.contains("endpoint.0"), "falta endpoint.0:\n{status_out}");
    assert!(status_out.contains("endpoint.1"), "falta endpoint.1:\n{status_out}");
}

// ─── 8. get by file ────────────────────────────────────────────────────────

#[test]
fn get_by_file_returns_bilink_after_check() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);

    run_in(&root, &["check", "."]);

    let (out, _, ok) = run_in(&root, &["get", "docs/spec.md"]);
    assert!(ok, "get by file failed");
    assert!(!out.is_empty(), "expected at least one bilink in output:\n{out}");
}

// ─── 9. bilinker index ────────────────────────────────────────────────────

#[test]
fn index_build_creates_index_file() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);

    let (stdout, stderr, ok) = run_in(&root, &["index", "build"]);
    assert!(ok, "index build failed:\n{stderr}");
    assert!(stdout.contains("entries"), "expected entry count in output:\n{stdout}");

    let index_path = root.join(".bilink/index/index");
    assert!(index_path.exists(), ".bilink/index/index was not created");

    let contents = std::fs::read_to_string(&index_path).unwrap();
    assert!(contents.contains("docs/spec.md"),   "spec.md missing from index");
    assert!(contents.contains("src/Service.java"), "Service.java missing from index");
}

#[test]
fn index_gitignore_contains_index_entry() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);
    run_in(&root, &["index", "build"]);

    let gi = std::fs::read_to_string(root.join(".bilink/.gitignore")).unwrap();
    assert!(gi.contains("index/"), ".gitignore missing index/");
    assert!(gi.contains(".pending/"), ".gitignore missing .pending/");
}

#[test]
fn index_status_ok_after_build() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);
    run_in(&root, &["index", "build"]);

    let (stdout, _, ok) = run_in(&root, &["index", "status"]);
    assert!(ok, "index status should exit 0 when OK");
    assert!(stdout.contains("OK"), "expected OK in status output:\n{stdout}");
}

#[test]
fn index_status_stale_after_new_chain() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);
    run_in(&root, &["index", "build"]);

    // Create a new chain after the index was built
    std::fs::write(root.join("docs/other.md"), "# Other\n").unwrap();
    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/other.md",
        "--tip", "src/Service.java",
    ]);

    let (stdout, _, ok) = run_in(&root, &["index", "status"]);
    assert!(!ok, "index status should exit 1 when stale");
    assert!(stdout.contains("STALE"), "expected STALE in status output:\n{stdout}");
}

#[test]
fn index_status_missing_when_never_built() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);

    let (stdout, _, ok) = run_in(&root, &["index", "status"]);
    assert!(!ok, "index status should exit 1 when missing");
    assert!(stdout.contains("MISSING"), "expected MISSING in status output:\n{stdout}");
}

#[test]
fn index_recursive_covers_all_layers() {
    let (_tmp, root) = isolated_workspace();
    std::fs::create_dir_all(root.join(".stratum/impl")).unwrap();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", ">impl/src/Service.java",
    ]);

    let (stdout, stderr, ok) = run_in(&root, &["index", "build", "--recursive"]);
    assert!(ok, "index build --recursive failed:\n{stderr}");

    // Both layers should have an index
    assert!(root.join(".bilink/index/index").exists(), "root layer index missing");
    assert!(root.join(".stratum/impl/.bilink/index/index").exists(), "impl layer index missing");
    assert_eq!(stdout.lines().count(), 2, "expected two index lines in output:\n{stdout}");
}

// ─── 8. la salida de check es fiel al estado ───────────────────────────────

/// Crea una cadena sobre `docs/spec.md`, la acepta, y devuelve `(root, uuid8)`.
fn accepted_chain_on_spec(root: &std::path::Path) -> String {
    let (stdout, stderr, ok) = run_in(root, &[
        "chain", "new",
        "--tip", "docs/spec.md:1:1",
        "--tip", "src/Service.java:2:5",
    ]);
    assert!(ok, "chain new failed:\n{stderr}");
    let uuid = stdout.lines()
        .find_map(|l| l.strip_prefix("Created chain: "))
        .expect("uuid en la salida de chain new")
        .trim()
        .to_string();

    run_in(root, &["check", "."]);
    let (_, stderr, ok) = run_in(root, &["accept", "."]);
    assert!(ok, "accept failed:\n{stderr}");

    uuid[..8].to_string()
}

/// EXPANDED no está OK, así que se imprime — aunque no haga fallar a `check`.
///
/// Fija la separación entre "qué se muestra" y "qué código de salida se
/// devuelve": el filtro de salida excluye OK, no enumera estados. Si volviera a
/// enumerarlos, cada estado con auto-fix quedaría mudo.
#[test]
fn check_reports_an_expanded_endpoint() {
    let (_tmp, root) = isolated_git_workspace();
    let uuid8 = accepted_chain_on_spec(&root);

    // La sección crece alrededor de lo aceptado, sin tocarlo → EXPANDED.
    let spec = root.join("docs/spec.md");
    let grown = fs::read_to_string(&spec).unwrap() + "\nUna línea más.\n";
    fs::write(&spec, grown).unwrap();

    let (stdout, _stderr, ok) = run_in(&root, &["check", "."]);
    assert!(stdout.contains(&uuid8),
        "un endpoint EXPANDED tiene que aparecer en la salida:\n{stdout}");
    assert!(stdout.contains("EXPANDED"),
        "esperaba EXPANDED en la salida:\n{stdout}");
    assert!(ok, "EXPANDED tiene auto-fix: no debe cambiar el código de salida");
}

/// Revertir la edición devuelve el endpoint a OK, sin pasar por `accept`.
///
/// El fast-path de `check` pregunta si el archivo cambió desde `commit.N`. Una
/// edición y su reversión se cancelan, así que la respuesta es "no" — y el
/// estado cacheado, calculado sobre el árbol de trabajo sucio, describiría un
/// contenido que ya no está. Por eso solo se conserva un OK.
#[test]
fn check_clears_a_stale_state_when_the_edit_is_reverted() {
    let (_tmp, root) = isolated_git_workspace();
    let uuid8 = accepted_chain_on_spec(&root);

    let spec = root.join("docs/spec.md");
    let original = fs::read_to_string(&spec).unwrap();

    fs::write(&spec, original.clone() + "\nUna línea más.\n").unwrap();
    let (stdout, _, _) = run_in(&root, &["check", "."]);
    assert!(stdout.contains(&uuid8), "el estado sucio tiene que verse primero:\n{stdout}");

    fs::write(&spec, &original).unwrap();
    let (stdout, _, ok) = run_in(&root, &["check", "."]);
    assert!(!stdout.contains(&uuid8),
        "revertir la edición tiene que devolver el endpoint a OK:\n{stdout}");
    assert!(ok);
}

// ─── 9. la query tiene que identificar el fragmento ────────────────────────

fn write_and_commit(root: &std::path::Path, rel: &str, content: &str) {
    fs::write(root.join(rel), content).unwrap();
    for args in [vec!["add", "-A"], vec!["commit", "-qm", "fixture"]] {
        std::process::Command::new("git")
            .current_dir(root).args(&args).output().unwrap();
    }
}

/// Un `impl` de Rust no tiene campo `name`: lo distinguen el tipo y el trait.
///
/// Sin los dos, `impl Foo` y `impl Default for Foo` producen la misma query
/// —`(impl_item type: … "Foo")`— y `capture` devolvería el primero.
#[test]
fn capture_disambiguates_a_rust_impl_block() {
    let (_tmp, root) = isolated_git_workspace();
    write_and_commit(&root, "src/lib.rs", concat!(
        "pub struct Foo;\n",
        "\n",
        "impl Foo {\n",
        "    pub fn inherent(&self) {}\n",
        "}\n",
        "\n",
        "impl Default for Foo {\n",
        "    fn default() -> Self { Foo }\n",
        "}\n",
    ));

    let (stdout, stderr, ok) = run_in(&root, &["capture", "src/lib.rs", "7:1", "7:1"]);
    assert!(ok, "capture del impl de trait falló:\n{stderr}");

    let uuid = stdout.trim();
    let cap = fs::read_to_string(root.join(format!(".bilink/capture/{uuid}.yaml"))).unwrap();
    assert!(cap.contains("trait:"), "la query tiene que discriminar por trait:\n{cap}");
    assert!(cap.contains("\"Default\""), "falta el trait en la query:\n{cap}");

    // Y resuelve al impl del trait, no al inherente.
    let (stdout, stderr, ok) = run_in(&root, &[
        "chain", "new", "--tip", "src/lib.rs:7:1", "--tip", "docs/spec.md",
    ]);
    assert!(ok, "chain new failed:\n{stderr}");
    let chain = stdout.lines()
        .find_map(|l| l.strip_prefix("Created chain: "))
        .expect("uuid").trim().to_string();

    let (shown, stderr, ok) = run_in(&root, &["get", &format!("{}.0", &chain[..8])]);
    assert!(ok, "get failed:\n{stderr}");
    assert!(shown.contains("impl Default for Foo"),
        "el capture tiene que apuntar al impl del trait:\n{shown}");
    assert!(!shown.contains("pub fn inherent"),
        "apuntó al impl inherente:\n{shown}");
}

/// Un ancla sin nada que la distinga no se escribe: se falla.
///
/// `(line_comment) @target` matchea el primer comentario del archivo. Escribirlo
/// daría un capture que apunta a otra cosa y que `check` reporta en OK.
#[test]
fn capture_refuses_an_anchor_it_cannot_identify() {
    let (_tmp, root) = isolated_git_workspace();
    write_and_commit(&root, "src/notes.rs", concat!(
        "// primero\n",
        "pub fn a() {}\n",
        "// segundo\n",
        "pub fn b() {}\n",
    ));

    let (_stdout, stderr, ok) = run_in(&root, &["capture", "src/notes.rs", "3:1", "3:1"]);
    assert!(!ok, "capturar un comentario ambiguo tiene que fallar");
    assert!(stderr.contains("line_comment"),
        "el error tiene que nombrar el ancla que no se puede distinguir:\n{stderr}");

    let cap_dir = root.join(".bilink/capture");
    let written = fs::read_dir(&cap_dir).map(|d| d.count()).unwrap_or(0);
    assert_eq!(written, 0, "no se debe escribir ningún capture cuando la query es ambigua");
}

// ─── 10. el endpoint issue ─────────────────────────────────────────────────

/// `issue <id>` encuentra el ítem sin que el endpoint lleve el tipo.
///
/// Los ítems del worklist son archivos sueltos `<id>.<tipo>.md` en un solo
/// directorio, así que se busca por prefijo. Dejar el tipo afuera es lo que hace
/// que el vínculo sobreviva a recolgar el ítem de otro padre: eso cambia un campo
/// del ítem, no el nombre de su archivo.
#[test]
fn an_issue_endpoint_resolves_by_id_whatever_the_item_type() {
    let (_tmp, root) = isolated_git_workspace();
    fs::create_dir_all(root.join(".stratum/worklist")).unwrap();
    write_and_commit(&root, ".stratum/worklist/3a.user-story.md",
        "---\ntitle: una story\nstatus: open\n---\n\nCuerpo.\n");

    let (stdout, stderr, ok) = run_in(&root, &["capture", "docs/spec.md", "1:1", "1:1"]);
    assert!(ok, "capture failed:\n{stderr}");
    let cap = stdout.trim();

    let uuid = "aaaaaaaa-0000-4000-8000-000000000001";
    fs::write(root.join(format!(".bilink/{uuid}.yaml")),
        format!("endpoint:\n  0: {{link: capture {cap}}}\n  1: {{link: issue 3a}}\n")).unwrap();

    let (stdout, _, _) = run_in(&root, &["check", "."]);
    assert!(stdout.contains("aaaaaaaa"), "el bilink tiene que aparecer:\n{stdout}");
    assert!(!stdout.contains("TODO"),
        "TODO significa que no encontró el ítem — un `.user-story.md` es un ítem válido:\n{stdout}");
    assert!(stdout.contains("PENDING, PENDING"),
        "los dos extremos resueltos y sin aceptar:\n{stdout}");
}

/// Un id que no existe es TODO, no un panic ni un OK silencioso.
#[test]
fn an_unknown_issue_id_is_todo() {
    let (_tmp, root) = isolated_git_workspace();
    fs::create_dir_all(root.join(".stratum/worklist")).unwrap();

    let (stdout, stderr, ok) = run_in(&root, &["capture", "docs/spec.md", "1:1", "1:1"]);
    assert!(ok, "capture failed:\n{stderr}");
    let cap = stdout.trim();

    let uuid = "bbbbbbbb-0000-4000-8000-000000000001";
    fs::write(root.join(format!(".bilink/{uuid}.yaml")),
        format!("endpoint:\n  0: {{link: capture {cap}}}\n  1: {{link: issue zz9}}\n")).unwrap();

    let (stdout, _, _) = run_in(&root, &["check", "."]);
    assert!(stdout.contains("TODO"), "un id inexistente tiene que dar TODO:\n{stdout}");
}

// ─── helpers ───────────────────────────────────────────────────────────────

