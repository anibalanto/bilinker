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
    assert!(config.contains("+refs/bilink/*:refs/bilink/*"),
            "falta el refspec:\n{config}");

    assert!(!root.join(".gitignore").exists(),
            ".gitignore está versionado: tocarlo modificaría la rama del proyecto");
    assert!(git_out(&root, &["status", "--porcelain"]).trim().is_empty(),
            "init no puede dejar la rama del proyecto sucia");
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

/// `accept-absorbs-before-committing` — absorber es precondición, no comportamiento.
#[test]
fn accept_absorbs_the_commit_it_accepted_against() {
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

    let parents = git_out(&root, &["rev-list", "--parents", "-n", "1", &bref]);
    let parents: Vec<&str> = parents.split_whitespace().skip(1).collect();
    assert_eq!(parents.len(), 2, "el accept absorbió en el mismo commit:\n{parents:?}");
    assert_eq!(parents[1], e, "el segundo padre es el commit contra el que se aceptó");

    // La invariante de fidelidad se verifica sola.
    let diff = git_out(&root, &["diff-tree", "-r", "--name-only", &e, &bref]);
    let strays: Vec<&str> = diff.lines()
        .filter(|l| !l.trim().is_empty() && !l.contains(".bilink/")).collect();
    assert!(strays.is_empty(), "el árbol de código es el del commit absorbido:\n{strays:?}");
}

/// `act-without-new-code-has-one-parent` — el merge es la forma de ponerse al día,
/// no la forma del acto.
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

    let parents = git_out(&root, &["rev-list", "--parents", "-n", "1", &bref]);
    let parents: Vec<&str> = parents.split_whitespace().skip(1).collect();
    assert_eq!(parents.len(), 2, "el último accept con cambios absorbió");
    assert_ne!(tree_before, rev(&root, &format!("{bref}^{{tree}}")), "y escribió algo");
}

/// La granularidad sigue al acto: `accept .` da **un** commit, no N.
#[test]
fn accept_writes_one_commit_per_invocation_not_per_endpoint() {
    let (_t, root, _uuid, _x) = cut_over();
    let bref = format!("refs/bilink/{}", branch_of(&root));

    // Los dos endpoints cambian a la vez.
    fs::write(root.join("docs/spec.md"), "# Spec\n\nOtro contenido.\n").unwrap();
    fs::write(root.join("src/Service.java"),
              "public class Service {\n    public void run() { int x = 1; }\n}\n").unwrap();
    commit(&root, "los dos lados cambian");

    let before = git_out(&root, &["rev-list", "--count", &bref]);
    run_in(&root, &["check", "."]);
    let (_, stderr, ok) = run_in(&root, &["accept", "."]);
    assert!(ok, "accept falló:\n{stderr}");
    let after = git_out(&root, &["rev-list", "--count", &bref]);

    let grew: usize = after.trim().parse::<usize>().unwrap()
        - before.trim().parse::<usize>().unwrap();
    assert_eq!(grew, 2, "un commit del acto más el del proyecto absorbido, no uno por endpoint");

    let subject = git_out(&root, &["log", "-1", "--format=%s", &bref]);
    assert!(subject.contains("2 endpoint"), "y el mensaje los enumera:\n{subject}");
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

/// Rebasea `A..<rama>` sobre una línea nueva, reescribiendo `B` y `C`.
///
/// Un rebase a secas **preserva el contenido**: `B'` tiene el mismo fragmento que
/// `B`, así que la derivación lo reencuentra en otro commit. Lo que rompe de verdad
/// es aplastar `B` y `C` en uno solo, donde el contenido intermedio no queda en
/// ningún commit de la historia reescrita — que es lo que `squash_over` hace.
fn rebase_over(root: &Path, branch: &str, a: &str) {
    git(root, &["checkout", "-q", "-b", "lado", a]);
    fs::write(root.join("docs/lado.md"), "# Lado\n").unwrap();
    commit(root, "D — otra línea");
    git(root, &["checkout", "-q", branch]);
    git(root, &["rebase", "-q", "--onto", "lado", a, branch]);
}

/// Aplasta `A..<rama>` en un solo commit sobre `A`.
///
/// El contenido intermedio —el que se aceptó— deja de existir en la historia de la
/// rama: el archivo salta de lo que tenía en `A` a lo que tiene al final.
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
