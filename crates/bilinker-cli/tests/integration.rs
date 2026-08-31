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

/// `apply` no decide con el estado cacheado: lo re-deriva.
///
/// El estado que escribió el último `check` describe el árbol de ese momento.
/// Acá el archivo se reescribe después, así que la cache queda diciendo que hay
/// algo que arreglar cuando ya no lo hay — y `apply` no tiene que creerle.
#[test]
fn apply_rederives_the_state_instead_of_trusting_the_cache() {
    let (_t, root, _u) = accepted_layer();

    // check deja un estado no-OK en la cache…
    let original = fs::read_to_string(root.join("docs/spec.md")).unwrap();
    fs::write(root.join("docs/spec.md"), original.clone() + "\nUna línea más.\n").unwrap();
    assert!(!check_states(&root).contains("OK\n"), "el escenario no se armó");

    // …y después el archivo vuelve atrás, sin volver a correr check.
    fs::write(root.join("docs/spec.md"), &original).unwrap();

    let (out, err, _) = run_in(&root, &["apply", "--dry-run"]);
    assert!(!out.contains("EXPANDED") && !out.contains("MOVED"),
            "apply propuso un fix contra un estado que ya no es:\n{out}{err}");
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

// ─── La ref de bilinks ─────────────────────────────────────────────────────
//
// ADR-0004. Los escenarios están en `subsystems/bilinker/scenarios/init.yaml` y
// `sync.yaml`.

/// `git` que devuelve stdout, para los tests que interrogan la ref.
fn git_out(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(root).args(args).output().unwrap();
    assert!(out.status.success(),
            "git {} falló: {}", args.join(" "),
            String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn rev(root: &Path, r: &str) -> String {
    git_out(root, &["rev-parse", r]).trim().to_string()
}

fn branch_of(root: &Path) -> String {
    git_out(root, &["symbolic-ref", "--short", "HEAD"]).trim().to_string()
}

/// Los padres de un commit, en orden. **La forma de un commit de la ref se lee de
/// acá y de [`bilink_diff`]**: cuántos padres, de dónde vienen, y cuál de los dos
/// árboles se movió. Nada más hace falta para distinguir los tres tipos.
fn parents_of(root: &Path, commit: &str) -> Vec<String> {
    git_out(root, &["rev-list", "--parents", "-n", "1", commit])
        .split_whitespace().skip(1).map(str::to_string).collect()
}

/// Qué cambió de `.bilink/` entre dos commits. Vacío en una absorción.
fn bilink_diff(root: &Path, from: &str, to: &str) -> Vec<String> {
    git_out(root, &["diff-tree", "-r", "--name-only", from, to])
        .lines().filter(|l| l.contains(".bilink/")).map(str::to_string).collect()
}

/// Los commits **propios** de la ref, del más nuevo al corte. El freno es la
/// disyunción, igual que en `Repo::ref_chain`: los del proyecto no llevan `.bilink/`.
fn ref_commits(root: &Path, bref: &str) -> Vec<String> {
    git_out(root, &["rev-list", "--first-parent", bref])
        .lines()
        .take_while(|c| git_out(root, &["ls-tree", "-r", "--name-only", c])
                        .contains(".bilink/"))
        .map(str::to_string)
        .collect()
}

/// El primer commit de la ref, bajando por primeros padres, que cumple algo.
fn first_parent_matching(
    root: &Path, bref: &str, pred: impl Fn(&str) -> bool,
) -> Option<String> {
    ref_commits(root, bref).into_iter().find(|c| pred(c))
}

/// El corte `005`, tal cual lo describe el ADR:
///
/// ```text
/// 1. UN commit que saca .bilink/ del índice de la rama   → X
/// 2. git update-ref refs/bilink/<branch> X
/// 3. bilinker init  (exclude + refspec)
/// 4. Commit sobre refs/bilink/<branch>: agrega .bilink/  → ●0, padre X
/// ```
///
/// Devuelve `(tmp, root, uuid, X)`. El paso 4 lo hace `sync`, que es la puerta
/// por la que todo commit sobre la ref pasa.
fn cut_over() -> (tempfile::TempDir, PathBuf, String, String) {
    let (tmp, root, uuid) = accepted_layer();
    let branch = branch_of(&root);

    // Antes del corte los bilinks viven en la rama, como hoy.
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "bilinks en la rama"]);

    // Paso 1: UN commit que los saca del índice. Los archivos se quedan en disco,
    // así que no hay que restaurarlos para commitearlos a la ref.
    git(&root, &["rm", "--cached", "-r", "-q", ".bilink"]);
    git(&root, &["commit", "-qm", "corte: los bilinks salen de la rama"]);
    let x = rev(&root, "HEAD");

    // Pasos 2 y 3: el exclude y el refspec. `init` no materializa nada porque el
    // .bilink/ del árbol todavía no tiene procedencia.
    let (_, stderr, ok) = run_in(&root, &["init"]);
    assert!(ok, "init falló:\n{stderr}");

    // Paso 4: el commit que crea la ref desde cero, con X como padre único.
    let (_, stderr, ok) = run_in(&root, &["track", &branch]);
    assert!(ok, "el commit del corte falló:\n{stderr}");

    (tmp, root, uuid, x)
}

/// `init-does-not-touch-gitignore` + el refspec en `.git/config`.
#[test]
fn init_writes_exclude_and_refspec_without_touching_the_branch() {
    let (_t, root) = isolated_git_workspace();
    git(&root, &["remote", "add", "origin", "https://example.invalid/r.git"]);

    let (_, stderr, ok) = run_in(&root, &["init"]);
    assert!(ok, "init falló:\n{stderr}");

    let exclude = fs::read_to_string(root.join(".git/info/exclude")).unwrap();
    assert!(exclude.contains(".bilink/"), "falta .bilink/ en el exclude:\n{exclude}");
    assert!(exclude.contains(".bilink-migrate-*"),
            "las carpetas de migración también van al exclude:\n{exclude}");

    let config = fs::read_to_string(root.join(".git/config")).unwrap();
    assert!(config.contains("refs/bilink/*:refs/bilink/*"),
            "falta el refspec:\n{config}");
    assert!(!config.contains("+refs/bilink/*:refs/bilink/*"),
            "y va **sin `+`**: con él, un fetch de una ref divergida pisa la local \
             en silencio y se lleva puesto un padre:\n{config}");

    assert!(!root.join(".gitignore").exists(),
            ".gitignore está versionado: tocarlo modificaría la rama del proyecto");
    assert!(git_out(&root, &["status", "--porcelain"]).trim().is_empty(),
            "init no puede dejar la rama del proyecto sucia");
}

/// Un clon que corrió un `init` viejo lleva el refspec con `+`. **Sacarlo es parte
/// de `init`**: agregar el nuevo al lado dejaría los dos, y git aplica los dos.
#[test]
fn init_removes_the_forced_refspec_a_previous_version_wrote() {
    let (_t, root) = isolated_git_workspace();
    git(&root, &["remote", "add", "origin", "https://example.invalid/r.git"]);
    git(&root, &["config", "--add", "remote.origin.fetch", "+refs/bilink/*:refs/bilink/*"]);

    let (_, stderr, ok) = run_in(&root, &["init"]);
    assert!(ok, "init falló:\n{stderr}");

    let fetch = git_out(&root, &["config", "--get-all", "remote.origin.fetch"]);
    assert!(!fetch.contains("+refs/bilink"), "el forzado tiene que salir:\n{fetch}");
    assert!(fetch.contains("refs/bilink/*:refs/bilink/*"), "y quedar el otro:\n{fetch}");
}

/// `init-is-idempotent` — correrlo dos veces no agrega nada.
#[test]
fn init_is_idempotent() {
    let (_t, root) = isolated_git_workspace();
    git(&root, &["remote", "add", "origin", "https://example.invalid/r.git"]);

    run_in(&root, &["init"]);
    let first = fs::read_to_string(root.join(".git/config")).unwrap();
    run_in(&root, &["init"]);
    let second = fs::read_to_string(root.join(".git/config")).unwrap();

    assert_eq!(first, second, "el segundo init duplicó el refspec");
}

/// `init-does-not-overwrite-bilinks-without-head` — el paso 3 no pisa nada.
///
/// Es lo que hace que el corte pueda ser un `init` a secas: ahí el `.bilink/`
/// todavía no está en la ref, y materializar lo borraría.
#[test]
fn init_does_not_materialize_over_bilinks_without_provenance() {
    let (_t, root, _uuid) = accepted_layer();
    let branch = branch_of(&root);
    git(&root, &["rm", "--cached", "-r", "-q", ".bilink"]);
    git(&root, &["commit", "-qm", "corte"]);
    git(&root, &["update-ref", &format!("refs/bilink/{branch}"), "HEAD"]);

    let before: Vec<_> = fs::read_dir(root.join(".bilink")).unwrap()
        .filter_map(|e| e.ok()).map(|e| e.file_name()).collect();

    let (stdout, stderr, ok) = run_in(&root, &["init"]);
    assert!(ok, "init falló:\n{stderr}");
    assert!(stdout.contains("sin head"), "init tiene que decir que no materializó:\n{stdout}");
    assert!(!root.join(".bilink/head").exists(),
            "sin procedencia no se escribe head tampoco");

    let after: Vec<_> = fs::read_dir(root.join(".bilink")).unwrap()
        .filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
    assert_eq!(before.len(), after.len(), "init borró bilinks que no estaban en la ref");
}

/// `init-reports-branch-without-ref` — una rama sin ref no es un error.
#[test]
fn init_points_at_track_when_the_branch_has_no_ref() {
    let (_t, root) = isolated_git_workspace();
    let (stdout, _, ok) = run_in(&root, &["init"]);
    assert!(ok, "una rama sin ref no es un error de init");
    assert!(stdout.contains("bilinker track"),
            "de quién hereda los bilinks lo decide track, y init lo dice:\n{stdout}");
}

/// El corte: `●0` nace de `X` como padre único, y su árbol de código es el de `X`.
///
/// `cutover-first-absorb-is-clean` depende de esto: la ref nace de un commit del
/// proyecto donde `.bilink/` ya no está en el árbol, y eso hace disjuntos los dos
/// lados de ahí en adelante.
#[test]
fn the_cut_commit_has_the_project_commit_as_its_only_parent() {
    let (_t, root, _uuid, x) = cut_over();
    let branch = branch_of(&root);
    let bref = format!("refs/bilink/{branch}");

    let parents = git_out(&root, &["rev-list", "--parents", "-n", "1", &bref]);
    let parents: Vec<&str> = parents.split_whitespace().skip(1).collect();
    assert_eq!(parents, vec![x.as_str()], "●0 tiene a X como padre único");

    assert_eq!(rev(&root, &format!("{bref}^{{tree}}")).is_empty(), false);
    let files = git_out(&root, &["ls-tree", "-r", "--name-only", &bref]);
    assert!(files.contains(".bilink/"), "●0 tiene que llevar .bilink/:\n{files}");
    assert!(files.contains("src/Service.java"), "y el árbol del proyecto:\n{files}");
}

/// `ref-carries-only-bilink-from-worktree` — cache, index y head quedan afuera.
#[test]
fn the_ref_does_not_carry_the_derived_files() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    let files = git_out(&root, &["ls-tree", "-r", "--name-only", &bref]);
    for derived in [".bilink/cache", ".bilink/index", ".bilink/head"] {
        assert!(!files.contains(derived),
                "{derived} es derivado y no se commitea:\n{files}");
    }
}

/// `project-branch-has-no-bilinks` + `bilink-ref-not-listed-as-branch`.
#[test]
fn after_the_cut_the_branch_has_no_bilinks_and_the_ref_is_not_a_branch() {
    let (_t, root, _uuid, _x) = cut_over();

    let tracked = git_out(&root, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(!tracked.contains(".bilink/"),
            "ninguna rama del proyecto contiene .bilink/:\n{tracked}");

    let branches = git_out(&root, &["branch", "-a"]);
    assert!(!branches.contains("bilink"),
            "la ref vive fuera de refs/heads/ y no se lista:\n{branches}");

    // Y sigue estando en el árbol de trabajo, donde check la necesita.
    assert!(root.join(".bilink").is_dir(), "los .bilink/ están en el árbol de trabajo");
}

/// `bilinks-present-but-excluded` — el proyecto no los ve.
#[test]
fn the_project_status_does_not_show_the_bilinks() {
    let (_t, root, _uuid, _x) = cut_over();
    let status = git_out(&root, &["status", "--porcelain"]);
    assert!(!status.contains(".bilink"),
            "el índice del proyecto los ignora vía info/exclude:\n{status}");
}

/// `merge-absorbs-project-without-conflict` — el segundo padre es el tip.
#[test]
fn sync_absorbs_the_project_tip_as_the_second_parent() {
    let (_t, root, _uuid, _x) = cut_over();
    let branch = branch_of(&root);
    let bref = format!("refs/bilink/{branch}");

    fs::write(root.join("src/Other.java"), "public class Other {}\n").unwrap();
    commit(&root, "otro archivo");
    let tip = rev(&root, "HEAD");

    let (_, stderr, ok) = run_in(&root, &["sync"]);
    assert!(ok, "sync falló:\n{stderr}");

    let parents = git_out(&root, &["rev-list", "--parents", "-n", "1", &bref]);
    let parents: Vec<&str> = parents.split_whitespace().skip(1).collect();
    assert_eq!(parents.len(), 2, "absorber es un merge:\n{parents:?}");
    assert_eq!(parents[1], tip, "el segundo padre es lo que se absorbe");
}

/// `sync-diff-against-first-parent-is-empty` — sync no registra ninguna decisión.
#[test]
fn the_sync_commit_has_an_empty_diff_against_its_first_parent() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    fs::write(root.join("src/Other.java"), "public class Other {}\n").unwrap();
    commit(&root, "otro archivo");
    run_in(&root, &["sync"]);

    let diff = git_out(&root, &["diff", "--name-only",
                                &format!("{bref}~1"), &bref, "--", "."]);
    let bilink_changes: Vec<&str> = diff.lines().filter(|l| l.contains(".bilink/")).collect();
    assert!(bilink_changes.is_empty(),
            "sync alinea la foto y nada más:\n{bilink_changes:?}");
}

/// `ref-snapshot-is-faithful` — el árbol de código es el del commit absorbido.
#[test]
fn every_ref_commit_carries_the_code_of_the_commit_it_absorbed() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    fs::write(root.join("src/Other.java"), "public class Other {}\n").unwrap();
    commit(&root, "otro archivo");
    run_in(&root, &["sync"]);
    let tip = rev(&root, "HEAD");

    // Todo lo que difiere entre el commit absorbido y el de la ref cae en .bilink/.
    let diff = git_out(&root, &["diff-tree", "-r", "--name-only", &tip, &bref]);
    let strays: Vec<&str> = diff.lines()
        .filter(|l| !l.trim().is_empty() && !l.contains(".bilink/"))
        .collect();
    assert!(strays.is_empty(), "el árbol de código tiene que ser idéntico:\n{strays:?}");
}

/// `sync-is-noop-when-already-absorbed`.
#[test]
fn sync_writes_nothing_when_the_tip_is_already_absorbed() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));
    let before = rev(&root, &bref);

    let (stdout, stderr, ok) = run_in(&root, &["sync"]);
    assert!(ok, "sync falló:\n{stderr}");
    assert!(stdout.contains("nada que hacer"), "y lo dice:\n{stdout}");
    assert_eq!(before, rev(&root, &bref), "no se escribió ningún commit");
}

/// `merge-back-is-detected` — la disyunción va sobre el ÁRBOL, no sobre el diff.
#[test]
fn absorbing_a_commit_that_carries_bilinks_is_aborted() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));
    let before = rev(&root, &bref);

    // Alguien commitea bilinks a mano en la rama del proyecto.
    git(&root, &["add", "-f", ".bilink"]);
    git(&root, &["commit", "-qm", "bilinks a mano en la rama"]);

    let (_, stderr, ok) = run_in(&root, &["sync"]);
    assert!(!ok, "absorber un commit con .bilink/ en el árbol tiene que abortar");
    assert!(stderr.contains(".bilink/"), "y decir por qué:\n{stderr}");
    assert_eq!(before, rev(&root, &bref), "no se escribió nada");
}

/// El commit que **borra** `.bilink/` tiene un diff que lo toca y un árbol que no,
/// y es exactamente el que hay que poder absorber. Por eso la verificación es
/// sobre el árbol.
#[test]
fn the_commit_that_removes_bilinks_is_absorbable() {
    let (_t, root, _uuid, x) = cut_over();
    // `cut_over` ya absorbió `X`, que es ese commit: si la verificación mirara el
    // diff, el corte mismo sería imposible.
    let bref = format!("refs/bilink/{}", branch_of(&root));
    let parents = git_out(&root, &["rev-list", "--parents", "-n", "1", &bref]);
    assert!(parents.contains(&x), "X es el commit que borra .bilink/ y se absorbió");
}

/// `sync-dry-run-is-inert`.
#[test]
fn sync_dry_run_writes_nothing() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));
    fs::write(root.join("src/Other.java"), "public class Other {}\n").unwrap();
    commit(&root, "otro archivo");
    let before = rev(&root, &bref);

    let (stdout, stderr, ok) = run_in(&root, &["sync", "--dry-run"]);
    assert!(ok, "dry-run falló:\n{stderr}");
    assert!(stdout.contains("no se escribió nada"), "y lo dice:\n{stdout}");
    assert_eq!(before, rev(&root, &bref), "dry-run no escribe");
}

/// `sync-refuses-on-detached-head` — sin rama no se commitea sobre la ref.
#[test]
fn sync_refuses_on_a_detached_head() {
    let (_t, root, _uuid, _x) = cut_over();
    git(&root, &["checkout", "-q", "--detach", "HEAD"]);

    let (_, stderr, ok) = run_in(&root, &["sync"]);
    assert!(!ok, "adivinar una rama sería peor que no hacer nada");
    assert!(stderr.contains("desacoplado"), "y se dice:\n{stderr}");
}

/// `init-writes-head-and-version` — la materialización deja procedencia.
#[test]
fn committing_on_the_ref_leaves_head_pointing_at_the_new_commit() {
    let (_t, root, _uuid, _x) = cut_over();
    let branch = branch_of(&root);
    let bref = format!("refs/bilink/{branch}");

    let head = fs::read_to_string(root.join(".bilink/head")).unwrap();
    assert!(head.contains(&format!("branch {branch}")), "head nombra la rama:\n{head}");
    assert!(head.contains(&rev(&root, &bref)),
            "y el commit de la ref al que corresponde el árbol:\n{head}");
}

/// El commit de la ref avanza y `head` lo sigue, o la guarda se dispararía después
/// de cada aceptación.
#[test]
fn head_follows_the_ref_after_every_commit() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    fs::write(root.join("src/Other.java"), "public class Other {}\n").unwrap();
    commit(&root, "otro archivo");
    run_in(&root, &["sync"]);

    let head = fs::read_to_string(root.join(".bilink/head")).unwrap();
    assert!(head.contains(&rev(&root, &bref)),
            "head quedó atrás del commit que acaba de escribirse:\n{head}");
}

/// Una rama con su propia ref y otros bilinks, para ejercer la materialización.
///
/// Sale de antes del corte, así que ningún commit de `refs/bilink/main` califica
/// como herencia y su ref nace desde cero — con el `.bilink/` del árbol, al que le
/// sacamos un bilink para que las dos ramas difieran.
fn second_branch_without(root: &Path, uuid: &str) -> String {
    let base = rev(root, "HEAD~2");
    git(root, &["checkout", "-q", "-b", "otra", &base]);
    fs::remove_file(root.join(format!(".bilink/{uuid}.yaml"))).unwrap();

    let (_, stderr, ok) = run_in(root, &["track", "otra"]);
    assert!(ok, "track otra falló:\n{stderr}");
    "otra".to_string()
}

/// `branch-switch-rematerializes` — cambiar de rama corrige el árbol sin pedir nada.
#[test]
fn switching_branches_rematerializes_the_bilinks_of_the_new_branch() {
    let (_t, root, uuid, _x) = cut_over();
    let main = branch_of(&root);
    let bilink = root.join(format!(".bilink/{uuid}.yaml"));

    second_branch_without(&root, &uuid);
    assert!(!bilink.exists(), "la ref de otra no lleva ese bilink");

    git(&root, &["checkout", "-q", &main]);
    assert!(!bilink.exists(),
            "git checkout no toca .bilink/: son archivos ignorados para el proyecto");

    // Un comando cualquiera, no `init`: la corrección es automática y sin ceremonia.
    let (_, stderr, ok) = run_in(&root, &["check", "."]);
    assert!(ok || !stderr.contains("error"), "la materialización falló:\n{stderr}");
    assert!(bilink.exists(), "el .bilink/ de la rama actual se materializa solo");

    let head = fs::read_to_string(root.join(".bilink/head")).unwrap();
    assert!(head.contains(&format!("branch {main}")), "y head lo dice:\n{head}");
}

/// `cache-invalidates-on-branch-change` es la otra mitad del par: `head` protege a
/// la fuente y el commit anotado en la cache protege al derivado.
///
/// Sin esto la cache devuelve estados de la rama anterior **en silencio**, y lo
/// peor no es el reporte: `accept` le cree y no acepta nada, así que la rama se
/// queda con un `accepted` viejo sin que nadie se entere.
#[test]
fn the_cache_does_not_return_states_from_the_previous_branch() {
    let (_t, root, uuid, _x) = cut_over();
    let main = branch_of(&root);

    // Una segunda rama trackeada, con los mismos bilinks.
    git(&root, &["checkout", "-q", "-b", "otra"]);
    run_in(&root, &["track", "otra"]);

    // En main el fragmento cambia y alguien lo acepta.
    git(&root, &["checkout", "-q", &main]);
    run_in(&root, &["init"]);
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);
    run_in(&root, &["sync"]);
    let accepted_in_main = fs::read_to_string(root.join(format!(".bilink/{uuid}.yaml"))).unwrap();

    // En la otra rama el mismo cambio, sobre bilinks que todavía dicen lo viejo.
    git(&root, &["checkout", "-q", "otra"]);
    run_in(&root, &["init"]);
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el mismo cambio, del otro lado");

    let states = check_states(&root);
    assert!(states.contains("ALTERED"),
            "la cache de main no puede contestar por otra: el fragmento acá sí cambió:\n{states}");

    run_in(&root, &["accept", "."]);
    assert_eq!(fs::read_to_string(root.join(format!(".bilink/{uuid}.yaml"))).unwrap(),
               accepted_in_main,
               "aceptar el mismo contenido en dos HEADs escribe los mismos valores");
}

/// El commit anotado en la cache sale de `head`, que es un hecho sobre el árbol.
#[test]
fn the_cache_records_the_ref_commit_it_was_computed_against() {
    let (_t, root, _uuid, _x) = cut_over();
    run_in(&root, &["check", "."]);

    let cache = fs::read_to_string(root.join(".bilink/cache/state")).unwrap();
    let head = fs::read_to_string(root.join(".bilink/head")).unwrap();
    let commit = head.lines().find_map(|l| l.strip_prefix("commit ")).unwrap().trim();

    assert!(cache.contains(commit),
            "la cache tiene que anotar de qué commit de la ref salió:\n{cache}");
}

/// `branch-switch-refuses-on-dirty-bilinks` — la guarda es la de git.
#[test]
fn materialization_refuses_when_the_bilinks_carry_uncommitted_work() {
    let (_t, root, uuid, _x) = cut_over();
    let main = branch_of(&root);

    second_branch_without(&root, &uuid);
    git(&root, &["checkout", "-q", &main]);

    // Trabajo en .bilink/ que no está en ninguna parte: .bilink/ está fuera del
    // git del proyecto, así que materializar lo destruiría.
    let cap = fs::read_dir(root.join(".bilink/capture")).unwrap()
        .filter_map(|e| e.ok()).next().unwrap().path();
    let dirty = format!("{}# editado a mano\n", fs::read_to_string(&cap).unwrap());
    fs::write(&cap, &dirty).unwrap();

    let (_, stderr, ok) = run_in(&root, &["check", "."]);
    assert!(!ok, "con trabajo sin commitear no se materializa nada:\n{stderr}");
    assert!(stderr.contains("difiere"), "y se dice por qué:\n{stderr}");
    assert_eq!(fs::read_to_string(&cap).unwrap(), dirty, "el trabajo sigue ahí");
}

/// `track-picks-newest-reachable` — la ref adelantada no arrastra bilinks de más.
#[test]
fn track_inherits_from_the_commit_whose_absorbed_is_still_an_ancestor() {
    let (_t, root, _uuid, _x) = cut_over();
    let main = branch_of(&root);

    // La rama sale de D; la ref de main sigue hasta E.
    let d = rev(&root, "HEAD");
    fs::write(root.join("src/Later.java"), "public class Later {}\n").unwrap();
    commit(&root, "E, que feature/x no tiene");
    run_in(&root, &["sync"]);

    git(&root, &["checkout", "-q", "-b", "feature/x", &d]);
    let (_, stderr, ok) = run_in(&root, &["track", "feature/x"]);
    assert!(ok, "track falló:\n{stderr}");

    // El primer padre es un commit de refs/bilink/main cuyo absorbido es D, no E.
    let parents = git_out(&root, &["rev-list", "--parents", "-n", "1", "refs/bilink/feature/x"]);
    let parents: Vec<&str> = parents.split_whitespace().skip(1).collect();
    assert_eq!(parents.len(), 2, "track escribe un commit de dos padres:\n{parents:?}");
    assert_eq!(parents[1], rev(&root, "HEAD"), "el segundo padre es el tip de la rama");

    let inherited_absorbs = git_out(&root, &["rev-list", "--parents", "-n", "1", parents[0]]);
    assert!(inherited_absorbs.contains(&d) || parents[0] == d,
            "heredar del tip traería bilinks que describen código que la rama no tiene");
    assert!(!inherited_absorbs.contains(&rev(&root, &format!("refs/heads/{main}"))),
            "y no del commit que absorbió E");
}

/// `track` no decide nada: su diff contra el primer padre es vacío.
#[test]
fn the_track_commit_records_no_decision() {
    let (_t, root, _uuid, _x) = cut_over();
    let d = rev(&root, "HEAD");
    git(&root, &["checkout", "-q", "-b", "feature/x", &d]);
    run_in(&root, &["track", "feature/x"]);

    let diff = git_out(&root, &["diff", "--name-only",
                                "refs/bilink/feature/x^1", "refs/bilink/feature/x"]);
    let bilink_changes: Vec<&str> = diff.lines().filter(|l| l.contains(".bilink/")).collect();
    assert!(bilink_changes.is_empty(), "track hereda, no decide:\n{bilink_changes:?}");
}

/// `track` sobre una rama que ya tiene ref es un error, no un no-op silencioso.
#[test]
fn track_refuses_when_the_branch_already_has_a_ref() {
    let (_t, root, _uuid, _x) = cut_over();
    let branch = branch_of(&root);
    let (_, stderr, ok) = run_in(&root, &["track", &branch]);
    assert!(!ok, "la ref ya existe");
    assert!(stderr.contains("sync"), "y el arreglo es sync:\n{stderr}");
}

// ─── adopt ─────────────────────────────────────────────────────────────────

/// Dos ramas trackeadas que salen del mismo punto, para ejercer el merge a tres
/// puntas de `adopt`.
///
/// La base sale gratis: es la base de merge real, porque `track` puso el commit
/// heredado como **primer padre** en vez de copiar archivos.
fn two_tracked_branches() -> (tempfile::TempDir, PathBuf, String, String) {
    let (tmp, root, uuid, _x) = cut_over();
    let main = branch_of(&root);

    git(&root, &["checkout", "-q", "-b", "feature/x"]);
    let (_, stderr, ok) = run_in(&root, &["track", "feature/x"]);
    assert!(ok, "track feature/x falló:\n{stderr}");

    (tmp, root, main, uuid)
}

/// Acepta de nuevo el endpoint estructural tras cambiar el fragmento, y commitea
/// sobre la ref. Es lo que produce una decisión del otro lado.
fn decide_on(root: &Path, branch: &str, content: &str) {
    git(root, &["checkout", "-q", branch]);
    run_in(root, &["init"]);
    fs::write(root.join("src/Service.java"), content).unwrap();
    commit(root, "el fragmento cambia");
    run_in(root, &["check", "."]);
    let (_, stderr, ok) = run_in(root, &["accept", "."]);
    assert!(ok, "accept falló en {branch}:\n{stderr}");
    let (_, stderr, ok) = run_in(root, &["sync"]);
    assert!(ok, "sync falló en {branch}:\n{stderr}");
}

/// `adopt-brings-neighbour-decisions` — las aceptaciones del vecino entran.
#[test]
fn adopt_brings_the_decisions_of_the_neighbour_branch() {
    let (_t, root, main, uuid) = two_tracked_branches();

    decide_on(&root, &main, "public class Service {\n    public void run() { int x = 1; }\n}\n");

    git(&root, &["checkout", "-q", "feature/x"]);
    run_in(&root, &["init"]);
    // La rama rebasea sobre main: el código del vecino entra acá.
    git(&root, &["rebase", "-q", &main]);
    run_in(&root, &["init"]);

    let (stdout, stderr, ok) = run_in(&root, &["adopt", &main]);
    assert!(ok, "adopt falló:\n{stderr}\n{stdout}");
    assert!(stdout.contains("entra limpio"), "las decisiones de main entran:\n{stdout}");

    let bl = fs::read_to_string(root.join(format!(".bilink/{uuid}.yaml"))).unwrap();
    let main_bl = git_out(&root, &["show",
        &format!("refs/bilink/{main}:.bilink/{uuid}.yaml")]);
    assert_eq!(bl.trim(), main_bl.trim(),
               "el accepted del vecino llegó entero");
}

/// `adopt-dry-run-is-inert-and-exact`.
#[test]
fn adopt_dry_run_writes_nothing() {
    let (_t, root, main, uuid) = two_tracked_branches();
    decide_on(&root, &main, "public class Service {\n    public void run() { int x = 1; }\n}\n");

    git(&root, &["checkout", "-q", "feature/x"]);
    run_in(&root, &["init"]);
    git(&root, &["rebase", "-q", &main]);
    run_in(&root, &["init"]);

    let before_ref = rev(&root, "refs/bilink/feature/x");
    let before_bl = fs::read_to_string(root.join(format!(".bilink/{uuid}.yaml"))).unwrap();

    let (stdout, stderr, ok) = run_in(&root, &["adopt", &main, "--dry-run"]);
    assert!(ok, "dry-run falló:\n{stderr}");
    assert!(stdout.contains("no se escribió nada"), "y lo dice:\n{stdout}");
    assert_eq!(before_ref, rev(&root, "refs/bilink/feature/x"), "la ref no se movió");
    assert_eq!(before_bl,
               fs::read_to_string(root.join(format!(".bilink/{uuid}.yaml"))).unwrap(),
               "ni el bilink del árbol");
}

/// `adopt-converges-without-conflict` — lo que las dos ramas aceptaron igual no
/// conflictúa, y es la fila "ya coincidía".
#[test]
fn what_both_branches_accepted_alike_does_not_conflict() {
    let (_t, root, main, _uuid) = two_tracked_branches();
    let same = "public class Service {\n    public void run() { int x = 1; }\n}\n";

    decide_on(&root, &main, same);
    // La misma decisión, del otro lado y sobre otro HEAD.
    decide_on(&root, "feature/x", same);

    let (stdout, stderr, ok) = run_in(&root, &["adopt", &main, "--dry-run"]);
    assert!(ok, "adopt falló:\n{stderr}\n{stdout}");
    assert!(!stdout.contains("conflicto"),
            "dos personas que aceptan el mismo contenido escriben los mismos valores:\n{stdout}");
}

/// Los conflictos paran el comando, y no se escribe nada — ni siquiera la absorción.
#[test]
fn a_conflict_stops_adopt_without_writing_anything() {
    let (_t, root, main, _uuid) = two_tracked_branches();

    decide_on(&root, &main, "public class Service {\n    public void run() { int x = 1; }\n}\n");
    decide_on(&root, "feature/x", "public class Service {\n    public void run() { int y = 2; }\n}\n");

    let before = rev(&root, "refs/bilink/feature/x");
    let (stdout, _, ok) = run_in(&root, &["adopt", &main]);
    assert!(!ok, "con conflicto el comando falla:\n{stdout}");
    assert!(stdout.contains("conflicto"), "y los enumera:\n{stdout}");
    assert!(stdout.contains("no se escribió nada"), "y lo dice:\n{stdout}");
    assert_eq!(before, rev(&root, "refs/bilink/feature/x"), "todo o nada");
}

/// `adopt` es asimétrico: lo que sólo yo decidí se queda como está.
#[test]
fn adopt_never_overwrites_a_decision_only_this_branch_made() {
    let (_t, root, main, uuid) = two_tracked_branches();

    // Sólo feature/x decide. main no tocó nada.
    decide_on(&root, "feature/x", "public class Service {\n    public void run() { int y = 2; }\n}\n");
    let mine = fs::read_to_string(root.join(format!(".bilink/{uuid}.yaml"))).unwrap();

    let (stdout, stderr, ok) = run_in(&root, &["adopt", &main]);
    assert!(ok, "adopt falló:\n{stderr}\n{stdout}");
    assert_eq!(mine, fs::read_to_string(root.join(format!(".bilink/{uuid}.yaml"))).unwrap(),
               "ninguna decisión propia se pisa");
}

/// Adoptar de la rama actual no tiene sentido y se dice.
#[test]
fn adopt_refuses_the_current_branch() {
    let (_t, root, _uuid, _x) = cut_over();
    let branch = branch_of(&root);
    let (_, stderr, ok) = run_in(&root, &["adopt", &branch]);
    assert!(!ok, "no hay nada que adoptar de uno mismo");
    assert!(stderr.contains("rama actual"), "y se dice:\n{stderr}");
}

// ─── accept y apply commitean sobre la ref ─────────────────────────────────

/// `accept-absorbs-before-committing` — absorber es precondición, y **ocurre en un
/// commit propio**: el tip queda siendo la decisión, de un solo padre, y la absorción
/// es lo que tiene arriba.
#[test]
fn accept_absorbs_in_a_commit_of_its_own_right_before_deciding() {
    let (_t, root, _uuid, _x) = cut_over();
    let branch = branch_of(&root);
    let bref = format!("refs/bilink/{branch}");

    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");
    let e = rev(&root, "HEAD");

    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", "."]);
    assert!(ok, "accept falló:\n{stderr}");

    // El tip es una decisión: un padre, y nada de código.
    let parents = parents_of(&root, &bref);
    assert_eq!(parents.len(), 1, "una decisión tiene un solo padre:\n{parents:?}");

    // Y la absorción está más abajo, con el commit del proyecto como segundo padre.
    let absorption = first_parent_matching(&root, &bref, |p| parents_of(&root, p).len() == 2)
        .expect("tiene que haber una absorción debajo de las decisiones");
    let ap = parents_of(&root, &absorption);
    assert_eq!(ap[1], e, "el segundo padre es el commit contra el que se aceptó");
    assert!(bilink_diff(&root, &ap[0], &absorption).is_empty(),
            "la absorción no toca .bilink/: trae código y nada más");

    // La invariante de fidelidad se verifica sola.
    let diff = git_out(&root, &["diff-tree", "-r", "--name-only", &e, &bref]);
    let strays: Vec<&str> = diff.lines()
        .filter(|l| !l.trim().is_empty() && !l.contains(".bilink/")).collect();
    assert!(strays.is_empty(), "el árbol de código es el del commit absorbido:\n{strays:?}");
}

/// Invariante 4: **ningún commit de la ref tiene dos padres y diff de `.bilink/` no
/// vacío.** Se lee sobre toda la ref, que es como un `pre-receive` la va a leer.
#[test]
fn no_ref_commit_both_absorbs_and_decides() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    // Un poco de todo: absorciones, decisiones sueltas y un bulk.
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtro contenido.\n").unwrap();
    commit(&root, "y la spec también");
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);
    run_in(&root, &["sync"]);

    for c in ref_commits(&root, &bref) {
        let parents = parents_of(&root, &c);
        if parents.len() < 2 { continue; }
        assert!(bilink_diff(&root, &parents[0], &c).is_empty(),
                "{c} absorbe y decide a la vez");
    }
}

/// `act-without-new-code-has-one-parent` — el merge nunca es la forma del acto: una
/// decisión tiene **siempre** un solo padre, se haya absorbido o no.
#[test]
fn an_accept_with_the_project_still_has_a_single_parent() {
    let (_t, root, _uuid, _x) = cut_over();
    let branch = branch_of(&root);
    let bref = format!("refs/bilink/{branch}");

    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);          // este absorbe

    let tree_before = rev(&root, &format!("{bref}^{{tree}}"));

    // Segundo acto, con el proyecto quieto: nada nuevo que traer.
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 2; }\n}\n").unwrap();
    commit(&root, "otra vez");
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);          // este también absorbe
    run_in(&root, &["accept", "."]);          // y este no tiene nada que hacer

    let parents = parents_of(&root, &bref);
    assert_eq!(parents.len(), 1, "el tip es la decisión, no la absorción:\n{parents:?}");
    assert_ne!(tree_before, rev(&root, &format!("{bref}^{{tree}}")), "y escribió algo");

    // La absorción está justo debajo, y trae código sin decidir nada.
    let below = parents_of(&root, &parents[0]);
    assert_eq!(below.len(), 2, "el padre de la decisión es la absorción:\n{below:?}");
    assert!(bilink_diff(&root, &below[0], &parents[0]).is_empty(),
            "y absorber no decide");
}

/// La granularidad sigue al **objeto**: `accept .` de dos endpoints da dos commits de
/// decisión, los dos hijos de la misma absorción.
#[test]
fn accept_writes_one_commit_per_acceptance_not_per_invocation() {
    let (_t, root, uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    // Los dos endpoints cambian a la vez.
    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtro contenido.\n").unwrap();
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "los dos lados cambian");

    let before = rev(&root, &bref);
    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", "."]);
    assert!(ok, "accept falló:\n{stderr}");

    // Lo nuevo **de la ref**, del más viejo al más nuevo: una absorción y dos
    // decisiones. `--first-parent` es lo que deja afuera el commit del proyecto, que
    // la absorción vuelve alcanzable por el segundo padre.
    let nuevos = git_out(&root, &["rev-list", "--reverse", "--first-parent",
                                  &format!("{before}..{bref}")]);
    let nuevos: Vec<&str> = nuevos.lines().collect();
    assert_eq!(nuevos.len(), 3, "una absorción y una decisión por endpoint:\n{nuevos:?}");

    assert_eq!(parents_of(&root, nuevos[0]).len(), 2, "primero la absorción");
    for d in &nuevos[1..] {
        assert_eq!(parents_of(&root, d).len(), 1, "y después las decisiones, de un padre");
    }

    // Las dos cuelgan de la misma absorción: la segunda es hija de la primera.
    assert_eq!(parents_of(&root, nuevos[2])[0], nuevos[1]);
    assert_eq!(parents_of(&root, nuevos[1])[0], nuevos[0]);

    // Y cada commit nombra **su** endpoint, no la invocación.
    let asuntos = git_out(&root, &["log", "-2", "--format=%s", &bref]);
    assert_eq!(asuntos.lines().count(), 2);
    for a in asuntos.lines() {
        assert!(a.starts_with(&format!("accept {uuid}.")),
                "cada decisión lleva su propio endpoint, no `accept .`:\n{a}");
    }
    assert!(asuntos.contains(&format!("accept {uuid}.0"))
            && asuntos.contains(&format!("accept {uuid}.1")),
            "y son los dos endpoints, uno por commit:\n{asuntos}");
}

/// El árbol de código de una decisión es el de la absorción que tiene arriba: no
/// cambia entre las N decisiones de un mismo `accept .`.
#[test]
fn the_decisions_of_one_invocation_share_the_code_tree_of_their_absorption() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtro contenido.\n").unwrap();
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "los dos lados cambian");
    let e = rev(&root, "HEAD");

    let before = rev(&root, &bref);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    for c in git_out(&root, &["rev-list", "--first-parent",
                              &format!("{before}..{bref}")]).lines() {
        let strays: Vec<String> = git_out(&root, &["diff-tree", "-r", "--name-only", &e, c])
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.contains(".bilink/"))
            .map(str::to_string)
            .collect();
        assert!(strays.is_empty(), "{c} movió código:\n{strays:?}");
    }
}

/// En un repo que todavía no cortó, `accept` no commitea nada: los bilinks viven en
/// la rama y commitearlos es de quien trabaja.
#[test]
fn accept_does_not_commit_in_a_repo_that_has_not_cut_over() {
    let (_t, root, _uuid) = accepted_layer();
    let before = rev(&root, "HEAD");

    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");
    let after_project_commit = rev(&root, "HEAD");

    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", "."]);
    assert!(ok, "accept falló:\n{stderr}");

    assert_ne!(before, after_project_commit);
    assert_eq!(after_project_commit, rev(&root, "HEAD"),
               "sin ref, accept no escribe ningún commit");
    assert!(git_out(&root, &["for-each-ref", "refs/bilink/"]).trim().is_empty(),
            "y no crea ninguna ref por su cuenta");
}

// ─── task `1g`: la ref es protegida ────────────────────────────────────────

/// `verify-ref` sobre un rango, con la salida y el código de salida.
fn verify(root: &Path, args: &[&str]) -> (String, bool) {
    let mut a = vec!["verify-ref"];
    a.extend_from_slice(args);
    let (out, err, ok) = run_in(root, &a);
    (format!("{out}{err}"), ok)
}

/// Lo que bilinker escribe pasa su propia verificación. Es el piso: sin esto, el
/// hook rechazaría los pushes legítimos.
#[test]
fn verify_ref_accepts_what_bilinker_writes() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);
    run_in(&root, &["sync"]);

    let (out, ok) = verify(&root, &[&bref]);
    assert!(ok, "la ref que bilinker escribió no verifica:\n{out}");
    assert!(out.contains("ok"), "{out}");
    assert!(out.contains("la firma no se verificó"),
            "sin allowlist hay que decir que no se verificó, no callarse:\n{out}");
}

/// Borrar la ref no está permitido: sin esto, "sólo avanza" se esquiva borrándola y
/// empujándola de nuevo.
#[test]
fn verify_ref_rejects_a_delete() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));
    let tip = rev(&root, &bref);
    let ceros = "0".repeat(40);

    let (out, ok) = pre_receive(&root, &format!("{tip} {ceros} {bref}"));
    assert!(!ok, "un delete se rechaza:\n{out}");
    assert!(out.contains("borrar la ref"), "y se dice por qué:\n{out}");
}

/// La ref es append-only: un no-fast-forward es una reescritura, y reescribirla deja
/// sin baseline a toda aceptación del repo.
#[test]
fn verify_ref_rejects_a_non_fast_forward() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));
    let viejo = rev(&root, &bref);

    // Una historia que no desciende de ésta: el mismo árbol, sin padre.
    let tree = rev(&root, &format!("{viejo}^{{tree}}"));
    let nuevo = git_out(&root, &["commit-tree", &tree, "-m", "otra historia"]);
    let nuevo = nuevo.trim().to_string();

    let (out, ok) = pre_receive(&root, &format!("{viejo} {nuevo} {bref}"));
    assert!(!ok, "un no-fast-forward se rechaza:\n{out}");
    assert!(out.contains("append-only"), "y se dice por qué:\n{out}");
}

/// Un commit con la forma vieja —absorbe y decide a la vez— se rechaza. Es la
/// invariante 4, leída del lado que puede decir que no.
#[test]
fn verify_ref_rejects_a_commit_that_absorbs_and_decides() {
    let (_t, root, uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    let viejo = rev(&root, &bref);
    let malo = handmade_commit(&root, &bref, &uuid, true, "accept 00000000-0000-4000-8000-000000000000.0");
    let (out, ok) = verify(&root, &[&format!("{viejo}..{malo}")]);
    assert!(!ok, "absorber y decidir a la vez se rechaza:\n{out}");
    assert!(out.contains("absorbe y decide"), "y se dice por qué:\n{out}");
}

/// Un capture no se modifica: su id es el hash de su ubicación, así que cambiarlo le
/// cambia el nombre. Un capture con el mismo nombre y otro contenido es una
/// ubicación reescrita bajo una identidad ajena.
#[test]
fn verify_ref_rejects_a_modified_capture() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));
    let viejo = rev(&root, &bref);

    let cap = fs::read_dir(root.join(".bilink/capture")).unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("yaml"))
        .expect("hay captures").file_name().to_string_lossy().to_string();

    fs::write(root.join(format!(".bilink/capture/{cap}")), "file: otra/cosa.md\n").unwrap();
    let malo = commit_worktree_bilinks(&root, &bref, "accept 00000000-0000-4000-8000-000000000000.0");

    let (out, ok) = verify(&root, &[&format!("{viejo}..{malo}")]);
    assert!(!ok, "modificar un capture se rechaza:\n{out}");
    assert!(out.contains("un capture no se modifica"), "y se dice por qué:\n{out}");
}

/// **Nadie aprueba en nombre de otro.** Es la fila que convierte `agree` de
/// atribución en atestación, y no necesita traducir ningún nombre a ninguna clave:
/// los dos extremos se comparan contra el mismo campo, el autor.
#[test]
fn verify_ref_rejects_adding_someone_else_to_agree() {
    let (_t, root, uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));
    let viejo = rev(&root, &bref);

    // Pablo escribe `- ana` a mano, sobre el bilink del árbol.
    let path = root.join(format!(".bilink/{uuid}.yaml"));
    let texto = fs::read_to_string(&path).unwrap()
        .replace("    accepted:\n      agree:\n      - t\n",
                 "    accepted:\n      agree:\n      - ana\n      - t\n");
    fs::write(&path, &texto).unwrap();
    assert!(texto.contains("- ana"), "el fixture tiene que haber cambiado algo:\n{texto}");

    as_person(&root, "pablo");
    let malo = commit_worktree_bilinks(&root, &bref, &format!("accept {uuid}.0"));

    let (out, ok) = verify(&root, &[&format!("{viejo}..{malo}")]);
    assert!(!ok, "aprobar en nombre de otro se rechaza:\n{out}");
    assert!(out.contains("- ana") && out.contains("pablo"),
            "y se dice quién agregó a quién:\n{out}");
}

/// **La gramática no vuelve para atrás.** El prefijo anterior pasa una vez —o el
/// primer push de un repo que cortó antes se rechazaría entero— y esa puerta se
/// cierra con una regla de orden.
#[test]
fn the_prefix_before_the_grammar_passes_once_and_cannot_come_back() {
    let (_t, root, uuid, x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    // Una ref como la de un repo que cortó antes de que la gramática existiera: su
    // historia entera es de la forma vieja, y el primer push la empuja completa.
    let corte = {
        let tree = rev(&root, &format!("{bref}^{{tree}}"));
        let sha = git_out(&root, &["commit-tree", &tree, "-p", &x, "-m", "corte 005"]);
        sha.trim().to_string()
    };
    git(&root, &["update-ref", &bref, &corte]);
    let viejo_estilo = pre_grammar_commit(&root, &bref, &uuid, "accept .: 9 endpoint(s)");
    let (out, ok) = verify(&root, &[&bref]);
    assert!(ok, "el prefijo anterior a la gramática no se rechaza:\n{out}");
    assert!(out.contains("anteriores a la gramática"), "y se dice:\n{out}");

    // Ahora uno **con** la gramática, y encima otro sin ella: eso ya no es historia
    // vieja, es alguien esquivando la verificación.
    let con_gramatica = commit_worktree_bilinks(
        &root, &bref, &format!("accept {uuid}.0"));
    let vuelta = pre_grammar_commit(&root, &bref, &uuid, "accept .: otra vez sin trailer");

    let (out, ok) = verify(&root, &[&format!("{viejo_estilo}..{vuelta}")]);
    let _ = con_gramatica;
    assert!(!ok, "volver a la forma vieja encima de la nueva se rechaza:\n{out}");
    assert!(out.contains("no vuelve para atrás"), "y se dice por qué:\n{out}");
}

/// La firma es lo que ata el commit a una persona. Sin una clave de la allowlist, no
/// entra.
#[test]
fn verify_ref_rejects_a_commit_signed_by_a_key_outside_the_allowlist() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));
    let viejo = rev(&root, &bref);

    // Una clave, y una allowlist que la tiene.
    let key = root.join("id_ed25519");
    let kg = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "t@t", "-f"])
        .arg(&key).status();
    if !matches!(kg, Ok(st) if st.success()) {
        eprintln!("sin ssh-keygen: se saltea");
        return;
    }
    let pubkey = fs::read_to_string(root.join("id_ed25519.pub")).unwrap();
    let allow = root.join("allowed_signers");
    fs::write(&allow, format!("t@t {}", pubkey.trim())).unwrap();

    // Un commit firmado con esa clave verifica.
    git(&root, &["config", "gpg.format", "ssh"]);
    git(&root, &["config", "user.signingkey", &key.display().to_string()]);
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");
    run_in(&root, &["check", "."]);
    git(&root, &["config", "commit.gpgsign", "true"]);
    run_in(&root, &["accept", "."]);
    git(&root, &["config", "commit.gpgsign", "false"]);

    let firmado = rev(&root, &bref);
    let (out, ok) = verify(&root, &[&format!("{viejo}..{firmado}"),
                                    "--signers", &allow.display().to_string()]);
    assert!(ok, "un commit firmado por una clave de la allowlist entra:\n{out}");
    assert!(out.contains("firmados"), "y se dice que se verificó:\n{out}");

    // Y uno sin firma, no.
    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtro contenido.\n").unwrap();
    commit(&root, "la spec cambia");
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    let (out, ok) = verify(&root, &[&format!("{firmado}..{}", rev(&root, &bref)),
                                    "--signers", &allow.display().to_string()]);
    assert!(!ok, "sin firma no entra:\n{out}");
    assert!(out.contains("sin firma de la allowlist"), "y se dice por qué:\n{out}");
}

/// El hook lee `<viejo> <nuevo> <ref>` y **ignora lo que no es de la ref**: no opina
/// sobre las ramas del proyecto.
#[test]
fn the_hook_ignores_refs_that_are_not_bilink() {
    let (_t, root, _uuid, _x) = cut_over();
    let main = branch_of(&root);
    let tip = rev(&root, "HEAD");
    let ceros = "0".repeat(40);

    let (out, ok) = pre_receive(&root, &format!("{tip} {ceros} refs/heads/{main}"));
    assert!(ok, "un delete de una rama del proyecto no es asunto de este hook:\n{out}");
}

// ─── helpers de `1g` ───────────────────────────────────────────────────────

/// Corre `verify-ref --stdin` con las líneas dadas, como haría un `pre-receive`.
fn pre_receive(root: &Path, lines: &str) -> (String, bool) {
    use std::io::Write;
    let mut child = std::process::Command::new(bilinker())
        .current_dir(root)
        .args(["verify-ref", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("verify-ref");
    child.stdin.as_mut().unwrap().write_all(lines.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    (
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
        out.status.success(),
    )
}

/// Un commit sobre la ref escrito a mano, para poder fabricar las formas que
/// bilinker no escribe. `absorbe` le agrega el tip del proyecto como segundo padre.
fn handmade_commit(root: &Path, bref: &str, uuid: &str, absorbe: bool, msg: &str) -> String {
    let tip = rev(root, bref);
    let path = root.join(format!(".bilink/{uuid}.yaml"));
    let texto = format!("{}# a mano\n", fs::read_to_string(&path).unwrap());
    fs::write(&path, texto).unwrap();
    let sha = commit_worktree_bilinks(root, bref, msg);
    if !absorbe {
        return sha;
    }
    // El mismo árbol, con el commit del proyecto de segundo padre.
    let tree = rev(root, &format!("{sha}^{{tree}}"));
    let proyecto = rev(root, "HEAD");
    let msg = format!("{msg}\n\nBilinker-Version: {}", bilinker::refmsg::VERSION);
    let con_dos = git_out(root, &["commit-tree", &tree, "-p", &tip, "-p", &proyecto, "-m", &msg]);
    let con_dos = con_dos.trim().to_string();
    git(root, &["update-ref", bref, &con_dos]);
    con_dos
}

/// Un commit **sin** `Bilinker-Version`, o sea con la forma que escribía el código
/// anterior a `1f`. Es lo que un repo que cortó antes tiene en su historia.
fn pre_grammar_commit(root: &Path, bref: &str, uuid: &str, msg: &str) -> String {
    let path = root.join(format!(".bilink/{uuid}.yaml"));
    let texto = format!("{}# a mano\n", fs::read_to_string(&path).unwrap());
    fs::write(&path, texto).unwrap();
    let tip = rev(root, bref);
    let tree = rev(root, &format!("{tip}^{{tree}}"));
    let sha = git_out(root, &["commit-tree", &tree, "-p", &tip, "-m", msg]);
    let sha = sha.trim().to_string();
    git(root, &["update-ref", bref, &sha]);
    sha
}

/// Un commit de un padre sobre la ref con el `.bilink/` del árbol de trabajo, sin
/// pasar por bilinker. Es lo que permite fabricar un commit inválido.
fn commit_worktree_bilinks(root: &Path, bref: &str, msg: &str) -> String {
    // Con el trailer: sin él, el commit cae en "anterior a la gramática" y nunca se
    // llega a la verificación que el test quiere ejercer.
    let msg = &format!("{msg}\n\nBilinker-Version: {}", bilinker::refmsg::VERSION);
    let tip = rev(root, bref);
    let index = root.join(".git/handmade-index");
    let _ = fs::remove_file(&index);
    let g = |args: &[&str]| -> String {
        let out = std::process::Command::new("git")
            .current_dir(root)
            .env("GIT_INDEX_FILE", &index)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    g(&["read-tree", &tip]);
    for entry in walk_bilinks(root) {
        g(&["add", "-f", "--", &entry]);
    }
    let tree = g(&["write-tree"]);
    let sha = git_out(root, &["commit-tree", &tree, "-p", &tip, "-m", msg]);
    let sha = sha.trim().to_string();
    git(root, &["update-ref", bref, &sha]);
    sha
}

/// Los archivos de `.bilink/` que se commitean — sin `cache/`, `index/` ni `head`.
fn walk_bilinks(root: &Path) -> Vec<String> {
    fn rec(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().to_string();
            if ["cache", "index", "head"].iter().any(|d| rel.starts_with(&format!(".bilink/{d}"))) {
                continue;
            }
            if p.is_dir() { rec(&p, base, out) } else { out.push(rel) }
        }
    }
    let mut out = Vec::new();
    rec(&root.join(".bilink"), root, &mut out);
    out.sort();
    out
}

// ─── task `1n`: quiénes aprobaron ──────────────────────────────────────────

/// Quién acepta, para los tests que necesitan más de una persona.
fn as_person(root: &Path, name: &str) {
    git(root, &["config", "user.name", name]);
    git(root, &["config", "user.email", &format!("{name}@t")]);
}

/// El `accepted` de un endpoint, tal como está en el archivo.
fn accepted_of(root: &Path, uuid: &str, n: u8) -> bilink_format::Accepted {
    let bl = bilink_format::BiLink::load(&root.join(format!(".bilink/{uuid}.yaml"))).unwrap();
    bl.endpoint.get(n).accepted.clone().expect("el endpoint está aceptado")
}

fn agree_of(root: &Path, uuid: &str, n: u8) -> Vec<String> {
    accepted_of(root, uuid, n).agree.into_iter().collect()
}

/// Aceptar escribe quién aprobó, y **el primero entra solo**: si no, el campo
/// significaría "los que *además* aprobaron", que es otra cosa.
#[test]
fn accepting_writes_who_approved() {
    let (_t, root, uuid) = accepted_layer();
    assert_eq!(agree_of(&root, &uuid, 0), vec!["t"]);
    assert_eq!(agree_of(&root, &uuid, 1), vec!["t"]);
}

/// **Un segundo aprobador tiene algo que escribir.** Sobre un endpoint ya `OK` no
/// cambiaba ningún byte: no había diff, ni commit, ni firma. Ahora es un diff de una
/// línea sobre un commit propio.
#[test]
fn a_second_endorsement_adds_a_name_and_writes_a_commit() {
    let (_t, root, uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    assert!(check_states(&root).trim().is_empty(), "el endpoint arranca OK");
    let before = rev(&root, &bref);

    as_person(&root, "ana");
    let (out, stderr, ok) = run_in(&root, &["accept", &format!("{uuid}.0")]);
    assert!(ok, "endosar un endpoint OK falló:\n{stderr}");
    assert!(out.contains("agree: ana, t"), "y lo dice:\n{out}");

    assert_eq!(agree_of(&root, &uuid, 0), vec!["ana", "t"], "alfabético, no cronológico");
    assert_ne!(before, rev(&root, &bref), "el endoso produjo un commit");

    // Y no cambió el estado: `OK` no depende de cuántos aprobaron.
    assert!(check_states(&root).trim().is_empty(), "sigue OK con un aprobador más");
}

/// **Un nombre por línea, y por eso `git blame` atribuye cada endoso por separado.**
/// En flow, N actos colapsan en un lugar y blame devuelve el del último.
#[test]
fn blame_attributes_each_endorsement_to_the_commit_that_added_it() {
    let (_t, root, uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    as_person(&root, "ana");
    run_in(&root, &["accept", &format!("{uuid}.0")]);
    as_person(&root, "pablo");
    run_in(&root, &["accept", &format!("{uuid}.0")]);

    assert_eq!(agree_of(&root, &uuid, 0), vec!["ana", "pablo", "t"]);

    // El blame va sobre el archivo **de la ref**: ahí es donde viven los bilinks.
    let file = format!(".bilink/{uuid}.yaml");
    let blame = git_out(&root, &["blame", "--line-porcelain", &bref, "--", &file]);

    // De cada línea `- <nombre>`, el commit y el autor que la escribieron.
    let mut por_nombre: std::collections::BTreeMap<String, (String, String)> = Default::default();
    let mut commit = String::new();
    let mut autor = String::new();
    for line in blame.lines() {
        if let Some(rest) = line.strip_prefix("author ") { autor = rest.to_string(); }
        else if line.starts_with('\t') {
            if let Some(nombre) = line.trim().strip_prefix("- ") {
                por_nombre.insert(nombre.to_string(), (commit.clone(), autor.clone()));
            }
        } else if line.len() >= 40 && line[..40].chars().all(|c| c.is_ascii_hexdigit()) {
            commit = line[..40].to_string();
        }
    }

    let (c_ana, a_ana) = por_nombre.get("ana").expect("la línea de ana");
    let (c_pablo, a_pablo) = por_nombre.get("pablo").expect("la línea de pablo");
    assert_ne!(c_ana, c_pablo, "cada endoso es su propio commit");
    assert_eq!(a_ana, "ana", "y su autor es quien endosó");
    assert_eq!(a_pablo, "pablo");
}

/// Publicar dos veces la misma aprobación no dice nada nuevo: no hay diff, así que
/// no hay commit.
#[test]
fn endorsing_twice_writes_nothing() {
    let (_t, root, uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    as_person(&root, "ana");
    run_in(&root, &["accept", &format!("{uuid}.0")]);
    let after_first = rev(&root, &bref);

    let (out, _, ok) = run_in(&root, &["accept", &format!("{uuid}.0")]);
    assert!(ok, "repetir no es un error");
    assert!(out.contains("nada que agregar"), "y se dice:\n{out}");
    assert_eq!(after_first, rev(&root, &bref), "no se escribió ningún commit");
    assert_eq!(agree_of(&root, &uuid, 0), vec!["ana", "t"], "y no se duplicó");
}

/// **Cambian los valores, se vacía la lista.** Quien aprobó el hash anterior no
/// aprobó el nuevo, y arrastrar su nombre le atribuiría una decisión que no tomó.
#[test]
fn changing_the_values_empties_the_list() {
    let (_t, root, uuid, _x) = cut_over();

    as_person(&root, "ana");
    run_in(&root, &["accept", &format!("{uuid}.0")]);
    assert_eq!(agree_of(&root, &uuid, 0), vec!["ana", "t"]);

    // El fragmento cambia y alguien más lo aprueba: los anteriores no aprobaron esto.
    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtro contenido.\n").unwrap();
    commit(&root, "la spec cambia");
    as_person(&root, "pablo");
    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", &format!("{uuid}.0")]);
    assert!(ok, "accept falló:\n{stderr}");

    assert_eq!(agree_of(&root, &uuid, 0), vec!["pablo"],
               "los aprobadores del contenido anterior quedan en sus commits, no acá");
}

/// **Un endpoint `path` no copia el `agree` del vecino.** Los de allá aprobaron ese
/// fragmento; los de acá aprobaron esta copia.
#[test]
fn a_path_endpoint_does_not_copy_the_agree_of_its_neighbour() {
    let (_t, root) = isolated_git_workspace();
    let impl_dir = root.join(".stratum/impl");
    fs::create_dir_all(impl_dir.join("src")).unwrap();
    fs::write(impl_dir.join("src/Service.java"),
              "public class Service {\n    public void run() {}\n}\n").unwrap();
    commit(&root, "la capa de abajo");

    let (stdout, stderr, ok) = run_in(&root, &[
        "chain", "new", "--tip", "docs/spec.md", "--tip", ">impl/src/Service.java",
    ]);
    assert!(ok, "chain new falló:\n{stderr}");
    let uuid = stdout.lines().find_map(|l| l.strip_prefix("Created chain: "))
        .expect("uuid").trim().to_string();

    // Ana aprueba el fragmento, abajo. Pablo aprueba la copia, arriba.
    as_person(&root, "ana");
    run_in(&impl_dir, &["check", "."]);
    let (_, stderr, ok) = run_in(&impl_dir, &["accept", &format!("{uuid}.1")]);
    assert!(ok, "accept del fragmento falló:\n{stderr}");

    as_person(&root, "pablo");
    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", &format!("{uuid}.1")]);
    assert!(ok, "accept de la copia falló:\n{stderr}");

    assert_eq!(agree_of(&impl_dir, &uuid, 1), vec!["ana"], "abajo, quien aprobó el fragmento");
    assert_eq!(agree_of(&root, &uuid, 1), vec!["pablo"],
               "arriba, quien aprobó **esta copia** — y nadie del otro lado de la cadena");
}

/// **`adopt` une los dos `agree` sin reportar conflicto.** Es la diferencia con
/// `commit`, el campo que no está en `accepted`: acá la resolución es correcta y
/// única.
#[test]
fn adopt_unites_the_two_agree_without_conflict() {
    let (_t, root, main, uuid) = two_tracked_branches();
    let same = "public class Service {\n    public void run() { int x = 1; }\n}\n";

    as_person(&root, "ana");
    decide_on(&root, &main, same);
    as_person(&root, "pablo");
    decide_on(&root, "feature/x", same);

    // Los mismos valores, listas distintas.
    assert_eq!(agree_of(&root, &uuid, 1), vec!["pablo"]);

    let (stdout, stderr, ok) = run_in(&root, &["adopt", &main]);
    assert!(ok, "adopt falló:\n{stderr}\n{stdout}");
    assert!(!stdout.contains("conflicto"), "unir no es un conflicto:\n{stdout}");

    assert_eq!(agree_of(&root, &uuid, 1), vec!["ana", "pablo"],
               "el resultado dice algo verdadero que antes no se podía decir: los dos aprobaron");
}

// ─── task `1f`: el mensaje es el comando ───────────────────────────────────

/// **Todo commit que bilinker escribe sobre la ref parsea contra la gramática**, y
/// lleva `Bilinker-Version`. Es el contrato del que cuelga el replay.
#[test]
fn every_message_written_on_the_ref_parses_against_the_grammar() {
    use bilinker::refmsg::{read, Read};

    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    // Un poco de cada verbo: corte (el fixture), absorción, y decisiones.
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);

    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtro contenido.\n").unwrap();
    commit(&root, "y la spec");
    run_in(&root, &["sync"]);
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "--content", "."]);

    let commits = ref_commits(&root, &bref);
    assert!(commits.len() >= 5, "hacen falta varios actos para que valga:\n{commits:?}");

    for c in &commits {
        let msg = git_out(&root, &["log", "-1", "--format=%B", c]);
        match read(&msg) {
            Ok(Read::Parsed(m)) => {
                assert_eq!(m.version, bilinker::refmsg::VERSION,
                           "{c} lleva otra versión:\n{msg}");
            }
            Ok(Read::PreGrammar) => panic!("{c} no lleva Bilinker-Version:\n{msg}"),
            Err(e) => panic!("{c} no parsea: {e}\n{msg}"),
        }
    }
}

/// El corte no tiene verbo propio: su mensaje es `track <rama>`, igual que el de un
/// `track` que hereda, y **lo que los separa son los padres**.
#[test]
fn the_cut_and_an_inheriting_track_share_a_verb_and_differ_in_their_parents() {
    use bilinker::refmsg::{parse, RefCommand};

    let (_t, root, _uuid, x) = cut_over();
    let main = branch_of(&root);

    // `●0` — el corte: `track` sin candidato del que heredar.
    let corte = rev(&root, &format!("refs/bilink/{main}"));
    let msg = git_out(&root, &["log", "-1", "--format=%B", &corte]);
    assert_eq!(parse(&msg).expect("parsea").command,
               RefCommand::Track { branch: main.clone() });
    assert_eq!(parents_of(&root, &corte), vec![x], "un padre, y del proyecto");
    assert!(msg.contains("corte"), "el acto se nombra en la prosa, no en el verbo:\n{msg}");

    // `●a` — otra rama, ésta con candidato: el mismo verbo, y dos padres.
    git(&root, &["checkout", "-q", "-b", "feature/x"]);
    let (_, stderr, ok) = run_in(&root, &["track", "feature/x"]);
    assert!(ok, "track falló:\n{stderr}");

    let heredado = rev(&root, "refs/bilink/feature/x");
    let msg = git_out(&root, &["log", "-1", "--format=%B", &heredado]);
    assert_eq!(parse(&msg).expect("parsea").command,
               RefCommand::Track { branch: "feature/x".to_string() },
               "el mismo verbo para los dos nacimientos");
    assert_eq!(parents_of(&root, &heredado).len(), 2,
               "y lo que los separa son los padres: heredar tiene dos");
}

/// **La gramática no es retroactiva.** Los commits que el código viejo dejó en la ref
/// no se pueden reescribir, así que se leen como anteriores a ella y no como un
/// error — y conviven con los nuevos en la misma ref.
#[test]
fn the_history_written_before_the_grammar_is_read_as_pre_grammar() {
    use bilinker::refmsg::{read, Read};

    let (_t, root, _uuid, _x) = cut_over();
    let branch = branch_of(&root);
    let bref = format!("refs/bilink/{branch}");

    // Un commit con la forma que escribía el código anterior a `1e` y a `1f`: un
    // merge que absorbe y decide a la vez, con el mensaje en prosa.
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");
    let e = rev(&root, "HEAD");
    let tip = rev(&root, &bref);
    let tree = rev(&root, &format!("{tip}^{{tree}}"));
    let viejo = git_out(&root, &["commit-tree", &tree, "-p", &tip, "-p", &e,
                                 "-m", "accept .: 9 endpoint(s)"]);
    let viejo = viejo.trim().to_string();
    git(&root, &["update-ref", &bref, &viejo]);

    assert_eq!(read(&git_out(&root, &["log", "-1", "--format=%B", &viejo])).unwrap(),
               Read::PreGrammar,
               "un commit sin el trailer es anterior a la gramática, no un error");

    // Y la ref sigue usable: el acto siguiente escribe con la gramática nueva encima.
    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", "."]);
    assert!(ok, "accept sobre una ref con historia vieja falló:\n{stderr}");

    let msg = git_out(&root, &["log", "-1", "--format=%B", &bref]);
    assert!(matches!(read(&msg), Ok(Read::Parsed(_))), "el nuevo sí parsea:\n{msg}");
}

/// `apply` escribe **un commit por `link` repuntado**, no uno por invocación: el
/// mensaje `apply <uuid>.<N> <capture-nuevo>` nombra un endpoint, y uno que nombrara
/// tres no sería reproducible contra el árbol de un solo padre.
#[test]
fn apply_writes_one_commit_per_repointed_link() {
    use bilinker::refmsg::{parse, RefCommand};

    let (_t, root, uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    // El proyecto avanza por otro lado, para que haya algo que absorber.
    fs::write(root.join("src/Other.java"), "public class Other {}\n").unwrap();
    commit(&root, "otro archivo");

    // Los dos endpoints se mueven a la vez: dos fixes, una invocación. El rename se
    // deja en el índice — `git diff -M` lo lee de ahí, y commitearlo lo esconde.
    git(&root, &["mv", "docs/spec.md", "docs/renombrada.md"]);
    git(&root, &["mv", "src/Service.java", "src/Servicio.java"]);

    let before = rev(&root, &bref);
    run_in(&root, &["check", "."]);
    let (out, stderr, ok) = run_in(&root, &["apply", "-y"]);
    assert!(ok, "apply falló:\n{out}\n{stderr}");

    let nuevos = git_out(&root, &["rev-list", "--reverse", "--first-parent",
                                  &format!("{before}..{bref}")]);
    let nuevos: Vec<&str> = nuevos.lines().collect();
    assert_eq!(nuevos.len(), 3, "una absorción y un commit por fix:\n{nuevos:?}");
    assert_eq!(parents_of(&root, nuevos[0]).len(), 2, "primero la absorción");

    let mut endpoints = Vec::new();
    for d in &nuevos[1..] {
        assert_eq!(parents_of(&root, d).len(), 1, "las decisiones tienen un padre");
        let msg = git_out(&root, &["log", "-1", "--format=%B", d]);
        match parse(&msg).expect("el mensaje de apply parsea").command {
            RefCommand::Apply { uuid: u, n, capture } => {
                assert_eq!(u, uuid, "cada commit nombra su bilink");
                assert_eq!(capture.len(), 32, "y el capture nuevo, entero:\n{msg}");
                endpoints.push(n);
            }
            other => panic!("el verbo tiene que ser apply, no {other:?}"),
        }
        assert!(msg.contains("Invocation: bilinker apply -y"),
                "lo que se tipeó va como auditoría:\n{msg}");
    }
    endpoints.sort();
    assert_eq!(endpoints, vec![0, 1], "un commit por endpoint repuntado");
}

// ─── La superficie de revisión ─────────────────────────────────────────────

/// `bilinker-has-its-own-status` — los cambios de bilinker se ven en su índice, no
/// en el del proyecto.
#[test]
fn the_bilink_changes_are_visible_to_bilinker_and_invisible_to_git() {
    let (_t, root, uuid, _x) = cut_over();

    // Un cambio en .bilink/ sin commitear sobre la ref.
    let path = root.join(format!(".bilink/{uuid}.yaml"));
    fs::write(&path, format!("{}# a mano\n", fs::read_to_string(&path).unwrap())).unwrap();

    let (mine, stderr, ok) = run_in(&root, &["status", "--porcelain"]);
    assert!(ok, "status --porcelain falló:\n{stderr}");
    assert!(mine.contains(&format!("M .bilink/{uuid}.yaml")),
            "el índice propio lo ve:\n{mine}");

    let theirs = git_out(&root, &["status", "--porcelain"]);
    assert!(!theirs.contains(".bilink"),
            "el índice del proyecto no, porque los tiene excluidos:\n{theirs}");
}

/// `first-parent-shows-only-bilinks` — el registro de decisiones no lleva una sola
/// línea del historial del proyecto.
#[test]
fn log_shows_only_the_acts_of_the_ref() {
    let (_t, root, _uuid, _x) = cut_over();

    for (content, msg) in [
        ("public class Service {\n    public void run() { int x = 1; }\n}\n", "uno"),
        ("public class Service {\n    public void run() { int x = 2; }\n}\n", "dos"),
    ] {
        fs::write(root.join("src/Service.java"), content).unwrap();
        commit(&root, msg);
        run_in(&root, &["check", "."]);
        run_in(&root, &["accept", "."]);
    }

    let (out, stderr, ok) = run_in(&root, &["log"]);
    assert!(ok, "log falló:\n{stderr}");

    for line in out.lines() {
        assert!(!line.contains(" uno") && !line.contains(" dos"),
                "el log de la ref no muestra commits del proyecto:\n{out}");
    }
    assert!(out.lines().any(|l| l.contains("corte")), "y llega hasta el corte:\n{out}");
    assert_eq!(out.lines().filter(|l| l.contains("accept")).count(), 2,
               "un acto por invocación:\n{out}");
}

/// El log se corta en el corte: sin freno seguiría por la historia del proyecto.
#[test]
fn log_stops_at_the_cut() {
    let (_t, root, _uuid, _x) = cut_over();
    let (out, _, ok) = run_in(&root, &["log"]);
    assert!(ok);
    assert_eq!(out.lines().count(), 1, "sólo el corte, no la historia del proyecto:\n{out}");
}

/// `bilinker log <suya> --not <mía>` es lo que contesta qué actos hubo del otro lado.
#[test]
fn log_can_exclude_what_this_branch_already_has() {
    let (_t, root, main, _uuid) = two_tracked_branches();
    decide_on(&root, &main, "public class Service {\n    public void run() { int x = 1; }\n}\n");

    git(&root, &["checkout", "-q", "feature/x"]);
    run_in(&root, &["init"]);

    let (out, stderr, ok) = run_in(&root, &["log", &main, "--not", "feature/x"]);
    assert!(ok, "log falló:\n{stderr}");
    assert!(out.lines().any(|l| l.contains("accept")),
            "el acto de main que feature/x no tiene:\n{out}");
    assert!(!out.lines().any(|l| l.contains("corte")),
            "y no lo que las dos comparten:\n{out}");
}

/// `bilinker diff` muestra lo que `git diff` no puede mostrar.
#[test]
fn diff_shows_the_uncommitted_bilink_changes() {
    let (_t, root, uuid, _x) = cut_over();
    let path = root.join(format!(".bilink/{uuid}.yaml"));
    fs::write(&path, format!("{}# a mano\n", fs::read_to_string(&path).unwrap())).unwrap();

    let (out, stderr, ok) = run_in(&root, &["diff"]);
    assert!(ok, "diff falló:\n{stderr}");
    assert!(out.contains("# a mano"), "el diff propio lo muestra:\n{out}");

    let theirs = git_out(&root, &["diff"]);
    assert!(!theirs.contains("a mano"), "el del proyecto no:\n{theirs}");
}

// ─── La ref vuelve inmutable el commit de una aceptación ───────────────────
//
// Task `16`. Es la clase de propiedad que se cree evidente y falla por un detalle
// de plumbing, así que se verifica, no se supone.

/// Deja la rama con `A · B(aceptado) · C`, acepta en `B`, y devuelve `(uuid, B, A)`.
fn accepted_then_changed() -> (tempfile::TempDir, PathBuf, String, String, String) {
    let (tmp, root, uuid, _x) = cut_over();

    fs::write(root.join("docs/otro.md"), "# Otro\n").unwrap();
    commit(&root, "A — un commit intermedio");
    let a = rev(&root, "HEAD");

    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "B — el contenido que se acepta");
    let b = rev(&root, "HEAD");
    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", "."]);
    assert!(ok, "accept falló:\n{stderr}");

    // El fragmento cambia después, para que el endpoint quede no-OK y `--diff`
    // tenga que recuperar el texto aceptado de la historia.
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 99; }\n}\n").unwrap();
    commit(&root, "C — y después cambia");

    (tmp, root, uuid, b, a)
}

/// Aplasta `A..<rama>` en un solo commit sobre `A`.
///
/// **Un rebase a secas no serviría acá, y ésa es media conclusión de la task `16`:**
/// preserva el contenido, así que el fragmento aceptado aparece igual en el commit
/// reescrito y la derivación lo reencuentra sin ayuda de nadie. Lo que rompe de
/// verdad es un squash o un `filter-branch`, donde el contenido intermedio deja de
/// existir en ningún commit de la rama — y ahí el único lugar donde sigue estando es
/// la ref.
fn squash_over(root: &Path, branch: &str, a: &str) {
    git(root, &["reset", "-q", "--soft", a]);
    git(root, &["commit", "-qm", "B+C aplastados"]);
    let _ = branch;
}

/// Sin la ref, un rebase de la rama del proyecto por encima del commit aceptado
/// deja `accepted.hash` siendo un hash **que no se puede resolver a texto**: el
/// commit cacheado desapareció, y la derivación tampoco lo encuentra porque el
/// rebase reescribió los commits donde el fragmento tenía ese contenido.
///
/// Con la ref tiene que funcionar por cualquiera de los dos caminos: la ref absorbe
/// el commit del proyecto como segundo padre y **no se rebasea nunca**, así que git
/// lo conserva por ser alcanzable — deja de estar en la rama y sigue existiendo.
#[test]
fn a_rebase_does_not_destroy_the_text_of_an_acceptance() {
    let (_t, root, uuid, b, a) = accepted_then_changed();
    let main = branch_of(&root);

    // 2. Borrar cache/state: sin ella, el `commit` guardado no está y hay que
    //    derivarlo caminando la historia del archivo.
    let _ = fs::remove_dir_all(root.join(".bilink/cache"));

    // 3. Aplastar la rama por encima del commit aceptado.
    squash_over(&root, &main, &a);
    let after = git_out(&root, &["log", "--format=%H", &main]);
    assert!(!after.contains(&b), "el rebase sacó el commit aceptado de la rama");

    // Y sin embargo sigue existiendo, porque la ref lo alcanza.
    let kind = git_out(&root, &["cat-file", "-t", &b]);
    assert_eq!(kind.trim(), "commit",
               "la ref lo absorbió como segundo padre y no se rebasea nunca");

    // 4. El texto aceptado se sigue recuperando.
    run_in(&root, &["check", "."]);
    let (out, stderr, _) = run_in(&root, &["get", &format!("{}.1", &uuid[..8]), "--diff"]);
    assert!(out.contains("int x = 1"),
            "el texto aceptado se perdió tras el rebase:\n{out}\n{stderr}");
    assert!(out.contains("int x = 99"), "y el actual está del otro lado:\n{out}");
}

/// La otra mitad: la ref no sólo protege al `commit` guardado, protege también a la
/// **derivación**, que camina la historia del archivo y necesita que esos commits
/// sigan existiendo.
#[test]
fn the_ref_keeps_the_accepted_commit_reachable_after_a_rebase() {
    let (_t, root, _uuid, b, a) = accepted_then_changed();
    let main = branch_of(&root);
    let bref = format!("refs/bilink/{main}");

    squash_over(&root, &main, &a);

    let is_ancestor = |of: &str| {
        std::process::Command::new("git")
            .current_dir(&root)
            .args(["merge-base", "--is-ancestor", &b, of])
            .output().unwrap().status.success()
    };

    assert!(is_ancestor(&bref),
            "todo commit alguna vez absorbido queda alcanzable desde la ref para siempre");
    assert!(!is_ancestor(&main),
            "y la rama no lo protege: se rebasea, se force-pushea, se cambia");
}

/// El control negativo, sin el cual los dos tests de arriba no dicen nada.
///
/// El mismo escenario en un repo que **no** cortó a la ref: no hay nada que alcance
/// el commit aceptado, git lo deja inalcanzable, y el texto aceptado deja de poder
/// recuperarse. Es la objeción que ADR-0003 dejó abierta, reproducida.
#[test]
fn without_the_ref_the_same_rebase_loses_the_accepted_text() {
    let (_t, root, uuid) = accepted_layer();
    let main = branch_of(&root);

    fs::write(root.join("docs/otro.md"), "# Otro\n").unwrap();
    commit(&root, "A");
    let a = rev(&root, "HEAD");

    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "B — el contenido que se acepta");
    let b = rev(&root, "HEAD");
    run_in(&root, &["check", "."]);
    run_in(&root, &["accept", "."]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "la aceptación, en la rama"]);

    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 99; }\n}\n").unwrap();
    commit(&root, "C");

    let _ = fs::remove_dir_all(root.join(".bilink/cache"));
    squash_over(&root, &main, &a);

    assert!(git_out(&root, &["for-each-ref", "refs/bilink/"]).trim().is_empty(),
            "este repo no cortó: no hay ninguna ref");

    // El commit aceptado quedó fuera de toda ref. Sigue en el objeto store hasta que
    // pase el gc, así que lo que se verifica es lo que importa: nada lo alcanza.
    let reachable = git_out(&root, &["for-each-ref", "--format=%(refname)", "--contains", &b]);
    assert!(reachable.trim().is_empty(),
            "sin la ref, ninguna referencia alcanza el commit aceptado:\n{reachable}");

    // Y la derivación tampoco lo encuentra: el rebase reescribió los commits donde
    // el fragmento tenía el contenido aceptado.
    run_in(&root, &["check", "."]);
    let (out, _, _) = run_in(&root, &["get", &format!("{}.1", &uuid[..8]), "--diff"]);
    assert!(!out.contains("int x = 1"),
            "sin la ref el texto aceptado no se puede recuperar — y acá se recuperó:\n{out}");
}

/// La ref es **por repo**, y el recorrido se para en la frontera.
///
/// En accreta cada subsistema tiene su capa de implementación en un repo propio,
/// gitignoreado por el padre. Sin este freno el corte del padre se traga los bilinks
/// de los hijos: quedan en un snapshot cuyo árbol de código no los contiene, así que
/// ni la disyunción ni la fidelidad hablan de ellos.
#[test]
fn the_cut_does_not_swallow_the_bilinks_of_a_nested_repo() {
    let (_t, root, _uuid, _x) = cut_over();

    // Un subsistema con su propio repo adentro, como los `.stratum/impl` de accreta.
    let nested = root.join("subsystems/otro");
    fs::create_dir_all(nested.join("src")).unwrap();
    fs::write(nested.join("src/Lib.java"), "public class Lib {\n    public void go() {}\n}\n").unwrap();
    for args in [
        vec!["init", "-q"], vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "t"], vec!["add", "-A"], vec!["commit", "-qm", "init"],
    ] {
        std::process::Command::new("git").current_dir(&nested).args(&args).output().unwrap();
    }
    let (_, stderr, ok) = run_in(&nested, &["chain", "new",
        "--tip", "src/Lib.java:1:1", "--tip", "src/Lib.java:2:5"]);
    assert!(ok, "chain new en el repo anidado falló:\n{stderr}");
    assert!(nested.join(".bilink").is_dir(), "el anidado tiene sus propios bilinks");

    // El padre avanza y absorbe. Sus bilinks son los suyos, no los del hijo.
    fs::write(root.join("docs/otro.md"), "# Otro\n").unwrap();
    commit(&root, "el padre avanza");
    let (_, stderr, ok) = run_in(&root, &["sync"]);
    assert!(ok, "sync falló:\n{stderr}");

    let files = git_out(&root, &["ls-tree", "-r", "--name-only",
                                 &format!("refs/bilink/{}", branch_of(&root))]);
    assert!(!files.contains("subsystems/otro/.bilink"),
            "la ref del padre se tragó los bilinks de otro repo:\n{files}");
    assert!(files.contains(".bilink/"), "y sí lleva los propios:\n{files}");
}

// ─── El corte contra la forma real: varios repos ───────────────────────────
//
// Task `1a`. Lo multi-capa está cubierto por todos lados; lo multi-repo no lo
// estaba, y ahí vivía el bug que se escapó de los otros 92. Estos tests arman la
// forma de accreta —un padre con capas, más repos anidados con la suya, y una
// cadena que cruza— y corren el script del corte en cada repo.

/// Corre `scripts/corte-005.sh` sobre un repo, que es el runbook de la task `e`.
///
/// El script y el test son el mismo artefacto: lo que acá se verifica es lo que
/// alguien va a correr sobre los repos de verdad.
fn corte(repo: &Path) -> (String, String, bool) {
    let out = std::process::Command::new("bash")
        .arg(workspace().join("scripts/corte-005.sh"))
        .arg(repo)
        // El script llama a `bilinker`, y tiene que ser **este** binario.
        .env("PATH", format!("{}:{}",
             bilinker().parent().unwrap().display(),
             std::env::var("PATH").unwrap_or_default()))
        .output()
        .expect("failed to run corte-005.sh");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn git_init(dir: &Path) {
    for args in [
        vec!["init", "-q"], vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "t"],
    ] {
        std::process::Command::new("git").current_dir(dir).args(&args).output().unwrap();
    }
}

fn git_commit_all(dir: &Path, msg: &str) {
    std::process::Command::new("git").current_dir(dir).args(["add", "-A"]).output().unwrap();
    std::process::Command::new("git").current_dir(dir)
        .args(["commit", "-qm", msg]).output().unwrap();
}

/// La forma de accreta, en chico:
///
/// ```text
/// padre/                          repo A
///   docs/spec.md                    capa raíz
///   subsystems/uno/                 otra capa del MISMO repo
///   subsystems/uno/.stratum/impl/   repo B, gitignoreado por A
/// ```
///
/// Y una cadena que cruza de la capa `uno` al repo anidado, que es lo que hace
/// distinta a esta forma de un `.stratum/impl` que vive en el mismo repo.
fn accreta_shape() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let tmp = tempfile::tempdir().unwrap();
    let padre = tmp.path().to_path_buf();
    let impl_dir = padre.join("subsystems/uno/.stratum/impl");

    fs::create_dir_all(padre.join("docs")).unwrap();
    fs::create_dir_all(padre.join("subsystems/uno/concepts")).unwrap();
    fs::create_dir_all(impl_dir.join("src")).unwrap();

    // `subsystems/uno` es una **capa**, no un subdirectorio cualquiera: la raíz se
    // resuelve caminando hacia arriba hasta el primer `.bilink/` o `.git/`, así que
    // sin este directorio la capa colapsaría contra la raíz del padre. En accreta ya
    // existe; acá hay que ponerlo.
    fs::create_dir_all(padre.join("subsystems/uno/.bilink")).unwrap();

    fs::write(padre.join("docs/spec.md"), "# Spec\n\nLa raíz.\n").unwrap();
    fs::write(padre.join("subsystems/uno/concepts/cosa.md"), "# Cosa\n\nLa capa uno.\n").unwrap();
    fs::write(impl_dir.join("src/Service.java"),
              "public class Service {\n    public void run() {}\n}\n").unwrap();

    // El anidado es un repo propio, y el padre lo ignora.
    git_init(&impl_dir);
    git_commit_all(&impl_dir, "init");

    fs::write(padre.join(".gitignore"), "subsystems/*/.stratum/impl/\n").unwrap();
    git_init(&padre);
    git_commit_all(&padre, "init");

    // Una cadena que cruza la frontera: capa `uno` del padre ↔ repo anidado.
    let uno = padre.join("subsystems/uno");
    let (stdout, stderr, ok) = run_in(&uno, &[
        "chain", "new",
        "--tip", "concepts/cosa.md:1:1",
        "--tip", ">impl/src/Service.java:2:5",
    ]);
    assert!(ok, "chain new cruzando repos falló:\n{stderr}");
    let uuid = stdout.lines()
        .find_map(|l| l.strip_prefix("Created chain: "))
        .expect("uuid").trim().to_string();

    // Dos vueltas: la propagación por la cadena es unidireccional, desde el endpoint
    // estructural hacia los `path`, así que el lado que acepta primero deja al otro
    // con algo que copiar recién en la vuelta siguiente.
    for _ in 0..2 {
        for r in [&impl_dir, &uno, &padre] {
            run_in(r, &["check", "."]);
            run_in(r, &["accept", "."]);
        }
    }
    for r in [&impl_dir, &padre] {
        git_commit_all(r, "los bilinks, todavía en la rama");
    }

    (tmp, padre, impl_dir, uuid)
}

/// El script corta cada repo y sus verificaciones pasan sobre la forma real.
#[test]
fn the_cutover_script_runs_clean_on_the_accreta_shape() {
    let (_t, padre, impl_dir, _uuid) = accreta_shape();

    for repo in [&impl_dir, &padre] {
        let (out, err, ok) = corte(repo);
        assert!(ok, "el corte falló en {}:\n{out}\n{err}", repo.display());
        assert!(out.contains("verif:"), "las verificaciones tienen que correr:\n{out}");
        assert!(out.contains("cortado."), "y terminar:\n{out}");
    }
}

/// Cada ref lleva **sólo lo suyo**, a la profundidad de accreta: el padre tiene dos
/// capas en su propio árbol y un repo anidado que no es suyo.
#[test]
fn each_ref_carries_only_the_bilinks_of_its_own_repo() {
    let (_t, padre, impl_dir, _uuid) = accreta_shape();
    corte(&impl_dir);
    corte(&padre);

    let bref = format!("refs/bilink/{}", branch_of(&padre));
    let del_padre = git_out(&padre, &["ls-tree", "-r", "--name-only", &bref]);
    assert!(del_padre.contains("subsystems/uno/.bilink/"),
            "las capas del padre van en la misma ref:\n{del_padre}");
    assert!(!del_padre.contains(".stratum/impl/.bilink"),
            "y ninguna del repo anidado:\n{del_padre}");

    let del_impl = git_out(&impl_dir, &["ls-tree", "-r", "--name-only", &bref]);
    assert!(del_impl.contains(".bilink/"), "el anidado lleva los suyos:\n{del_impl}");
    assert!(!del_impl.contains("subsystems/"), "y nada del padre:\n{del_impl}");
}

/// **Lo que de verdad no estaba probado**: un endpoint `path` cuyo vecino vive en
/// otro repo, con los dos repos cortados.
///
/// El vecino se lee del árbol de trabajo, que está materializado. Que eso siga
/// valiendo cuando el vecino dejó de estar en ninguna rama es lo que había que ver.
#[test]
fn a_path_endpoint_still_resolves_across_a_repo_boundary_after_both_cut_over() {
    let (_t, padre, impl_dir, _uuid) = accreta_shape();
    corte(&impl_dir);
    corte(&padre);

    for (label, dir) in [("el anidado", impl_dir.clone()),
                         ("la capa que cruza", padre.join("subsystems/uno"))] {
        let (out, err, ok) = run_in(&dir, &["check", "."]);
        assert!(!out.contains("BROKEN") && !out.contains("TODO"),
                "el endpoint path dejó de ver a su vecino en {label}:\n{out}\n{err}");
        assert!(ok, "check en {label} no quedó limpio tras el corte:\n{out}\n{err}");
    }
}

/// `CHAIN_DIRTY` cruzando la frontera del repo, con **dos refs** de por medio.
///
/// La propagación se probaba entre capas del mismo repo, donde no hay dos refs ni
/// dos índices. Con dos repos, aceptar de un lado y del otro son dos actos que caen
/// en dos historias distintas — y eso es lo que se verifica acá.
#[test]
fn chain_dirty_propagates_across_a_repo_boundary_between_two_refs() {
    let (_t, padre, impl_dir, uuid) = accreta_shape();
    corte(&impl_dir);
    corte(&padre);

    let uno = padre.join("subsystems/uno");
    let bref = format!("refs/bilink/{}", branch_of(&padre));
    let ref_impl_before = rev(&impl_dir, &bref);
    let ref_padre_before = rev(&padre, &bref);

    // El fragmento del repo anidado cambia y alguien lo acepta ahí.
    fs::write(impl_dir.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&impl_dir, "el fragmento del anidado cambia");
    run_in(&impl_dir, &["check", "."]);
    let (_, stderr, ok) = run_in(&impl_dir, &["accept", "."]);
    assert!(ok, "accept en el anidado falló:\n{stderr}");

    assert_ne!(ref_impl_before, rev(&impl_dir, &bref),
               "el acto quedó en la ref del anidado");
    assert_eq!(ref_padre_before, rev(&padre, &bref),
               "y no tocó la del padre: son dos refs y dos actos");

    // Del otro lado de la frontera, el endpoint `path` lo detecta.
    let states = run_in(&uno, &["check", "."]).0;
    assert!(states.contains("CHAIN_DIRTY"),
            "el vecino re-aceptó y este lado tiene que verlo:\n{states}");

    // Y aceptarlo acá escribe en la ref del padre, no en la del anidado.
    let ref_impl_now = rev(&impl_dir, &bref);
    // El endpoint que propaga es el `path`, que en esta cadena es el `.1`. Se acepta
    // la capa entera, que es lo que alguien tipea.
    let (_, stderr, ok) = run_in(&uno, &["accept", "."]);
    assert!(ok, "accept del lado del padre falló:\n{stderr}");
    let _ = &uuid;

    assert_ne!(ref_padre_before, rev(&padre, &bref),
               "el acto del padre quedó en la ref del padre");
    assert_eq!(ref_impl_now, rev(&impl_dir, &bref),
               "y no tocó la del anidado");
    assert!(!check_states(&uno).contains("CHAIN_DIRTY"), "y quedó sincronizado");
}

/// El script se niega sobre un repo que ya cortó, en vez de hacer un segundo corte.
#[test]
fn the_cutover_script_refuses_a_repo_that_already_cut() {
    let (_t, _padre, impl_dir, _uuid) = accreta_shape();
    corte(&impl_dir);

    let (_, err, ok) = corte(&impl_dir);
    assert!(!ok, "cortar dos veces tiene que fallar");
    assert!(err.contains("ya cortó"), "y decir por qué:\n{err}");
}

/// Y sobre un árbol sucio: el corte parte de un árbol limpio, o el commit del paso
/// 1 se lleva trabajo que nadie revisó.
#[test]
fn the_cutover_script_refuses_a_dirty_tree() {
    let (_t, padre, _impl_dir, _uuid) = accreta_shape();
    fs::write(padre.join("docs/spec.md"), "# Spec\n\nsin commitear\n").unwrap();

    let (_, err, ok) = corte(&padre);
    assert!(!ok, "con el árbol sucio el corte tiene que negarse");
    assert!(err.contains("sin commitear"), "y decir por qué:\n{err}");
}

/// El ledger registra el corte, y en el commit del paso 1 — el que dice "este repo
/// movió sus bilinks a la ref".
#[test]
fn the_cutover_records_itself_in_the_ledger() {
    let (_t, _padre, impl_dir, _uuid) = accreta_shape();
    corte(&impl_dir);

    let ledger = fs::read_to_string(impl_dir.join(".accreta/migrations")).unwrap();
    assert!(ledger.contains("bilinker-005-ref-cutover"),
            "el corte tiene que quedar registrado:\n{ledger}");

    let x = git_out(&impl_dir, &["rev-parse", "HEAD"]);
    let en_x = git_out(&impl_dir, &["show", &format!("{}:.accreta/migrations", x.trim())]);
    assert!(en_x.contains("bilinker-005-ref-cutover"),
            "y en el mismo commit que saca .bilink/ del índice:\n{en_x}");
}

/// **La cache no puede conservar un `OK` que ya no es cierto.**
///
/// Hubo un fast-path que lo hacía: con un `OK` cacheado y el archivo sin cambios
/// desde el commit del contenido aceptado, devolvía `OK` sin volver a hashear. Su
/// premisa es un proxy —"¿el archivo cambió?"— de la pregunta real —"¿el fragmento
/// sigue hasheando a lo aceptado?"—, y las dos dejan de coincidir apenas el
/// `accepted.hash` deja de describir lo que hay, **sin que el archivo se toque**.
///
/// Este test lo fuerza por el camino más directo: se edita el `accepted.hash` a
/// mano. En producción llegó por otro —un cambio en cómo se resuelve el rango— pero
/// la forma es la misma, y lo que importa es que ningún atajo tape la diferencia.
#[test]
fn check_never_keeps_a_cached_ok_when_the_hash_no_longer_matches() {
    let (_t, root, uuid) = accepted_layer();
    let path = root.join(format!(".bilink/{uuid}.yaml"));

    // Con el archivo intacto y el estado en OK, la cache queda tibia.
    assert!(check_states(&root).trim().is_empty(), "arranca limpio");

    // El accepted.hash deja de describir el fragmento — y el archivo no se toca.
    let bl = fs::read_to_string(&path).unwrap();
    let doctored = bl.replacen("hash: ", "hash: ff", 1);
    assert_ne!(bl, doctored, "el hash tiene que haber cambiado");
    fs::write(&path, &doctored).unwrap();

    let states = check_states(&root);
    assert!(states.contains("ALTERED"),
            "el fragmento ya no coincide y check tiene que decirlo:\n{states}");
}

/// Y el corolario que hace que el bug importara: `accept` le cree a `check`, así
/// que un `OK` falso no es un reporte equivocado — es una aceptación que no ocurre.
#[test]
fn a_false_ok_would_silently_skip_the_acceptance() {
    let (_t, root, uuid) = accepted_layer();
    let path = root.join(format!(".bilink/{uuid}.yaml"));

    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "el fragmento cambia");

    let before = fs::read_to_string(&path).unwrap();
    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", "."]);
    assert!(ok, "accept falló:\n{stderr}");

    assert_ne!(before, fs::read_to_string(&path).unwrap(),
               "con el estado bien reportado, accept escribe la decisión");
    assert!(check_states(&root).trim().is_empty(), "y queda limpio");
}

// ─── La frontera entre proyectos ───────────────────────────────────────────
//
// US `h`. Los 18 escenarios de `scenarios/frontier.yaml`, todos entre **dos repos
// locales**: que el proveedor de verdad sea `hsi` es una circunstancia, no un
// requisito.

/// Un proveedor con una abstracción publicada y su ref cortada, más un consumidor
/// que la declara. Devuelve `(tmp, proveedor, consumidor, uuid)`.
fn provider_and_consumer() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let tmp = tempfile::tempdir().unwrap();
    let provider = tmp.path().join("hsi");
    let consumer = tmp.path().join("retinar");

    // ── el proveedor publica ────────────────────────────────────────────────
    fs::create_dir_all(provider.join("src")).unwrap();
    fs::write(provider.join("src/Perm.java"),
              "public class Perm {\n    public boolean can(String op) { return true; }\n}\n").unwrap();
    git_init(&provider);
    git_commit_all(&provider, "init");

    let (stdout, stderr, ok) = run_in(&provider, &[
        "chain", "new", "--tip", "src/Perm.java:2:5", "--tip", "abstract",
    ]);
    assert!(ok, "publicar una abstracción falló:\n{stderr}");
    let uuid = stdout.lines()
        .find_map(|l| l.strip_prefix("Created chain: "))
        .expect("uuid").trim().to_string();

    run_in(&provider, &["check", "."]);
    run_in(&provider, &["accept", "."]);
    git_commit_all(&provider, "el bilink abstracto");
    corte(&provider);

    // ── el consumidor declara ───────────────────────────────────────────────
    fs::create_dir_all(consumer.join("src")).unwrap();
    fs::write(consumer.join("src/permissions.ts"),
              "export function can(op: string) { return true; }\n").unwrap();
    git_init(&consumer);
    git_commit_all(&consumer, "init");

    // El alias: **el único lugar del consumidor que sabe algo del otro repo.**
    fs::create_dir_all(consumer.join(".bilink")).unwrap();
    fs::write(consumer.join(".bilink/.hsi.toml"),
              format!("remote = \"{}\"\nbranch = \"{}\"\n",
                      provider.display(), branch_of(&provider))).unwrap();

    (tmp, provider, consumer, uuid)
}

/// Enlaza el consumidor con la abstracción, y trae el repo del proveedor.
fn consume(consumer: &Path, uuid: &str) {
    let (_, stderr, ok) = run_in(consumer, &[
        "chain", "new", "--from-repo", &format!("hsi:{uuid}"),
        "--tip", "src/permissions.ts:1:1",
    ]);
    assert!(ok, "consumir la abstracción falló:\n{stderr}");

    let (_, stderr, ok) = run_in(consumer, &["fetch", "hsi"]);
    assert!(ok, "fetch falló:\n{stderr}");
}

/// `abstract-endpoint-is-open` + `provider-detects-own-drift`.
#[test]
fn the_abstract_endpoint_is_open_and_the_provider_still_sees_its_own_drift() {
    let (_t, provider, _c, _uuid) = provider_and_consumer();

    let (out, _, ok) = run_in(&provider, &["check", "."]);
    assert!(ok, "el proveedor arranca limpio:\n{out}");

    // Lo publicado deja de coincidir con lo aprobado. Es una pregunta puramente
    // local, y es la razón por la que al proveedor no le alcanzan los captures.
    fs::write(provider.join("src/Perm.java"),
              "public class Perm {\n    public boolean can(String op) { return check(op); }\n}\n").unwrap();
    commit(&provider, "el fragmento publicado cambia");

    let states = check_states(&provider);
    assert!(states.contains("ALTERED"), "el proveedor ve su propio drift:\n{states}");
    assert!(states.contains("OPEN"), "y la punta abierta sigue OPEN:\n{states}");
}

/// `accept-bulk-skips-open` — `accept .` nunca toca la punta abstracta.
#[test]
fn accept_bulk_never_touches_the_abstract_endpoint() {
    let (_t, provider, _c, uuid) = provider_and_consumer();

    fs::write(provider.join("src/Perm.java"),
              "public class Perm {\n    public boolean can(String op) { return check(op); }\n}\n").unwrap();
    commit(&provider, "el fragmento cambia");
    run_in(&provider, &["check", "."]);
    run_in(&provider, &["accept", "."]);

    let bl = fs::read_to_string(provider.join(format!(".bilink/{uuid}.yaml"))).unwrap();
    let after_abstract = bl.split("link: abstract").nth(1).unwrap_or("");
    assert!(!after_abstract.contains("accepted"),
            "la punta abierta no lleva `accepted`: no hay nada que bendecir ahí:\n{bl}");
}

/// `remote-unreachable-is-not-an-error` — y `check` **no hace red**.
#[test]
fn an_uncloned_provider_is_reported_and_does_not_break_the_check() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();

    let (_, stderr, ok) = run_in(&consumer, &[
        "chain", "new", "--from-repo", &format!("hsi:{uuid}"),
        "--tip", "src/permissions.ts:1:1",
    ]);
    assert!(ok, "chain new falló:\n{stderr}");

    let states = check_states(&consumer);
    assert!(states.contains("REMOTE_UNREACHABLE"),
            "el repo ajeno no está clonado, y eso se reporta:\n{states}");
    assert!(!consumer.join(".bilink/hsi").exists(),
            "check no clona: es masivo y no puede hacer red como efecto colateral");
}

/// `remote-ok-when-accepted-pair-unchanged` + `consumer-stores-nothing-about-provider`.
#[test]
fn the_consumer_stores_two_opaque_hashes_and_an_alias() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);

    run_in(&consumer, &["check", "."]);
    let (_, stderr, ok) = run_in(&consumer, &["accept", "."]);
    assert!(ok, "accept falló:\n{stderr}");
    assert!(check_states(&consumer).trim().is_empty(), "y queda OK");

    let bl = fs::read_to_string(consumer.join(format!(".bilink/{uuid}.yaml"))).unwrap();
    assert!(bl.contains("link: repo hsi"), "el endpoint nombra el alias:\n{bl}");
    for leak in ["http", "git@", "/tmp", ".java"] {
        assert!(!bl.contains(leak),
                "el bilink no contiene nada del proveedor salvo el alias ({leak}):\n{bl}");
    }
}

/// `remote-drift-after-provider-accepts` — y que `check` no lo vea hasta el fetch.
#[test]
fn the_consumer_sees_drift_only_after_bringing_the_provider() {
    let (_t, provider, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);
    run_in(&consumer, &["check", "."]);
    run_in(&consumer, &["accept", "."]);

    // El proveedor cambia lo publicado y lo acepta.
    fs::write(provider.join("src/Perm.java"),
              "public class Perm {\n    public boolean can(String op) { return check(op); }\n}\n").unwrap();
    commit(&provider, "el fragmento cambia");
    run_in(&provider, &["check", "."]);
    let (_, stderr, ok) = run_in(&provider, &["accept", "."]);
    assert!(ok, "accept del proveedor falló:\n{stderr}");

    // Sin traer nada, el consumidor sigue viendo lo que trajo la última vez.
    assert!(check_states(&consumer).trim().is_empty(),
            "check no hace red: lo que no se trajo, no se ve");

    let (_, stderr, ok) = run_in(&consumer, &["fetch", "hsi"]);
    assert!(ok, "fetch falló:\n{stderr}");
    let states = check_states(&consumer);
    assert!(states.contains("CHAIN_DIRTY"),
            "tras traerlo, el drift del proveedor se ve:\n{states}");
}

/// `rejected-when-remote-stops-being-abstract`.
///
/// Es un hecho distinto de "el fragmento cambió", y por eso no comparte token.
#[test]
fn the_link_is_rejected_when_the_other_end_stops_being_abstract() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);
    run_in(&consumer, &["check", "."]);
    run_in(&consumer, &["accept", "."]);

    let remote = consumer.join(format!(".bilink/hsi/.bilink/{uuid}.yaml"));
    let text = fs::read_to_string(&remote).unwrap();
    fs::write(&remote, text.replace("link: abstract", "link: issue 1")).unwrap();

    let states = check_states(&consumer);
    assert!(states.contains("REJECTED"),
            "la otra punta ya no admite ser ampliada:\n{states}");

    // Y aceptar se niega: fijaría el vínculo contra algo que dejó de sostenerlo.
    let (_, stderr, ok) = run_in(&consumer, &["accept", &format!("{}.0", &uuid[..8])]);
    assert!(!ok, "aceptar un REJECTED tiene que fallar");
    assert!(stderr.contains("abstract"), "y decir por qué:\n{stderr}");
}

/// `broken-when-remote-bilink-gone` — distinguirlo de `REMOTE_UNREACHABLE` es la
/// razón del desdoblamiento: uno se arregla trayendo, el otro investigando.
#[test]
fn a_removed_remote_bilink_is_broken_and_not_unreachable() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);
    run_in(&consumer, &["check", "."]);
    run_in(&consumer, &["accept", "."]);

    fs::remove_file(consumer.join(format!(".bilink/hsi/.bilink/{uuid}.yaml"))).unwrap();

    let states = check_states(&consumer);
    assert!(states.contains("BROKEN"), "el clon está y el bilink no:\n{states}");
    assert!(!states.contains("UNREACHABLE"),
            "no es una ausencia que se arregle trayendo algo:\n{states}");
}

/// `sparse-set-is-derived-and-incremental` — el conjunto sale de los bilinks.
#[test]
fn the_sparse_set_is_derived_from_the_bilinks() {
    let (_t, provider, consumer, uuid) = provider_and_consumer();

    // Un archivo del proveedor que **ningún** bilink referencia.
    fs::write(provider.join("src/Other.java"), "public class Other {}\n").unwrap();
    commit(&provider, "otro archivo, que nadie consume");
    run_in(&provider, &["sync"]);

    consume(&consumer, &uuid);

    let clone = consumer.join(".bilink/hsi");
    assert!(clone.join("src/Perm.java").exists(),
            "el archivo del fragmento referenciado sí se trae");
    assert!(!clone.join("src/Other.java").exists(),
            "y el que nadie referencia no: el conjunto se calcula, no se declara");
    assert!(clone.join(".bilink").is_dir(), "más los .bilink, que son el paso previo");
}

/// `remote-fan-out-is-independent` — el proveedor tiene **un** archivo, y no cambia.
#[test]
fn two_consumers_share_one_provider_file_that_never_changes() {
    let (tmp, provider, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);

    let before = rev(&provider, &format!("refs/bilink/{}", branch_of(&provider)));

    // Un segundo consumidor, en otro repo, sobre la misma abstracción.
    let otro = tmp.path().join("otro");
    fs::create_dir_all(otro.join("src")).unwrap();
    fs::write(otro.join("src/perm.ts"), "export function can(op: string) { return true; }\n").unwrap();
    git_init(&otro);
    git_commit_all(&otro, "init");
    fs::create_dir_all(otro.join(".bilink")).unwrap();
    fs::write(otro.join(".bilink/.hsi.toml"),
              format!("remote = \"{}\"\nbranch = \"{}\"\n",
                      provider.display(), branch_of(&provider))).unwrap();

    let (_, stderr, ok) = run_in(&otro, &[
        "chain", "new", "--from-repo", &format!("hsi:{uuid}"), "--tip", "src/perm.ts:1:1",
    ]);
    assert!(ok, "el segundo consumidor falló:\n{stderr}");
    run_in(&otro, &["fetch", "hsi"]);
    run_in(&otro, &["check", "."]);
    run_in(&otro, &["accept", "."]);
    assert!(check_states(&otro).trim().is_empty(), "y queda OK");

    // El uuid es el mismo de los dos lados: **es el rendezvous**.
    assert!(otro.join(format!(".bilink/{uuid}.yaml")).exists());
    assert!(consumer.join(format!(".bilink/{uuid}.yaml")).exists());

    // Y el proveedor no se enteró de ninguno de los dos.
    assert_eq!(before, rev(&provider, &format!("refs/bilink/{}", branch_of(&provider))),
               "sumar un consumidor no toca el repo del proveedor");
    let files = fs::read_dir(provider.join(".bilink")).unwrap()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("yaml"))
        .count();
    assert_eq!(files, 1, "un fragmento publicado es UN archivo, con C consumidores");
    let mentions = std::process::Command::new("grep")
        .args(["-rl", "retinar", "."])
        .current_dir(provider.join(".bilink"))
        .output().unwrap();
    assert!(mentions.stdout.is_empty(),
            "el proveedor no nombra a ningún consumidor: {}",
            String::from_utf8_lossy(&mentions.stdout));
}

/// La verificación de versión: **el consumidor se niega en vez de malinterpretar.**
#[test]
fn the_consumer_refuses_a_provider_format_it_does_not_understand() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);
    run_in(&consumer, &["check", "."]);
    run_in(&consumer, &["accept", "."]);
    assert!(check_states(&consumer).trim().is_empty(), "arranca limpio");

    let version = consumer.join(".bilink/hsi/.bilink/version");
    fs::write(&version, "4.0.0\n").unwrap();

    let (out, stderr, _) = run_in(&consumer, &["check", "."]);
    assert!(stderr.contains("4.0.0") || out.contains("4.0.0"),
            "se dice qué versión publica y cuál se lee:\n{out}\n{stderr}");
    assert!(!out.contains("OK") && !out.contains("ALTERED"),
            "no se reporta ningún estado sobre archivos que no se entendieron:\n{out}");

    // Un minor distinto sí se entiende: lo aditivo no rompe al que lee más nuevo.
    fs::write(&version, "3.0.0\n").unwrap();
    let (_, _, ok) = run_in(&consumer, &["check", "."]);
    assert!(ok, "un minor distinto del mismo major se lee igual");
}

/// `frontier-needs-no-migration` — los dos tipos son aditivos.
#[test]
fn the_frontier_is_additive_and_needs_no_migration() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);

    let ledger = consumer.join(".accreta/migrations");
    let before = fs::read_to_string(&ledger).unwrap_or_default();

    run_in(&consumer, &["check", "."]);
    run_in(&consumer, &["accept", "."]);

    assert_eq!(before, fs::read_to_string(&ledger).unwrap_or_default(),
               "ningún archivo existente usa los tipos nuevos: no hay qué migrar");
    let _ = uuid;
}

/// El clon de un proveedor **no entra al commit de la ref**, y no por suerte.
///
/// Vive en `.bilink/<alias>/` y es otro repo entero: se trae, se descarta y se
/// vuelve a traer, y su procedencia es su propio remoto. Lo que lo deja afuera es la
/// enumeración que construye el árbol —la misma frontera de repo que frena el
/// recorrido de capas— y **no una línea en ningún `.gitignore`**: la exclusión del
/// lado del proyecto ya la puso `init` en `.git/info/exclude`, una vez y por clon.
#[test]
fn a_provider_clone_never_enters_the_ref() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);
    run_in(&consumer, &["check", "."]);
    run_in(&consumer, &["accept", "."]);

    // El consumidor corta a la ref con el clon del proveedor ya en el árbol.
    git_commit_all(&consumer, "los bilinks, todavía en la rama");
    let (out, err, ok) = corte(&consumer);
    assert!(ok, "el corte falló:\n{out}\n{err}");

    let bref = format!("refs/bilink/{}", branch_of(&consumer));
    let files = git_out(&consumer, &["ls-tree", "-r", "--name-only", &bref]);

    assert!(!files.lines().any(|f| f.starts_with(".bilink/hsi/")),
            "el clon del proveedor no es contenido de esta capa:\n{files}");
    assert!(files.contains(".bilink/.hsi.toml"),
            "pero su declaración sí: es de quien la escribe:\n{files}");
    assert!(files.contains(&format!(".bilink/{uuid}.yaml")),
            "y los bilinks propios también:\n{files}");

    // Y el clon sigue en el árbol de trabajo, donde `check` lo necesita.
    assert!(consumer.join(".bilink/hsi/.bilink").is_dir(),
            "no se commitea, pero no se borra");
}

/// `bilinker fetch` **no toca ningún `.gitignore`.**
///
/// La exclusión de `.bilink/` del lado del proyecto la puso `init` una sola vez, y
/// la del commit de la ref es la enumeración. Escribir el alias en un archivo
/// versionado sería una escritura de contenido para resolver algo que es del índice.
#[test]
fn fetch_writes_no_ignore_rules() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();

    consume(&consumer, &uuid);
    let rule = fs::read_to_string(consumer.join(".bilink/.gitignore")).unwrap_or_default();

    // La regla que sí está es la de siempre —`cache/` e `index/`— y la escribió
    // quien creó la capa, no el fetch.
    assert!(!rule.contains("hsi"), "el alias no aparece en ninguna regla:\n{rule}");

    // Ni en el `.gitignore` del proyecto, que nadie toca nunca.
    assert!(!consumer.join(".gitignore").exists()
            || !fs::read_to_string(consumer.join(".gitignore")).unwrap().contains("hsi"),
            "el .gitignore del proyecto está versionado: tocarlo cambiaría su rama");

    // Y la exclusión que de verdad rige del lado del proyecto la pone `init`, una
    // sola vez y por clon — no `fetch`, y no una regla por alias.
    let (_, stderr, ok) = run_in(&consumer, &["init"]);
    assert!(ok, "init falló:\n{stderr}");
    let exclude = fs::read_to_string(consumer.join(".git/info/exclude")).unwrap_or_default();
    assert!(exclude.contains(".bilink/"),
            "un solo patrón cubre el directorio entero, clones incluidos:\n{exclude}");
}

/// `diff-deepens-on-demand` — ver qué cambió del lado del proveedor.
///
/// El clon arranca superficial, así que el commit donde vivía lo aceptado no está
/// en él: se trae acá, para **un** bilink, recorriendo la ref del proveedor hacia
/// atrás. Es el reparto que la frontera define: `check` es masivo y barato; ver el
/// diff es puntual y caro.
#[test]
fn get_diff_crosses_the_frontier_and_deepens_the_clone() {
    let (_t, provider, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);
    run_in(&consumer, &["check", "."]);
    run_in(&consumer, &["accept", "."]);

    fs::write(provider.join("src/Perm.java"),
              "public class Perm {\n    public boolean can(String op) { return check(op); }\n}\n").unwrap();
    commit(&provider, "el fragmento publicado cambia");
    run_in(&provider, &["check", "."]);
    run_in(&provider, &["accept", "."]);
    run_in(&consumer, &["fetch", "hsi"]);

    let (out, stderr, ok) = run_in(&consumer, &["get", &format!("{}.0", &uuid[..8]), "--diff"]);
    assert!(ok, "el diff cruzando la frontera falló:\n{out}\n{stderr}");
    assert!(out.contains("return true"), "el texto aceptado, de la historia del proveedor:\n{out}");
    assert!(out.contains("return check(op)"), "y el que publica ahora:\n{out}");
}

/// El diff de un endpoint repo **no se le pregunta a la historia local**.
///
/// El commit del contenido aceptado vive del lado del proveedor. Pedírselo a este
/// repo es preguntarle por algo que nunca tuvo, y la respuesta —"no aparece en los
/// últimos commits del archivo"— sería cierta sobre la pregunta equivocada.
#[test]
fn a_repo_endpoint_diff_does_not_look_in_the_local_history() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);
    run_in(&consumer, &["check", "."]);
    run_in(&consumer, &["accept", "."]);

    // Con la cache borrada, un endpoint local caería al walk local. Éste no.
    let _ = fs::remove_dir_all(consumer.join(".bilink/cache"));

    let (out, stderr, ok) = run_in(&consumer, &["get", &format!("{}.0", &uuid[..8]), "--diff"]);
    assert!(ok, "el diff tiene que salir del clon, no de esta historia:\n{out}\n{stderr}");
    assert!(!stderr.contains("últimos commits del archivo"),
            "ése es el error de buscar en el repo equivocado:\n{stderr}");
}

/// **Ninguna interacción con `refs/bilink/*` se hace tipeando git.**
///
/// La ref vive fuera de `refs/heads/`, así que `git push` a secas no la empuja y
/// hay que nombrarla con un refspec. Hacer que alguien lo tipee es una fuga del
/// namespace hacia afuera: a la segunda vez ya es una convención que alguien copia
/// mal. El refspec lo arma `bilinker push`.
#[test]
fn push_publishes_the_ref_without_anyone_typing_a_refspec() {
    let (_t, root, _uuid, _x) = cut_over();
    let branch = branch_of(&root);
    let bref = format!("refs/bilink/{branch}");

    // Un remoto de verdad, para que el push tenga a dónde ir.
    let bare = root.parent().unwrap().join("remoto.git");
    std::process::Command::new("git")
        .args(["init", "--bare", "-q"]).arg(&bare).output().unwrap();
    git(&root, &["remote", "add", "origin", &bare.to_string_lossy()]);

    let (out, stderr, ok) = run_in(&root, &["push"]);
    assert!(ok, "push falló:\n{out}\n{stderr}");
    assert!(out.contains("publicado"), "y lo dice:\n{out}");

    let there = git_out(&bare, &["rev-parse", &bref]);
    assert_eq!(there.trim(), rev(&root, &bref), "la ref quedó en el remoto");

    // Y no publicó la rama del proyecto: eso es decisión de quien trabaja.
    assert!(std::process::Command::new("git").current_dir(&bare)
                .args(["rev-parse", "--verify", &format!("refs/heads/{branch}")])
                .output().unwrap().stdout.is_empty(),
            "push publica la ref, no la rama");
}

/// Publicar dos veces no es un error: la segunda no mueve nada y lo dice.
#[test]
fn push_is_idempotent() {
    let (_t, root, _uuid, _x) = cut_over();
    let bare = root.parent().unwrap().join("remoto2.git");
    std::process::Command::new("git")
        .args(["init", "--bare", "-q"]).arg(&bare).output().unwrap();
    git(&root, &["remote", "add", "origin", &bare.to_string_lossy()]);

    run_in(&root, &["push"]);
    let (out, _, ok) = run_in(&root, &["push"]);
    assert!(ok, "el segundo push no es un error");
    assert!(out.contains("ya estaba"), "y dice que no movió nada:\n{out}");
}

/// `sync` alinea y **no publica**: son dos actos, y quien trabaja en una rama propia
/// hace el primero muchas veces antes del segundo.
#[test]
fn sync_does_not_publish() {
    let (_t, root, _uuid, _x) = cut_over();
    let bare = root.parent().unwrap().join("remoto3.git");
    std::process::Command::new("git")
        .args(["init", "--bare", "-q"]).arg(&bare).output().unwrap();
    git(&root, &["remote", "add", "origin", &bare.to_string_lossy()]);

    fs::write(root.join("src/Other.java"), "public class Other {}\n").unwrap();
    commit(&root, "el proyecto avanza");
    let (_, stderr, ok) = run_in(&root, &["sync"]);
    assert!(ok, "sync falló:\n{stderr}");

    let remote_has = std::process::Command::new("git").current_dir(&bare)
        .args(["rev-parse", "--verify", &format!("refs/bilink/{}", branch_of(&root))])
        .output().unwrap();
    assert!(!remote_has.status.success(), "sync no habla con la red");
}

/// El catálogo: **para colgarse de algo hay que poder ver de qué.**
///
/// Sin esto la lista de abstracciones de un proveedor viaja por chat, y elegir es
/// elegir entre uuids.
#[test]
fn the_catalog_shows_what_a_provider_publishes_with_its_code() {
    let (_t, provider, consumer, uuid) = provider_and_consumer();

    // Una segunda abstracción, que nadie consume.
    fs::write(provider.join("src/Turnos.java"),
              "public class Turnos {\n    public String reservar(String p) { return p; }\n}\n").unwrap();
    commit(&provider, "otro fragmento publicable");
    let (out, stderr, ok) = run_in(&provider, &[
        "chain", "new", "--tip", "src/Turnos.java:2:5", "--tip", "abstract"]);
    assert!(ok, "publicar la segunda falló:\n{stderr}");
    let otro = out.lines().find_map(|l| l.strip_prefix("Created chain: "))
        .expect("uuid").trim().to_string();
    run_in(&provider, &["check", "."]);
    run_in(&provider, &["accept", "."]);

    fs::create_dir_all(consumer.join(".bilink")).unwrap();
    let (_, stderr, ok) = run_in(&consumer, &["fetch", "hsi"]);
    assert!(ok, "fetch falló:\n{stderr}");

    let (out, stderr, ok) = run_in(&consumer, &["abstracts", "hsi"]);
    assert!(ok, "el catálogo falló:\n{out}\n{stderr}");

    // Las dos, con su código — que es lo que hace posible elegir.
    assert!(out.contains("Perm.java") && out.contains("Turnos.java"),
            "lista las dos:\n{out}");
    assert!(out.contains("public boolean can"), "con el código de una:\n{out}");
    assert!(out.contains("reservar"), "y el de la otra:\n{out}");
    assert!(out.contains(&uuid[..8]) && out.contains(&otro[..8]),
            "y el uuid con que colgarse de cada una:\n{out}");
}

/// **Mirar el catálogo no trae nada ni amplía el sparse.**
///
/// El clon recorta el árbol de trabajo, no el object store: los blobs del commit
/// traído están todos, así que el fragmento se lee con `git show` aunque el archivo
/// no esté en disco. El conjunto sparse sigue siendo derivado de lo que se consume,
/// no de lo que se miró.
#[test]
fn browsing_the_catalog_does_not_widen_the_sparse_set() {
    let (_t, provider, consumer, _uuid) = provider_and_consumer();

    fs::write(provider.join("src/Turnos.java"),
              "public class Turnos {\n    public String reservar(String p) { return p; }\n}\n").unwrap();
    commit(&provider, "otro fragmento publicable");
    run_in(&provider, &["chain", "new", "--tip", "src/Turnos.java:2:5", "--tip", "abstract"]);
    run_in(&provider, &["check", "."]);
    run_in(&provider, &["accept", "."]);

    fs::create_dir_all(consumer.join(".bilink")).unwrap();
    run_in(&consumer, &["fetch", "hsi"]);

    // Sin consumir nada, el árbol del clon no tiene ningún archivo de código.
    let en_arbol = |p: &Path| p.join(".bilink/hsi/src").read_dir()
        .map(|d| d.flatten().count()).unwrap_or(0);
    assert_eq!(en_arbol(&consumer), 0, "todavía no se consume nada");

    let (out, _, ok) = run_in(&consumer, &["abstracts", "hsi"]);
    assert!(ok, "el catálogo falló:\n{out}");
    assert!(out.contains("reservar"), "y sin embargo muestra el código:\n{out}");
    assert_eq!(en_arbol(&consumer), 0, "mirarlo no sacó ningún archivo al árbol");
}

/// El proveedor pregunta lo mismo desde su lado: *¿qué estoy publicando?*
#[test]
fn a_provider_can_list_what_it_publishes() {
    let (_t, provider, _c, uuid) = provider_and_consumer();

    let (out, stderr, ok) = run_in(&provider, &["abstracts"]);
    assert!(ok, "listar lo propio falló:\n{out}\n{stderr}");
    assert!(out.contains(&uuid[..8]), "con su uuid:\n{out}");
    assert!(out.contains("public boolean can"), "y su código, del árbol de trabajo:\n{out}");
    assert!(!out.contains("ya lo consumís"),
            "nadie consume lo propio: esa marca es sobre un proveedor ajeno:\n{out}");
}

/// Lo que ya se consume se marca, para no colgarse dos veces de lo mismo.
#[test]
fn the_catalog_marks_what_is_already_consumed() {
    let (_t, _p, consumer, uuid) = provider_and_consumer();
    consume(&consumer, &uuid);

    let (out, _, ok) = run_in(&consumer, &["abstracts", "hsi"]);
    assert!(ok);
    assert!(out.contains("ya lo consumís"), "se marca:\n{out}");
}
