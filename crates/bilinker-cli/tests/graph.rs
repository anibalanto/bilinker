//! `bilinker graph`: la exportación de aristas hacia lattice.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// El binario, con un PATH donde no hay `lspd`: la suite no le habla a un daemon real.
fn bilinker_cmd() -> Command {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::env::split_paths(&path)
        .filter(|d| !d.join(if cfg!(windows) { "lspd.exe" } else { "lspd" }).exists());
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bilinker"));
    cmd.env("PATH", std::env::join_paths(dirs).unwrap());
    cmd
}

fn code_in(root: &Path, args: &[&str]) -> (String, String, i32) {
    let out = bilinker_cmd().current_dir(root).args(args).output().expect("failed to run bilinker");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn git(root: &Path, args: &[&str]) {
    Command::new("git").current_dir(root).args(args).output().unwrap();
}

/// Un repo commiteado con una spec y un controller con anotaciones.
fn workspace() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/spec.md"), "# Spec\n\nSome spec content.\n").unwrap();
    fs::write(root.join("src/Service.java"), concat!(
        "@RestController\n",
        "@RequestMapping(\"/public-api/user\")\n",
        "public class Service {\n",
        "\n",
        "    public void run() {}\n",
        "\n",
        "    @GetMapping(\"/permissions/from-token\")\n",
        "    public List<PublicAuthorityDto> getPermissions(String token)\n",
        "    {\n",
        "        return svc.permissionsOf(token);\n",
        "    }\n",
        "}\n",
    )).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "init"],
    ] {
        git(&root, &args);
    }
    (tmp, root)
}

fn chain(root: &Path, extra: &[&str]) -> String {
    let mut args = vec!["chain", "new", "--yes", "--tip", "docs/spec.md:1:1"];
    args.extend_from_slice(extra);
    let (stdout, stderr, code) = code_in(root, &args);
    assert_eq!(code, 0, "{stderr}");
    let uuid = fs::read_dir(root.join(".bilink")).unwrap()
        .filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
        .find(|u| stdout.contains(&u[..8]) || stderr.contains(&u[..8]));
    uuid.unwrap_or_else(|| panic!("chain new no nombró el bilink:\n{stdout}\n{stderr}"))
}

fn edges(stdout: &str) -> Vec<serde_yaml_ng::Value> {
    serde_yaml_ng::from_str(stdout).unwrap_or_else(|e| panic!("no es JSON: {e}\n{stdout}"))
}

fn field<'a>(edge: &'a serde_yaml_ng::Value, key: &str) -> &'a str {
    edge.get(key).and_then(|v| v.as_str()).unwrap_or_else(|| panic!("falta `{key}`: {edge:?}"))
}

/// `archivo#inicio~fin`, con un solo rango.
fn range_of(node: &str) -> (usize, usize) {
    let (_, range) = node.rsplit_once('#').unwrap_or_else(|| panic!("sin rango: {node}"));
    let (a, b) = range.split_once('~').unwrap_or_else(|| panic!("no es inicio~fin: {node}"));
    (a.parse().unwrap_or_else(|_| panic!("inicio: {node}")), b.parse().unwrap_or_else(|_| panic!("fin: {node}")))
}

#[test]
fn graph_exports_the_yaml_bilinks_of_the_layer() {
    let (_tmp, root) = workspace();
    let uuid = chain(&root, &["--tip", "src/Service.java:5:17"]);
    code_in(&root, &["check", "."]);

    let (stdout, stderr, code) = code_in(&root, &["graph", ".", "--format", "json"]);
    assert_eq!(code, 0, "{stderr}");
    let edges = edges(&stdout);
    assert_eq!(edges.len(), 1, "una arista por cadena:\n{stdout}");
    let edge = &edges[0];
    assert_eq!(field(edge, "ref"), uuid);
    assert_eq!(field(edge, "kind"), "bilink");
    assert!(field(edge, "from").starts_with(".::docs/spec.md#"), "{stdout}");
    assert!(field(edge, "to").starts_with(".::src/Service.java#"), "{stdout}");
    range_of(field(edge, "to"));
}

#[test]
fn json_is_the_format_without_the_flag() {
    let (_tmp, root) = workspace();
    chain(&root, &["--tip", "src/Service.java:5:17"]);
    code_in(&root, &["check", "."]);

    let (with, _, _) = code_in(&root, &["graph", ".", "--format", "json"]);
    let (without, stderr, code) = code_in(&root, &["graph", "."]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(with, without);
}

#[test]
fn graph_has_no_tree_flat_or_depth() {
    let (_tmp, root) = workspace();
    chain(&root, &["--tip", "src/Service.java:5:17"]);
    code_in(&root, &["check", "."]);

    for args in [
        vec!["graph", ".", "--format", "tree"],
        vec!["graph", ".", "--format", "flat"],
        vec!["graph", ".", "--depth", "1"],
    ] {
        let (stdout, _, code) = code_in(&root, &args);
        assert_ne!(code, 0, "{args:?} no existe:\n{stdout}");
        assert!(stdout.is_empty(), "{args:?} no emite nada:\n{stdout}");
    }
}

#[test]
fn a_layer_without_bilinks_exits_with_one() {
    let (_tmp, root) = workspace();
    fs::create_dir_all(root.join(".bilink")).unwrap();

    let (_, stderr, code) = code_in(&root, &["graph", ".", "--format", "json"]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("no bilinks"), "{stderr}");
}

#[test]
fn bilinks_without_a_checked_range_exit_with_one_and_name_check() {
    let (_tmp, root) = workspace();
    chain(&root, &["--tip", "src/Service.java:5:17"]);
    let _ = fs::remove_dir_all(root.join(".bilink/cache"));

    let (stdout, stderr, code) = code_in(&root, &["graph", ".", "--format", "json"]);
    assert_eq!(code, 1, "sin rango no hay arista:\n{stdout}\n{stderr}");
    assert!(stderr.contains("bilinker check"), "{stderr}");
}

/// Los tramos de un nodo: `archivo#a~b,c~d`.
fn spans_of(node: &str) -> Vec<(usize, usize)> {
    let (_, ranges) = node.rsplit_once('#').unwrap_or_else(|| panic!("sin rango: {node}"));
    ranges.split(',').map(|r| {
        let (a, b) = r.split_once('~').unwrap_or_else(|| panic!("no es inicio~fin: {node}"));
        (a.parse().unwrap(), b.parse().unwrap())
    }).collect()
}

/// La query de un endpoint de cuatro partes, como las que se escribían cuando la
/// query componía el fragmento. Ya no la escribe ningún comando, y se sigue leyendo.
const FOUR_PARTS: &str = r#"(class_declaration
  (modifiers
    (_
          name: (identifier) @n0 (#match? @n0 "^(RequestMapping)$")) @target)
  body: (class_body
    (method_declaration
      (modifiers
        (_
          name: (identifier) @n1 (#match? @n1 "^(GetMapping|PostMapping|PutMapping|DeleteMapping|PatchMapping|RequestMapping)$")) @target)
      type: (_) @target
      name: (identifier) @n2 (#eq? @n2 "getPermissions")
      parameters: (_) @target)))"#;

/// Una cadena cuyo tip de código es un capture de cuatro partes.
fn four_part_chain(root: &Path) -> String {
    let uuid = chain(root, &["--tip", "src/Service.java:8:37"]);
    let cap = bilink_format::Capture { file: "src/Service.java".into(), query: Some(FOUR_PARTS.into()) };
    let (id, _, _) = cap.write_in(root).unwrap();
    let path = root.join(format!(".bilink/{uuid}.yaml"));
    let mut bl = bilink_format::BiLink::load(&path).unwrap();
    bl.endpoint.get_mut(1).link = format!("capture {id}").parse().unwrap();
    bl.write(&path).unwrap();
    uuid
}

fn spring_controller_graph(root: &Path) -> serde_yaml_ng::Value {
    four_part_chain(root);
    code_in(root, &["check", "."]);
    let (stdout, stderr, code) = code_in(root, &["graph", ".", "--format", "json"]);
    assert_eq!(code, 0, "{stderr}");
    edges(&stdout).remove(0)
}

#[test]
fn a_range_of_several_parts_has_one_span_per_part() {
    let (_tmp, root) = workspace();
    let edge = spring_controller_graph(&root);
    let to = field(&edge, "to");
    let spans = spans_of(to);
    assert_eq!(spans.len(), 4, "la ruta de la clase, la del método, el retorno y los parámetros: {to}");

    let src = fs::read_to_string(root.join("src/Service.java")).unwrap();
    assert_eq!(spans[0].0, src.find("@RequestMapping").unwrap(), "arranca en la primera parte: {to}");
    let name = src.find("getPermissions").unwrap();
    assert!(spans.iter().all(|&(a, b)| !(a <= name && name < b)), "el nombre queda entre dos tramos: {to}");
    let body = src.find("return svc").unwrap();
    assert!(spans.iter().all(|&(_, b)| b <= body), "sin el cuerpo: {to}");
}

#[test]
fn a_tip_of_several_parts_carries_the_declaration_of_its_anchor() {
    let (_tmp, root) = workspace();
    let edge = spring_controller_graph(&root);
    let decl = edge.get("declaration").and_then(|d| d.as_sequence())
        .unwrap_or_else(|| panic!("falta `declaration`: {edge:?}"));
    assert!(decl[0].is_null(), "la spec es de una sola parte: {edge:?}");

    let src = fs::read_to_string(root.join("src/Service.java")).unwrap();
    let (start, end) = range_of(&format!("#{}", decl[1].as_str().unwrap()));
    assert_eq!(start, src.find("@GetMapping").unwrap(), "el método, desde sus anotaciones");
    assert_eq!(end, src.find("    }\n}").unwrap() + 5, "hasta el final de su cuerpo");
}

#[test]
fn a_tip_of_one_part_has_no_declaration() {
    let (_tmp, root) = workspace();
    chain(&root, &["--tip", "src/Service.java:5:17"]);
    code_in(&root, &["check", "."]);

    let (stdout, stderr, code) = code_in(&root, &["graph", ".", "--format", "json"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(edges(&stdout)[0].get("declaration").is_none(), "{stdout}");
}

#[test]
fn a_declaration_is_not_emitted_against_stale_spans() {
    let (_tmp, root) = workspace();
    four_part_chain(&root);
    code_in(&root, &["check", "."]);
    let src = fs::read_to_string(root.join("src/Service.java")).unwrap();
    fs::write(root.join("src/Service.java"), format!("// movido\n{src}")).unwrap();

    let (stdout, stderr, code) = code_in(&root, &["graph", ".", "--format", "json"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(edges(&stdout)[0].get("declaration").is_none(),
            "los tramos son de la cache, y la declaración sería de hoy:\n{stdout}");
}
