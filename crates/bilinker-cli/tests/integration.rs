use std::fs;
use std::path::{Path, PathBuf};
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
fn the_derived_directories_are_left_out_of_git() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);
    run_in(&root, &["index", "build"]);

    let gi = std::fs::read_to_string(root.join(".bilink/.gitignore")).unwrap();
    assert!(gi.contains("cache/"), "falta cache/ en .gitignore:\n{gi}");
    assert!(gi.contains("index/"), "falta index/ en .gitignore:\n{gi}");
}

/// La regla no espera a que alguien corra `index`: `check` escribe la cache, y
/// escribir un derivado sin declararlo ignorado es lo que lo mete en git.
#[test]
fn check_alone_leaves_the_cache_out_of_git() {
    let (_tmp, root) = isolated_workspace();

    run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java",
    ]);
    run_in(&root, &["check", "."]);

    assert!(root.join(".bilink/cache/state").exists(), "check no escribió la cache");
    let gi = std::fs::read_to_string(root.join(".bilink/.gitignore"))
        .expect("check escribió cache/ sin declararla ignorada");
    assert!(gi.contains("cache/"), "falta cache/ en .gitignore:\n{gi}");
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


// ─── 11. los escenarios de check ───────────────────────────────────────────
//
// Cada uno implementa un escenario de `subsystems/bilinker/scenarios/check.yaml`,
// y el nombre de la función es el que su campo `impl:` nombra. Son de integración
// y no unitarios porque lo que el escenario describe es el estado que `check`
// reporta, que sale del binario y de la cache — no de una función.

/// Una capa con un bilink aceptado sobre `docs/spec.md` y `src/Service.java`.
fn accepted_layer() -> (tempfile::TempDir, PathBuf, String) {
    let (tmp, root) = isolated_git_workspace();
    let (stdout, stderr, ok) = run_in(&root, &[
        "chain", "new", "--tip", "docs/spec.md:1:1", "--tip", "src/Service.java:2:5",
    ]);
    assert!(ok, "chain new failed:\n{stderr}");
    let uuid = stdout.lines()
        .find_map(|l| l.strip_prefix("Created chain: "))
        .expect("uuid").trim().to_string();

    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", "."]);
    assert!(ok, "accept failed:\n{stderr}");
    (tmp, root, uuid)
}

fn check_states(root: &Path) -> String {
    run_in(root, &["check", "."]).0
}

/// `check-ok-after-accept` — aceptado y sin cambios reporta OK.
#[test]
fn check_whole_file_ok_when_hash_matches() {
    let (_t, root, _u) = accepted_layer();
    let (out, _, ok) = run_in(&root, &["check", "."]);
    assert!(ok, "sin cambios tiene que salir con 0:\n{out}");
    assert!(out.trim().is_empty(), "y no imprimir nada:\n{out}");
}

/// `check-altered-structural` — el contenido cambia bajo el mismo anchor.
#[test]
fn check_whole_file_altered_when_hash_differs() {
    let (_t, root, _u) = accepted_layer();
    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtro contenido.\n").unwrap();
    assert!(check_states(&root).contains("ALTERED"));
}

/// `check-chain-dirty-propagation` — el vecino se re-acepta y la copia queda vieja.
#[test]
fn check_layer_chain_dirty_when_hash_differs() {
    let (_tmp, root) = isolated_git_workspace();
    fs::create_dir_all(root.join(".stratum/impl/src")).unwrap();
    write_and_commit(&root, ".stratum/impl/src/lib.rs", "pub fn run() {}\n");

    run_in(&root, &["chain", "new",
        "--tip", "docs/spec.md:1:1", "--tip", ">impl/src/lib.rs:1:1"]);

    // El vecino primero: un endpoint `path` copia lo que su vecino aprobó, así que
    // no hay nada que copiar hasta que el vecino aceptó.
    let impl_layer = root.join(".stratum/impl");
    run_in(&impl_layer, &["check", "."]);
    run_in(&impl_layer, &["accept", "."]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);
    assert!(run_in(&root, &["check", "."]).2, "la cadena tiene que arrancar limpia");

    // El fragmento del otro extremo cambia y se re-acepta ahí.
    write_and_commit(&root, ".stratum/impl/src/lib.rs", "pub fn run() { let x = 1; }\n");
    run_in(&impl_layer, &["check", "."]);
    run_in(&impl_layer, &["accept", "."]);

    // Desde la capa spec, el endpoint `path` ve que su copia dejó de coincidir.
    assert!(check_states(&root).contains("CHAIN_DIRTY"),
        "el vecino se re-aceptó:\n{}", check_states(&root));
}

/// `check-pending-layer-exists` — la capa existe y nadie aceptó.
#[test]
fn check_layer_first_time_is_pending() {
    let (_tmp, root) = isolated_git_workspace();
    fs::create_dir_all(root.join(".stratum/impl/src")).unwrap();
    write_and_commit(&root, ".stratum/impl/src/lib.rs", "pub fn run() {}\n");

    run_in(&root, &["chain", "new",
        "--tip", "docs/spec.md:1:1", "--tip", ">impl/src/lib.rs:1:1"]);
    assert!(check_states(&root).contains("PENDING, PENDING"));
}

/// `check-todo-layer-missing` — la capa apuntada no existe todavía.
///
/// Es una intención declarada, no un error: por eso TODO y no BROKEN.
#[test]
fn check_layer_todo_when_adjacent_missing_and_no_hash() {
    let (_tmp, root) = isolated_git_workspace();
    let (stdout, _, _) = run_in(&root, &["capture", "docs/spec.md", "1:1", "1:1"]);
    let cap = stdout.trim();

    // A mano y no con `chain new`, que escribe el bilink en las dos capas: lo que se
    // quiere construir es justamente el caso en que la otra capa todavía no existe.
    fs::write(root.join(".bilink/aaaa0000-0000-4000-8000-000000000001.yaml"),
        format!("endpoint:\n  0: {{link: capture {cap}}}\n  1: {{link: path >impl}}\n")).unwrap();

    let out = check_states(&root);
    assert!(out.contains("TODO"), "la capa vecina no existe todavía:\n{out}");
}

/// `check-unresolved-file-missing` — el archivo se fue; el bilink no puede evaluarse.
#[test]
fn check_unresolved_when_file_missing() {
    let (_t, root, _u) = accepted_layer();
    fs::remove_file(root.join("docs/spec.md")).unwrap();
    assert!(check_states(&root).contains("UNRESOLVED"));
}

/// `check-moved-after-git-rename` — git detecta el rename.
#[test]
fn check_detects_moved_after_git_rename() {
    let (_t, root, _u) = accepted_layer();
    git(&root, &["mv", "docs/spec.md", "docs/renombrada.md"]);

    // El endpoint no puede evaluarse; el detalle —MOVED— lo lleva el capture, y
    // `apply` es quien lo sabe leer.
    assert!(check_states(&root).contains("UNRESOLVED"));
    let (out, _, _) = run_in(&root, &["apply", "--dry-run"]);
    assert!(out.contains("MOVED"), "apply tiene que ver el rename:\n{out}");
}

/// `check-deleted-when-file-removed` — borrado rastreable en git.
///
/// Distingue "alguien borró esto" de "esta referencia nunca ancló a nada".
#[test]
fn check_detects_deleted_when_file_removed_from_git() {
    let (_t, root, _u) = accepted_layer();
    git(&root, &["rm", "-q", "docs/spec.md"]);
    assert!(check_states(&root).contains("UNRESOLVED"));
}

/// `check-reanchored-by-similarity` — el anchor se renombró, el cuerpo quedó.
#[test]
fn check_detects_reanchored_when_anchor_is_renamed() {
    let (_tmp, root) = isolated_git_workspace();
    write_and_commit(&root, "src/lib.rs", concat!(
        "pub fn procesar() {\n",
        "    let x = 1;\n    let y = 2;\n    let z = 3;\n",
        "    println!(\"{} {} {}\", x, y, z);\n}\n"));
    run_in(&root, &["chain", "new", "--tip", "src/lib.rs:1:1", "--tip", "docs/spec.md"]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    write_and_commit(&root, "src/lib.rs", concat!(
        "pub fn transformar() {\n",
        "    let x = 1;\n    let y = 2;\n    let z = 3;\n",
        "    println!(\"{} {} {}\", x, y, z);\n}\n"));

    let (out, _, _) = run_in(&root, &["apply", "--dry-run"]);
    assert!(out.contains("REANCHORED"), "el anchor se renombró:\n{out}");
}

/// `check-reanchored-tolerates-an-edit` — renombre más un cambio menor.
#[test]
fn reanchored_survives_a_rename_plus_small_edit() {
    let (_tmp, root) = isolated_git_workspace();
    write_and_commit(&root, "src/lib.rs", concat!(
        "pub fn procesar() {\n",
        "    let x = 1;\n    let y = 2;\n    let z = 3;\n",
        "    println!(\"{} {} {}\", x, y, z);\n}\n"));
    run_in(&root, &["chain", "new", "--tip", "src/lib.rs:1:1", "--tip", "docs/spec.md"]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    // Renombrada **y** con una línea distinta: la similitud tiene que aguantar.
    write_and_commit(&root, "src/lib.rs", concat!(
        "pub fn transformar() {\n",
        "    let x = 1;\n    let y = 2;\n    let z = 99;\n",
        "    println!(\"{} {} {}\", x, y, z);\n}\n"));

    let (out, _, _) = run_in(&root, &["apply", "--dry-run"]);
    assert!(out.contains("REANCHORED"), "un cambio menor no debería romperlo:\n{out}");
}

/// `check-ambiguous-stays-unanchored` — dos candidatos parejos, ninguno gana.
///
/// Hace falta un margen del 15% sobre el segundo: ante un empate es preferible que
/// lo mire un humano antes que reanclar al nodo equivocado.
#[test]
fn ambiguous_candidates_stay_unanchored() {
    let (_tmp, root) = isolated_git_workspace();
    write_and_commit(&root, "src/lib.rs", concat!(
        "pub fn procesar() {\n    let x = 1;\n    let y = 2;\n    x + y\n}\n"));
    run_in(&root, &["chain", "new", "--tip", "src/lib.rs:1:1", "--tip", "docs/spec.md"]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    // Dos candidatos idénticos entre sí: ninguno le saca margen al otro.
    write_and_commit(&root, "src/lib.rs", concat!(
        "pub fn uno() {\n    let x = 1;\n    let y = 2;\n    x + y\n}\n",
        "pub fn dos() {\n    let x = 1;\n    let y = 2;\n    x + y\n}\n"));

    let (out, _, _) = run_in(&root, &["apply", "--dry-run"]);
    assert!(!out.contains("REANCHORED"), "ante un empate no debe reanclar:\n{out}");
}

/// `check-expanded-when-fragment-grows` — creció alrededor de lo aceptado.
///
/// La frontera con DISPLACED es un test de subcadena contra el texto aceptado, no
/// un umbral: que el fragmento lo contenga verbatim implica que su AST no cambió.
#[test]
fn check_detects_expanded_when_fragment_grows_around_accepted_text() {
    let (_t, root, _u) = accepted_layer();
    let original = fs::read_to_string(root.join("docs/spec.md")).unwrap();
    fs::write(root.join("docs/spec.md"), original + "\nUna línea más.\n").unwrap();
    assert!(check_states(&root).contains("EXPANDED"),
        "el fragmento contiene lo aceptado y algo más:\n{}", check_states(&root));
}

/// `check-altered-when-accepted-text-changed` — lo aceptado ya no aparece verbatim.
#[test]
fn check_says_altered_when_accepted_text_changed() {
    let (_t, root, _u) = accepted_layer();
    fs::write(root.join("docs/spec.md"), "# Spec\n\nReescrito de cero.\n").unwrap();
    assert!(check_states(&root).contains("ALTERED"));
}

/// `check-exit-zero-all-ok` y `check-exit-one-any-altered`.
#[test]
fn check_exit_code_follows_the_states() {
    let (_t, root, _u) = accepted_layer();
    assert!(run_in(&root, &["check", "."]).2, "todo OK sale con 0");

    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtra cosa.\n").unwrap();
    assert!(!run_in(&root, &["check", "."]).2, "un ALTERED sale con 1");
}

fn git(root: &Path, args: &[&str]) {
    std::process::Command::new("git").current_dir(root).args(args).output().unwrap();
}

/// `apply` no aplica un fix derivado de la cache: re-resuelve y compara.
///
/// El estado cacheado lo escribió el último `check`, y el archivo pudo cambiar
/// después. Corregir contra esa foto vieja repuntaría a una ubicación que ya no es.
#[test]
fn apply_discards_a_fix_when_the_cache_went_stale() {
    let (_t, root, _u) = accepted_layer();

    // El fragmento crece: check lo deja en EXPANDED, con fix disponible.
    let original = fs::read_to_string(root.join("docs/spec.md")).unwrap();
    fs::write(root.join("docs/spec.md"), original + "\nUna línea más.\n").unwrap();
    assert!(check_states(&root).contains("EXPANDED"));

    // Ahora el contenido cambia de verdad, **sin** volver a correr check: la cache
    // sigue diciendo EXPANDED y la realidad ya es otra.
    fs::write(root.join("docs/spec.md"), "# Spec\n\nReescrito de cero.\n").unwrap();

    let (out, err, _) = run_in(&root, &["apply", "--dry-run"]);
    assert!(!out.contains("EXPANDED"), "no debe ofrecer un fix contra la cache vieja:\n{out}");
    assert!(err.contains("cache") || err.contains("check"),
        "y tiene que decir por qué lo descartó:\n{err}");
}

/// Un fix que ya no hace falta se omite en silencio.
#[test]
fn apply_silently_skips_a_fix_that_is_no_longer_needed() {
    let (_t, root, _u) = accepted_layer();
    let original = fs::read_to_string(root.join("docs/spec.md")).unwrap();

    fs::write(root.join("docs/spec.md"), original.clone() + "\nUna línea más.\n").unwrap();
    assert!(check_states(&root).contains("EXPANDED"));

    // Se revierte: el endpoint vuelve a estar OK aunque la cache diga otra cosa.
    fs::write(root.join("docs/spec.md"), &original).unwrap();

    let (out, err, _) = run_in(&root, &["apply", "--dry-run"]);
    assert!(!out.contains("EXPANDED"), "el fix ya no hace falta:\n{out}");
    assert!(!err.contains("warn"), "y que algo se arregle solo no es una anomalía:\n{err}");
}


// ─── task `y`: crear una cadena entre subsistemas ───────────────────────────

/// Un tip puede atravesar directorios comunes antes de bajar a una capa.
///
/// `subsystems/bilinker>impl` es la forma de este proyecto: la capa raíz tiene
/// las specs de varios subsistemas y cada uno su impl abajo. Sin esto no se puede
/// crear una sola cadena de accreta desde la raíz.
#[test]
fn a_tip_can_cross_plain_directories_before_a_layer() {
    let (_t, root) = isolated_git_workspace();
    let sub = root.join("subsystems/thing");
    fs::create_dir_all(sub.join(".stratum/impl/src")).unwrap();
    fs::create_dir_all(sub.join("concepts")).unwrap();
    fs::write(sub.join("concepts/spec.md"), "# Spec\n\nContenido.\n").unwrap();
    fs::write(sub.join(".stratum/impl/src/lib.rs"), "pub fn run() {}\n").unwrap();
    for args in [vec!["add", "-A"], vec!["commit", "-qm", "sub"]] {
        std::process::Command::new("git").current_dir(&root).args(&args).output().unwrap();
    }

    let (out, err, ok) = run_in(&root, &[
        "chain", "new",
        "--tip", "subsystems/thing/concepts/spec.md",
        "--tip", "subsystems/thing>impl/src/lib.rs:1:1",
    ]);
    assert!(ok, "chain new no alcanzó la capa del subsistema:\n{err}");
    assert!(out.contains("Created chain"), "{out}{err}");
    assert!(sub.join(".stratum/impl/.bilink").exists(),
        "no escribió el bilink del otro extremo");
}

/// `capture` sin selección captura el archivo entero.
///
/// El formato lo contempla desde siempre —`query` ausente— y es la forma más
/// usada del lado de las specs, donde el fragmento suele ser el documento.
#[test]
fn capture_without_a_selection_takes_the_whole_file() {
    let (_t, root) = isolated_git_workspace();

    let (out, err, ok) = run_in(&root, &["capture", "docs/spec.md"]);
    assert!(ok, "capture sin selección falló:\n{err}");

    let id = out.trim();
    let cap = fs::read_to_string(root.join(format!(".bilink/capture/{id}.yaml"))).unwrap();
    assert!(cap.contains("file: docs/spec.md"), "{cap}");
    assert!(!cap.contains("query:"),  "el archivo entero no lleva query:\n{cap}");
    assert!(!cap.contains("offset:"), "el archivo entero no lleva offset:\n{cap}");
}


/// `remove` encuentra el bilink por prefijo, como todos los demás comandos.
#[test]
fn remove_finds_a_bilink_by_its_prefix() {
    let (_t, root) = isolated_git_workspace();
    run_in(&root, &["chain", "new", "--tip", "docs/spec.md", "--tip", "src/Service.java:2:5"]);
    let uuid = std::fs::read_dir(root.join(".bilink")).unwrap()
        .filter_map(|e| e.ok())
        .find_map(|e| e.file_name().to_str()?.strip_suffix(".yaml").map(str::to_owned))
        .unwrap();

    let (_o, err, ok) = run_in(&root, &["remove", &uuid[..8]]);
    assert!(ok, "remove no encontró un bilink que existe:\n{err}");
    assert!(!root.join(format!(".bilink/{uuid}.yaml")).exists(), "no lo borró");
}

/// Un estado con fix que `apply` no puede calcular se reporta, no se omite.
///
/// Un `offset` es relativo al nodo, así que un capture de archivo completo —que
/// no tiene nodo— no puede recibir uno. `check` igual dice EXPANDED, que es un
/// estado con fix. Callarse deja a `check` reportando un fix disponible y a
/// `apply` contestando que no hay nada que hacer, sin nadie que lo explique.
#[test]
fn apply_says_why_it_cannot_compute_a_fix() {
    let (_t, root) = isolated_git_workspace();

    // Sin posición, el tip captura el archivo entero: el capture sale sin query.
    run_in(&root, &["chain", "new", "--tip", "docs/spec.md", "--tip", "src/Service.java:2:5"]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    let spec = fs::read_to_string(root.join("docs/spec.md")).unwrap();
    fs::write(root.join("docs/spec.md"), format!("{spec}\nY algo más.\n")).unwrap();
    commit(&root, "creció");

    let states = check_states(&root);
    assert!(states.contains("EXPANDED"), "el escenario no se armó:\n{states}");

    let (out, err, _) = run_in(&root, &["apply", "--dry-run"]);
    assert!(err.contains("warn"),
            "apply se calló un fix que no puede calcular:\n{out}{err}");
    assert!(err.contains("archivo completo"), "y no dijo por qué:\n{err}");
}

fn commit(root: &Path, msg: &str) {
    for args in [vec!["add", "-A"], vec!["commit", "-qm", msg]] {
        std::process::Command::new("git").current_dir(root).args(&args).output().unwrap();
    }
}

// ─── task `4`: kind y name dejan de perderse ────────────────────────────────

/// `chain new` puebla `kind` y `name` sin que nadie abra el YAML.
///
/// Son campos de declaración, y todo archivo de bilinker sale de un comando: sin
/// los flags la única forma de escribirlos sería a mano, que es justo lo que el
/// formato no le pide a nadie.
#[test]
fn chain_new_writes_the_declaration_fields() {
    let (_t, root) = isolated_git_workspace();

    let (_o, err, ok) = run_in(&root, &[
        "chain", "new",
        "--tip", "docs/spec.md",
        "--tip", "src/Service.java:2:5",
        "--kind", "governs",
        "--name.0", "la-decision",
        "--name.1", "lo-gobernado",
    ]);
    assert!(ok, "chain new con --kind falló:\n{err}");

    let bl = std::fs::read_dir(root.join(".bilink")).unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".yaml")))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .unwrap();

    assert!(bl.contains("kind: governs"),      "falta kind:\n{bl}");
    assert!(bl.contains("name: la-decision"),  "falta el name del endpoint 0:\n{bl}");
    assert!(bl.contains("name: lo-gobernado"), "falta el name del endpoint 1:\n{bl}");
}

/// Sin los flags no aparece ningún campo: son opcionales, no vacíos.
#[test]
fn chain_new_omits_the_declaration_fields_when_not_given() {
    let (_t, root) = isolated_git_workspace();
    run_in(&root, &["chain", "new", "--tip", "docs/spec.md", "--tip", "src/Service.java:2:5"]);

    let bl = std::fs::read_dir(root.join(".bilink")).unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".yaml")))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .unwrap();

    assert!(!bl.contains("kind:"), "un kind ausente no se escribe:\n{bl}");
    assert!(!bl.contains("name:"), "un name ausente no se escribe:\n{bl}");
}

/// Y sobreviven a un `accept`: son inertes, así que nada los toca.
#[test]
fn the_declaration_fields_survive_an_accept() {
    let (_t, root) = isolated_git_workspace();
    run_in(&root, &[
        "chain", "new", "--tip", "docs/spec.md", "--tip", "src/Service.java:2:5",
        "--kind", "governs", "--name.0", "la-decision",
    ]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    let bl = std::fs::read_dir(root.join(".bilink")).unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".yaml")))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .unwrap();

    assert!(bl.contains("kind: governs"),     "accept perdió el kind:\n{bl}");
    assert!(bl.contains("name: la-decision"), "accept perdió el name:\n{bl}");
}

// ─── task `17`: el commit se re-deriva ──────────────────────────────────────

/// Con la cache borrada, `get --diff` sigue funcionando.
///
/// `commit` es un derivado y la cache no está en git: un clon fresco no la tiene.
/// Si de ahí saliera un error, `accepted.hash` sería un hash que no se puede
/// resolver a texto, y la decisión de sacar `commit` del formato le costaría a
/// todo el que clone.
#[test]
fn get_diff_works_with_a_cold_cache() {
    let (_t, root) = isolated_git_workspace();
    run_in(&root, &["chain", "new", "--tip", "docs/spec.md:1:1", "--tip", "src/Service.java:2:5"]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);
    let uuid = sole_uuid(&root);

    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtro contenido.\n").unwrap();
    commit(&root, "cambio");

    fs::remove_file(root.join(".bilink/cache/state")).unwrap();

    let (out, err, ok) = run_in(&root, &["get", &format!("{uuid}.0"), "--diff"]);
    assert!(ok, "get --diff falló con la cache fría:\n{err}");
    assert!(out.contains("Some spec content"), "no recuperó el texto aceptado:\n{out}");
}

/// Y `check` sigue distinguiendo EXPANDED de ALTERED.
///
/// Sin el texto aceptado, EXPANDED, DISPLACED y REANCHORED degradan los tres a
/// ALTERED — "algo cambió y no sé qué". Es la distinción que se pierde, no un
/// detalle de rendimiento.
#[test]
fn check_still_tells_expanded_from_altered_with_a_cold_cache() {
    let (_t, root) = isolated_git_workspace();
    run_in(&root, &["chain", "new", "--tip", "docs/spec.md:1:1", "--tip", "src/Service.java:2:5"]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    // El fragmento crece alrededor de lo aceptado.
    let spec = fs::read_to_string(root.join("docs/spec.md")).unwrap();
    fs::write(root.join("docs/spec.md"), format!("{spec}\nUna línea más.\n")).unwrap();
    commit(&root, "creció");

    fs::remove_file(root.join(".bilink/cache/state")).unwrap();

    let states = check_states(&root);
    assert!(states.contains("EXPANDED"),
            "con la cache fría degradó a ALTERED en vez de derivar el commit:\n{states}");
}

fn sole_uuid(root: &Path) -> String {
    std::fs::read_dir(root.join(".bilink")).unwrap()
        .filter_map(|e| e.ok())
        .find_map(|e| e.file_name().to_str()?.strip_suffix(".yaml").map(str::to_owned))
        .expect("un bilink")
}

/// El diff aproximado también funciona desde una capa que no es la raíz del repo.
///
/// Cuando el texto aceptado no se puede verificar por hash en el commit cacheado,
/// `get --diff` cae a mostrar el fragmento por su range: para un diff informativo
/// es mejor algo aproximado que nada. Ese camino tiene su propio `git show`, y
/// `git show <commit>:<path>` resuelve el path contra la raíz del **repo**, no
/// contra el `-C`. Una capa anidada —`subsystems/lattice` dentro de accreta—
/// guarda paths relativos a sí misma, y sin traducirlos el comando falla.
#[test]
fn the_approximate_diff_works_from_a_nested_layer() {
    let (_t, root) = isolated_git_workspace();
    let nested = root.join("subsystems/thing");
    fs::create_dir_all(nested.join("docs")).unwrap();
    fs::create_dir_all(nested.join(".stratum/impl/src")).unwrap();
    fs::write(nested.join("docs/spec.md"), "# Spec\n\nContenido original.\n").unwrap();
    fs::write(nested.join(".stratum/impl/src/lib.rs"), "pub fn run() {}\n").unwrap();
    // Lo que la hace una capa: su propio `.bilink/` sin su propio `.git/`.
    fs::create_dir_all(nested.join(".bilink")).unwrap();
    commit(&root, "capa anidada");

    run_in(&nested, &["chain", "new", "--tip", "docs/spec.md:1:1", "--tip", ">impl/src/lib.rs:1:1"]);
    run_in(&nested, &["check", "."]);
    run_in(&nested, &["accept", "."]);
    let uuid = sole_uuid(&nested);

    // El hash aceptado deja de verificar en ese commit: es lo que empuja al
    // camino aproximado, y es lo que pasa cuando el fragmento derivó de verdad.
    let bl_path = nested.join(format!(".bilink/{uuid}.yaml"));
    let bl = fs::read_to_string(&bl_path).unwrap();
    let cut = bl.find("      hash: ").unwrap() + "      hash: ".len();
    let end = bl[cut..].find('\n').unwrap() + cut;
    fs::write(&bl_path, format!("{}{}{}", &bl[..cut], "0".repeat(64), &bl[end..])).unwrap();

    fs::write(nested.join("docs/spec.md"), "# Spec\n\nContenido nuevo.\n").unwrap();

    let (out, err, ok) = run_in(&nested, &["get", &format!("{uuid}.0"), "--diff"]);
    assert!(ok, "el diff aproximado falló desde una capa anidada:\n{err}");
    assert!(out.contains("Contenido original"),
            "no recuperó de git el fragmento de antes:\n{out}{err}");
}

// ─── task `18`: el rango no depende de lo que viene después ─────────────────

/// Agregar un item de secuencia más abajo no toca al que ya estaba.
///
/// `block_sequence_item` empieza en el `-` cuando es el último y en la
/// indentación de su línea cuando lo sigue otro, así que sin recortar los bordes
/// un item que nadie editó cambiaba de bytes —y de hash— por lo que pasó abajo.
#[test]
fn appending_a_yaml_item_does_not_move_the_one_above() {
    let (_t, root) = isolated_git_workspace();
    fs::write(root.join("docs/spec.yaml"),
        "scenarios:\n\n  - id: uno\n    description: primero\n\n  - id: dos\n    description: segundo\n").unwrap();
    commit(&root, "dos items");

    // El **último** item: es el que cambia de forma cuando aparece otro abajo.
    run_in(&root, &["chain", "new", "--tip", "docs/spec.yaml:6:3", "--tip", "src/Service.java:2:5"]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    let spec = fs::read_to_string(root.join("docs/spec.yaml")).unwrap();
    fs::write(root.join("docs/spec.yaml"),
              format!("{spec}\n  - id: tres\n    description: tercero\n")).unwrap();
    commit(&root, "un item más");

    let states = check_states(&root);
    assert!(!states.contains("RESTYLED") && !states.contains("ALTERED"),
            "un item que nadie tocó cambió de identidad:\n{states}");
}
