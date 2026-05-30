use std::path::{Path, PathBuf};

/// Resolves a task ID to its absolute path and the project root.
///
/// Task files live at `<project-root>/.stratum/worklist/<id>.task`.
/// The project root is found by counting stratum depth from `layer_root`
/// (each stratum level = two path components: `.stratum/<name>`) and going
/// up that many component pairs.
pub fn resolve_task_path(layer_root: &Path, task_id: &str) -> (PathBuf, PathBuf) {
    let d = stratum::depth(layer_root);
    let canonical = layer_root.canonicalize().unwrap_or_else(|_| layer_root.to_path_buf());
    // Each stratum level = 2 path components; ancestors().nth(n) goes n levels up.
    let project_root = canonical.ancestors()
        .nth(d * 2)
        .unwrap_or(&canonical)
        .to_path_buf();
    let task_path = project_root
        .join(".stratum")
        .join("worklist")
        .join(format!("{task_id}.task"));
    (task_path, project_root)
}
