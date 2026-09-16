//! Un bilink sin rango en la cache: `graph` y `get` lo dicen, no lo omiten en silencio.

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

fn uuids(root: &Path) -> Vec<String> {
    fs::read_dir(root.join(".bilink")).unwrap()
        .filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
        .collect()
}

/// Dos bilinks sobre `src/lib.rs`: `uno`, con `check` corrido, y `dos`, creado después.
///
/// Devuelve los uuid en ese orden.
fn one_checked_and_one_new() -> (tempfile::TempDir, PathBuf, String, String) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/spec.md"), "# Spec\n\n## Uno\n\nuno\n\n## Dos\n\ndos\n").unwrap();
    fs::write(root.join("src/lib.rs"), "fn uno() {}\n\nfn dos() {}\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "t"],
        vec!["add", "-A"],
        vec!["commit", "-qm", "init"],
    ] {
        git(&root, &args);
    }

    let (_, stderr, code) = code_in(&root, &["chain", "new", "--yes", "--tip", "docs/spec.md:3:1", "--tip", "src/lib.rs:1:4"]);
    assert_eq!(code, 0, "{stderr}");
    let checked = uuids(&root).remove(0);
    code_in(&root, &["check", "."]);

    let (_, stderr, code) = code_in(&root, &["chain", "new", "--yes", "--tip", "docs/spec.md:7:1", "--tip", "src/lib.rs:3:4"]);
    assert_eq!(code, 0, "{stderr}");
    let new = uuids(&root).into_iter().find(|u| *u != checked).unwrap();
    (tmp, root, checked, new)
}

#[test]
fn graph_emits_what_it_has_and_exits_with_three() {
    let (_tmp, root, checked, new) = one_checked_and_one_new();

    let (stdout, stderr, code) = code_in(&root, &["graph", ".", "--format", "json"]);
    assert_eq!(code, 3, "incompleto no es completo:\n{stdout}\n{stderr}");
    assert!(stdout.contains(&checked), "la arista que tiene rango sale:\n{stdout}");
    assert!(!stdout.contains(&new), "la que no tiene rango no:\n{stdout}");
    assert!(stderr.contains("1 bilink"), "dice cuántos quedaron afuera:\n{stderr}");
    assert!(stderr.contains("bilinker check"), "{stderr}");
}

#[test]
fn graph_of_a_file_also_counts_the_bilinks_without_range() {
    let (_tmp, root, _, new) = one_checked_and_one_new();

    let (stdout, stderr, code) = code_in(&root, &["graph", "src/lib.rs", "--format", "json"]);
    assert_eq!(code, 3, "{stdout}\n{stderr}");
    assert!(!stdout.contains(&new), "{stdout}");
}

#[test]
fn graph_with_every_range_exits_with_zero() {
    let (_tmp, root, _, _) = one_checked_and_one_new();
    code_in(&root, &["check", "."]);

    let (stdout, stderr, code) = code_in(&root, &["graph", ".", "--format", "json"]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
}

#[test]
fn get_file_lists_an_endpoint_without_range() {
    let (_tmp, root, checked, new) = one_checked_and_one_new();

    let (stdout, stderr, code) = code_in(&root, &["get", "src/lib.rs"]);
    assert_eq!(code, 0, "{stderr}");
    let line_of = |uuid: &str| stdout.lines().find(|l| l.starts_with(uuid))
        .unwrap_or_else(|| panic!("falta {uuid}:\n{stdout}")).to_string();
    assert!(line_of(&checked).contains("bytes "), "{stdout}");
    assert!(line_of(&new).ends_with("sin rango"), "{stdout}");
    assert!(stderr.contains("bilinker check"), "{stderr}");
}

#[test]
fn get_file_with_every_range_says_nothing_on_stderr() {
    let (_tmp, root, _, _) = one_checked_and_one_new();
    code_in(&root, &["check", "."]);

    let (stdout, stderr, code) = code_in(&root, &["get", "src/lib.rs"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stdout.contains("sin rango"), "{stdout}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn get_position_warns_about_endpoints_without_range() {
    let (_tmp, root, _, _) = one_checked_and_one_new();

    let (stdout, stderr, code) = code_in(&root, &["get", "src/lib.rs:3:4"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.is_empty(), "sin rango no se puede afirmar que cubra:\n{stdout}");
    assert!(stderr.contains("1 endpoint"), "{stderr}");
    assert!(stderr.contains("bilinker check"), "{stderr}");
}
