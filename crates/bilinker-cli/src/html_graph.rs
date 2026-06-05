use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use bilinker::link::{LinkEndpoint, ByteRange};
use bilinker::bilink::BiLinkFile;
use bilinker::scip_index::ScipIndex;
use bilinker::sciplink::{ScipLink, ScipLinkState, normalize_symbol_id};

// ─── helpers ──────────────────────────────────────────────────────────────────

pub fn esc_json(s: &str) -> String {
    s.replace('\\', "\\\\")
     .replace('"',  "\\\"")
     .replace('\n', "\\n")
     .replace('\r', "\\r")
     .replace('\t', "\\t")
}

pub fn layer_id(label: &str) -> String {
    format!("L_{}", label.replace(['/', '.', '-'], "_"))
}

pub fn layer_depth(label: &str) -> usize {
    if label == "." { 0 } else { label.matches(".stratum/").count() }
}

fn lang_from_file(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or("") {
        "rs"           => "rust",
        "md"           => "markdown",
        "toml"         => "toml",
        "java"         => "java",
        "ts" | "tsx"   => "typescript",
        "js" | "jsx"   => "javascript",
        "py"           => "python",
        "yaml" | "yml" => "yaml",
        "json"         => "json",
        "sql"          => "sql",
        "sh" | "bash"  => "bash",
        "html"         => "html",
        "css"          => "css",
        _              => "plaintext",
    }
}

// ─── data model ───────────────────────────────────────────────────────────────

pub struct ScipLinkData {
    pub symbol:       String,
    pub symbol_short: String,   // last segment for display
    pub file:         String,
    pub state:        String,   // "OK", "ALTERED", "DELETED", etc.
    pub ok:           bool,
}

pub struct HtmlNode {
    pub id:            String,
    pub file:          String,
    pub label:         String,
    pub layer:         String,
    pub abs_path:      String,
    pub content:       String,
    pub start_line:    usize,
    pub lang:          &'static str,
    pub subgraph_sym:  Option<String>,
    pub sciplinks:     Vec<ScipLinkData>,
    pub subgraph_ok:   bool,   // false if any sciplink is not OK
}

pub struct HtmlEdge {
    pub id:     String,
    pub source: String,
    pub target: String,
    pub label:  String,
    pub states: String,
    pub link0:  String,
    pub link1:  String,
}

pub struct ScipNode {
    pub id:         String,
    pub sym:        String,
    pub short:      String,
    pub file:       String,
    pub state:      String,
    pub ok:         bool,
    pub depth:      usize,
    pub parent_id:  String,
    pub content:    String,
    pub start_line: usize,
    pub lang:       &'static str,
}

pub struct HtmlGraph {
    layers:          BTreeMap<String, usize>,
    nodes:           Vec<HtmlNode>,
    edges:           Vec<HtmlEdge>,
    scip_nodes:      Vec<ScipNode>,
    /// All caller→callee edges (parent_id, child_id) — may have duplicates filtered by seen_scip_edges
    scip_edges:      Vec<(String, String)>,
    seen_nodes:      HashSet<String>,
    seen_edges:      HashSet<String>,
    seen_scip_nodes: HashSet<String>,
    seen_scip_edges: HashSet<(String, String)>,
    scip_cache:      HashMap<std::path::PathBuf, Option<ScipIndex>>,
    daemon_ok:       Option<bool>,
}

impl Default for HtmlGraph {
    fn default() -> Self {
        Self {
            layers: BTreeMap::new(), nodes: Vec::new(), edges: Vec::new(),
            scip_nodes: Vec::new(), scip_edges: Vec::new(),
            seen_nodes: HashSet::new(), seen_edges: HashSet::new(),
            seen_scip_nodes: HashSet::new(), seen_scip_edges: HashSet::new(),
            scip_cache: HashMap::new(),
            daemon_ok: None,
        }
    }
}

impl HtmlGraph {
    pub fn new() -> Self { Self::default() }

    pub fn add_node(&mut self, node: HtmlNode) {
        let depth = layer_depth(&node.layer);
        self.layers.insert(node.layer.clone(), depth);
        if self.seen_nodes.insert(node.id.clone()) {
            self.nodes.push(node);
        }
    }

    pub fn add_scip_node(&mut self, node: ScipNode) {
        // Record the edge regardless (each caller gets its own edge)
        let edge = (node.parent_id.clone(), node.id.clone());
        if self.seen_scip_edges.insert(edge.clone()) {
            self.scip_edges.push(edge);
        }
        // Add the node only once (deduplicated by symbol id)
        if self.seen_scip_nodes.insert(node.id.clone()) {
            self.scip_nodes.push(node);
        }
    }

    pub fn add_edge(&mut self, edge: HtmlEdge) {
        let key = if edge.source <= edge.target {
            format!("{}↔{}↔{}", edge.source, edge.target, edge.label)
        } else {
            format!("{}↔{}↔{}", edge.target, edge.source, edge.label)
        };
        if self.seen_edges.insert(key) { self.edges.push(edge); }
    }

    pub fn emit(&self) -> String {
        let layers_json = self.layers.iter().map(|(lbl, depth)| {
            format!(r#"{{"id":"{}","label":"{}","depth":{}}}"#,
                esc_json(&layer_id(lbl)), esc_json(lbl), depth)
        }).collect::<Vec<_>>().join(",");

        // Build file-group nodes: one per (file, layer) pair
        let fg_id = |file: &str, layer: &str| -> String {
            format!("fg_{}_{}", file.replace(['/', '.', '-'], "_"), layer.replace(['/', '.', '-'], "_"))
        };
        // Build file-groups in directory order using a sorted snapshot of nodes
        let mut nodes_for_fg: Vec<&HtmlNode> = self.nodes.iter().collect();
        nodes_for_fg.sort_by(|a, b| {
            let da = *self.layers.get(&a.layer).unwrap_or(&0);
            let db = *self.layers.get(&b.layer).unwrap_or(&0);
            da.cmp(&db)
              .then(a.layer.cmp(&b.layer))
              .then(a.file.cmp(&b.file))
        });
        let mut seen_fg: HashSet<String> = HashSet::new();
        let mut fg_parts: Vec<String> = vec![];
        for n in &nodes_for_fg {
            let gid = fg_id(&n.file, &n.layer);
            if seen_fg.insert(gid.clone()) {
                let fname = std::path::Path::new(&n.file)
                    .file_name().and_then(|s| s.to_str()).unwrap_or(&n.file);
                fg_parts.push(format!(
                    r#"{{"id":"{}","label":"{}","layer_id":"{}","layer":"{}","type":"file-group"}}"#,
                    esc_json(&gid), esc_json(fname),
                    esc_json(&layer_id(&n.layer)), esc_json(&n.layer)
                ));
            }
        }
        let file_groups_json = fg_parts.join(",");

        // Sort nodes: group by (depth, layer, dir, filename, start_line)
        // so nodes are ordered by folder within each column
        let file_dir  = |f: &str| std::path::Path::new(f).parent()
            .and_then(|p| p.to_str()).unwrap_or("").to_string();
        let file_name = |f: &str| std::path::Path::new(f).file_name()
            .and_then(|s| s.to_str()).unwrap_or(f).to_string();

        let mut ordered: Vec<&HtmlNode> = self.nodes.iter().collect();
        ordered.sort_by(|a, b| {
            let da = *self.layers.get(&a.layer).unwrap_or(&0);
            let db = *self.layers.get(&b.layer).unwrap_or(&0);
            da.cmp(&db)
              .then(a.layer.cmp(&b.layer))
              .then(file_dir(&a.file).cmp(&file_dir(&b.file)))
              .then(file_name(&a.file).cmp(&file_name(&b.file)))
              .then(a.start_line.cmp(&b.start_line))
        });

        // Assign y positions: add a gap of +1 row between different file-groups in the same column
        let mut depth_y: HashMap<usize, usize> = HashMap::new();
        let mut prev_fg: HashMap<usize, String> = HashMap::new();
        let nodes_json = ordered.iter().map(|n| {
            let depth = *self.layers.get(&n.layer).unwrap_or(&0);
            let gid   = fg_id(&n.file, &n.layer);
            let y = {
                let row = depth_y.entry(depth).or_insert(0);
                let prev = prev_fg.entry(depth).or_default();
                if !prev.is_empty() && *prev != gid { *row += 1; } // gap between file-groups
                *prev = gid.clone();
                let v = *row;
                *row += 1;
                v
            };
            let sciplinks_json = n.sciplinks.iter().map(|sl| {
                format!(r#"{{"sym":"{}","short":"{}","file":"{}","state":"{}","ok":{}}}"#,
                    esc_json(&sl.symbol), esc_json(&sl.symbol_short),
                    esc_json(&sl.file), esc_json(&sl.state), sl.ok)
            }).collect::<Vec<_>>().join(",");
            let subgraph_sym_json = n.subgraph_sym.as_deref()
                .map(|s| format!(r#""{}""#, esc_json(s)))
                .unwrap_or_else(|| "null".to_string());
            format!(
                r#"{{"id":"{}","label":"{}","file_group_id":"{}","layer_id":"{}","layer":"{}","abs_path":"{}","content":"{}","start_line":{},"lang":"{}","xi":{},"yi":{},"subgraph_sym":{},"subgraph_ok":{},"sciplinks":[{}]}}"#,
                esc_json(&n.id), esc_json(&n.label),
                esc_json(&gid), esc_json(&layer_id(&n.layer)), esc_json(&n.layer),
                esc_json(&n.abs_path), esc_json(&n.content),
                n.start_line, n.lang, depth, y,
                subgraph_sym_json, n.subgraph_ok, sciplinks_json
            )
        }).collect::<Vec<_>>().join(",");

        let edges_json = self.edges.iter().map(|e| {
            format!(
                r#"{{"id":"{}","source":"{}","target":"{}","label":"{}","states":"{}","link0":"{}","link1":"{}"}}"#,
                esc_json(&e.id), esc_json(&e.source), esc_json(&e.target),
                esc_json(&e.label), esc_json(&e.states),
                esc_json(&e.link0), esc_json(&e.link1)
            )
        }).collect::<Vec<_>>().join(",");

        let scip_nodes_json = self.scip_nodes.iter().map(|sn| {
            format!(r#"{{"id":"{}","sym":"{}","short":"{}","file":"{}","state":"{}","ok":{},"content":"{}","start_line":{},"lang":"{}"}}"#,
                esc_json(&sn.id), esc_json(&sn.sym), esc_json(&sn.short),
                esc_json(&sn.file), esc_json(&sn.state), sn.ok,
                esc_json(&sn.content), sn.start_line, sn.lang)
        }).collect::<Vec<_>>().join(",");

        let scip_edges_json = self.scip_edges.iter().map(|(src, tgt)| {
            format!(r#"{{"s":"{}","t":"{}"}}"#, esc_json(src), esc_json(tgt))
        }).collect::<Vec<_>>().join(",");

        let data = format!(r#"{{"layers":[{layers_json}],"file_groups":[{file_groups_json}],"nodes":[{nodes_json}],"edges":[{edges_json}],"scip_nodes":[{scip_nodes_json}],"scip_edges":[{scip_edges_json}]}}"#);
        TEMPLATE.replace("GRAPH_DATA_PLACEHOLDER", &data)
    }
}

// ─── traversal ────────────────────────────────────────────────────────────────

pub fn collect(
    root: &Path,
    bl: &BiLinkFile,
    layer_root: &Path,
    visited: &mut HashSet<String>,
    hg: &mut HtmlGraph,
    url_scheme: &str,
    depth: usize,
    max_depth: Option<usize>,
) -> anyhow::Result<()> {
    let uuid_short = &bl.uuid[..8.min(bl.uuid.len())];
    let s0 = bilinker::state_str(&bl.state0);
    let s1 = bilinker::state_str(&bl.state1);
    let lbl = crate::layer_label(root, layer_root);

    let local_id = add_structural(bl, layer_root, &lbl, hg, url_scheme);

    if max_depth.map_or(true, |d| depth < d) {
        for (adj_path, adj_layer) in crate::layer_children(bl, layer_root) {
            let key = crate::visit_key(&bl.uuid, &adj_layer);
            let already = visited.contains(&key);
            if !already { visited.insert(key); }

            let adj_bl  = BiLinkFile::load(&adj_path)?;
            let adj_lbl = crate::layer_label(root, &adj_layer);
            let adj_id  = add_structural(&adj_bl, &adj_layer, &adj_lbl, hg, url_scheme);

            if let (Some(ref lid), Some(ref aid)) = (&local_id, &adj_id) {
                let src_state = worst_state_str(&[&bl.state0, &bl.state1]);
                let tgt_state = worst_state_str(&[&adj_bl.state0, &adj_bl.state1]);
                let link0 = structural_endpoint_str(&bl);
                let link1 = structural_endpoint_str(&adj_bl);
                hg.add_edge(HtmlEdge {
                    id:     format!("e_{uuid_short}_{}", &lid[..8.min(lid.len())]),
                    source: lid.clone(),
                    target: aid.clone(),
                    label:  uuid_short.to_string(),
                    states: format!("{src_state}↔{tgt_state}"),
                    link0,
                    link1,
                });
            }

            if !already {
                collect(root, &adj_bl, &adj_layer, visited, hg, url_scheme, depth + 1, max_depth)?;
            }
        }
    }
    Ok(())
}

fn structural_endpoint_str(bl: &BiLinkFile) -> String {
    match (&bl.link0, &bl.link1) {
        (LinkEndpoint::Structural(_), _) => bl.link0.to_string(),
        (_, LinkEndpoint::Structural(_)) => bl.link1.to_string(),
        _ => String::new(),
    }
}

fn worst_state_str(states: &[&Option<bilinker::link::EndpointState>]) -> String {
    use bilinker::link::EndpointState::*;
    let priority = |s: &Option<bilinker::link::EndpointState>| match s {
        None                   => 1,
        Some(Ok)               => 2,
        Some(Displaced)        => 3,
        Some(Moved)            => 3,
        Some(Reanchored)       => 3,
        Some(Expanded)         => 3,
        Some(ChainDirty)       => 4,
        Some(Pending)          => 5,
        Some(Todo)             => 5,
        Some(Altered)          => 6,
        Some(Unanchored)       => 7,
        Some(Deleted)          => 8,
        Some(Broken)           => 9,
    };
    states.iter()
        .max_by_key(|s| priority(s))
        .map(|s| bilinker::state_str(s))
        .unwrap_or_else(|| "NONE".to_string())
}

// ─── daemon IPC ───────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct DaemonCallee {
    symbol: String,
    name:   String,
    file:   String,
    line:   u32,
    col:    u32,
}

fn daemon_ping() -> bool {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let Ok(home) = std::env::var("HOME") else { return false };
    let socket = std::path::PathBuf::from(home).join(".bilinker/daemon.sock");
    let Ok(mut stream) = UnixStream::connect(&socket) else { return false };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    if stream.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"params\":{}}\n").is_err() {
        return false;
    }
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).is_ok() && line.contains("pong")
}

fn daemon_callees_rpc(abs_file: &str, line: u32, col: u32) -> Option<Vec<DaemonCallee>> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let Ok(home) = std::env::var("HOME") else { return None };
    let socket = std::path::PathBuf::from(home).join(".bilinker/daemon.sock");
    let mut stream = UnixStream::connect(&socket).ok()?;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let file_json = serde_json::to_string(abs_file).ok()?;
    let req = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"callees\",\"params\":{{\"file\":{file_json},\"line\":{line},\"col\":{col}}}}}\n"
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut resp = String::new();
    BufReader::new(stream).read_line(&mut resp).ok()?;
    let v: serde_json::Value = serde_json::from_str(resp.trim()).ok()?;
    serde_json::from_value(v["result"].clone()).ok()
}

/// Extract the bare function/method name from a SCIP-style symbol.
/// e.g. "rust-analyzer cargo bilinker 0.1.0 check/check_layer()."   → "check_layer"
///      "rust-analyzer cargo foo 0.1.0 mod/impl#[Type]method()."    → "method"
fn fn_name_from_sym(sym: &str) -> Option<String> {
    let descriptor = sym.split_whitespace().last()?;
    let name_part  = descriptor.rsplit('/').next()?;
    let trimmed = name_part.trim_end_matches('.').trim_end_matches(')').trim_end_matches('(');
    // impl#[Type]method → method
    let name = if let Some(pos) = trimmed.rfind(']') { &trimmed[pos+1..] } else { trimmed };
    if name.is_empty() { None } else { Some(name.to_string()) }
}

/// Find the first occurrence of `fn <name>` in the file; return (0-indexed line, col at name start).
fn find_fn_in_file(abs_file: &str, fn_name: &str) -> Option<(u32, u32)> {
    let src = std::fs::read_to_string(abs_file).ok()?;
    let needle = format!("fn {fn_name}");
    for (i, line) in src.lines().enumerate() {
        if let Some(col) = line.find(&needle) {
            return Some((i as u32, (col + 3) as u32)); // position at the identifier
        }
    }
    None
}

fn read_lines_at_file(abs_file: &str, start_line: usize, max_lines: usize) -> (String, usize) {
    let Ok(src) = std::fs::read_to_string(abs_file) else { return (String::new(), 1) };
    let lines: Vec<&str> = src.lines().collect();
    if lines.is_empty() { return (String::new(), 1); }
    let from = start_line.min(lines.len().saturating_sub(1));
    let to   = (from + max_lines).min(lines.len());
    (lines[from..to].join("\n"), from + 1)
}

fn collect_daemon_nodes(
    abs_file: &str,
    line: u32,
    col: u32,
    parent_id: &str,
    bilink_dir: &Path,
    workspace: &str,
    depth: usize,
    visited: &mut HashSet<String>,
    result: &mut Vec<ScipNode>,
    all_ok: &mut bool,
) {
    const MAX_DEPTH: usize = 3;
    if depth > MAX_DEPTH { return; }
    let Some(callees) = daemon_callees_rpc(abs_file, line, col) else { return };
    for callee in callees {
        // Skip stdlib/dependency files — only follow workspace code
        if !callee.file.starts_with(workspace) { continue; }
        let rel_file = callee.file
            .strip_prefix(workspace).unwrap_or(&callee.file)
            .trim_start_matches('/');

        let id = scip_node_id(&callee.symbol);
        if !visited.insert(id.clone()) { continue; }
        let sciplink_path = bilink_dir.join("sciplink").join(normalize_symbol_id(&callee.symbol));
        let (state, ok) = if sciplink_path.exists() {
            match ScipLink::load(&sciplink_path) {
                Ok(sl) => {
                    let ok = matches!(sl.state, Some(ScipLinkState::Ok) | None);
                    (sl.state.map(|s| s.as_str().to_string()).unwrap_or_else(|| "PENDING".to_string()), ok)
                }
                Err(_) => ("?".to_string(), true),
            }
        } else {
            ("PENDING".to_string(), true)
        };
        if !ok { *all_ok = false; }
        let lang = lang_from_file(rel_file);
        let (content, start_line) = read_lines_at_file(&callee.file, callee.line as usize, 100);
        result.push(ScipNode {
            id: id.clone(), sym: callee.symbol.clone(), short: callee.name.clone(),
            file: rel_file.to_string(), state, ok, depth,
            parent_id: parent_id.to_string(), content, start_line, lang,
        });
        collect_daemon_nodes(
            &callee.file, callee.line, callee.col, &id,
            bilink_dir, workspace, depth + 1, visited, result, all_ok,
        );
    }
}

fn add_structural(
    bl: &BiLinkFile,
    layer_root: &Path,
    lbl: &str,
    hg: &mut HtmlGraph,
    _url_scheme: &str,
) -> Option<String> {
    // Cache daemon availability once per HtmlGraph instance
    if hg.daemon_ok.is_none() {
        hg.daemon_ok = Some(daemon_ping());
    }
    let daemon_available = hg.daemon_ok == Some(true);

    // Load ScipIndex once per layer_root (cached in HtmlGraph)
    let scip_path = layer_root.join(".bilink/index/index.scip");
    let layer_root_buf = layer_root.to_path_buf();
    if !hg.scip_cache.contains_key(&layer_root_buf) {
        let loaded = if scip_path.exists() {
            ScipIndex::load(&scip_path, layer_root).ok()
        } else {
            None
        };
        hg.scip_cache.insert(layer_root_buf.clone(), loaded);
    }
    let scip = hg.scip_cache.get(&layer_root_buf).and_then(|o| o.as_ref());
    let (sref, range, ep_n) = match (&bl.link0, &bl.link1) {
        (LinkEndpoint::Structural(s), _) => (s, bl.range0.as_ref(), 0u8),
        (_, LinkEndpoint::Structural(s)) => (s, bl.range1.as_ref(), 1u8),
        _ => return None,
    };
    let (content, start_line) = file_content(layer_root, sref, range);
    let lang     = lang_from_file(&sref.file);
    let abs_path = layer_root.join(&sref.file)
                       .canonicalize()
                       .unwrap_or_else(|_| layer_root.join(&sref.file))
                       .to_string_lossy()
                       .into_owned();

    let id    = format!("{}@{lbl}#L{start_line}", sref.file);
    let label = if start_line > 1 {
        format!("{}#L{start_line}", sref.file)
    } else {
        sref.file.clone()
    };

    // Load subgraph callees: daemon first, SCIP as fallback
    let subgraph_sym = if ep_n == 0 { bl.subgraph0.as_deref() } else { bl.subgraph1.as_deref() };
    let (sciplinks, subgraph_ok) = if subgraph_sym.is_some() {
        let bilink_dir = layer_root.join(".bilink");
        let mut visited = HashSet::new();
        let mut nodes   = Vec::new();
        let mut all_ok  = true;

        let workspace = layer_root.canonicalize()
            .unwrap_or_else(|_| layer_root.to_path_buf())
            .to_string_lossy()
            .into_owned();
        let used_daemon = if daemon_available {
            if let Some(sym) = subgraph_sym {
                let full = layer_root.join(&sref.file);
                let abs  = full.canonicalize().unwrap_or(full).to_string_lossy().into_owned();
                if let Some(name) = fn_name_from_sym(sym) {
                    if let Some((ln, col)) = find_fn_in_file(&abs, &name) {
                        collect_daemon_nodes(&abs, ln, col, &id, &bilink_dir, &workspace, 1, &mut visited, &mut nodes, &mut all_ok);
                        true  // daemon queried — skip SCIP regardless of result count
                    } else { false }
                } else { false }
            } else { false }
        } else { false };

        if !used_daemon {
            if let (Some(sym), Some(idx)) = (subgraph_sym, scip) {
                collect_scip_nodes(idx, sym, &id, &bilink_dir, 1, &mut visited, &mut nodes, &mut all_ok);
            }
        }

        for sn in nodes {
            hg.add_scip_node(sn);
        }
        let flat = hg.scip_nodes.iter()
            .filter(|sn| sn.parent_id.starts_with(&id) || sn.parent_id == id
                || hg.scip_nodes.iter().any(|p| p.id == sn.parent_id))
            .map(|sn| ScipLinkData {
                symbol: sn.sym.clone(), symbol_short: sn.short.clone(),
                file: sn.file.clone(), state: sn.state.clone(), ok: sn.ok,
            }).collect::<Vec<_>>();
        (flat, all_ok)
    } else {
        (vec![], true)
    };

    hg.add_node(HtmlNode {
        id: id.clone(), file: sref.file.clone(), label, layer: lbl.to_string(),
        abs_path, content, start_line, lang,
        subgraph_sym: subgraph_sym.map(|s| s.to_string()),
        sciplinks, subgraph_ok,
    });
    Some(id)
}

fn scip_node_id(sym: &str) -> String {
    format!("scip_{}", normalize_symbol_id(sym).replace('.', "_").replace('-', "_"))
}

fn collect_scip_nodes(
    scip: &ScipIndex,
    sym: &str,
    parent_id: &str,
    bilink_dir: &Path,
    depth: usize,
    visited: &mut HashSet<String>,
    result: &mut Vec<ScipNode>,
    all_ok: &mut bool,
) {
    for (callee, _, _) in scip.direct_callees(sym) {
        if !visited.insert(callee.clone()) { continue; }

        let sciplink_path = bilink_dir.join("sciplink").join(normalize_symbol_id(&callee));
        let (state, ok) = if sciplink_path.exists() {
            match ScipLink::load(&sciplink_path) {
                Ok(sl) => {
                    let ok = matches!(sl.state, Some(ScipLinkState::Ok) | None);
                    (sl.state.map(|s| s.as_str().to_string()).unwrap_or_else(|| "PENDING".to_string()), ok)
                }
                Err(_) => ("?".to_string(), true),
            }
        } else {
            ("PENDING".to_string(), true)
        };

        if !ok { *all_ok = false; }

        let short = callee.rsplit('/').next().unwrap_or(&callee)
            .trim_end_matches('.').to_string();
        let file = scip.definition(&callee).map(|(f, _)| f.to_string()).unwrap_or_default();
        let id   = scip_node_id(&callee);
        let lang = lang_from_file(&file);

        // Read source code at body range
        let (content, start_line) = scip.body_range(&callee)
            .and_then(|(f, range)| {
                let src = std::fs::read(bilink_dir.parent()?.join(f)).ok()?;
                let frag = &src[range.start.min(src.len())..range.end.min(src.len())];
                let text = std::str::from_utf8(frag).ok()?;
                let s_line = src[..range.start.min(src.len())]
                    .iter().filter(|&&b| b == b'\n').count() + 1;
                let display = text.lines().take(100).collect::<Vec<_>>().join("\n");
                Some((display, s_line))
            })
            .unwrap_or_default();

        result.push(ScipNode {
            id: id.clone(), sym: callee.clone(), short, file, state, ok, depth,
            parent_id: parent_id.to_string(), content, start_line, lang,
        });

        collect_scip_nodes(scip, &callee, &id, bilink_dir, depth + 1, visited, result, all_ok);
    }
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) { i -= 1; }
    i
}

fn file_content(layer_root: &Path, sref: &bilinker::link::StructuralRef, range: Option<&ByteRange>) -> (String, usize) {
    let Ok(content) = std::fs::read_to_string(layer_root.join(&sref.file)) else {
        return (String::new(), 1);
    };

    // Prefer stored range; fall back to running the query; last resort: whole file
    let resolved_range: Option<(usize, usize)> = if let Some(r) = range {
        let start = floor_char_boundary(&content, r.start.min(content.len()));
        let end   = floor_char_boundary(&content, r.end.min(content.len()));
        Some((start, end))
    } else if let Some(q) = &sref.query {
        let lang = bilinker::grammar::language_for_file(&sref.file);
        bilinker::grammar::for_language(lang).ok()
            .and_then(|language| bilinker::query::find_target(language, &content, q).ok().flatten())
            .map(|(s, e)| (floor_char_boundary(&content, s), floor_char_boundary(&content, e)))
    } else {
        None
    };

    if let Some((start, end)) = resolved_range {
        let before     = &content[..start];
        let start_line = before.chars().filter(|&c| c == '\n').count() + 1;
        let frag       = &content[start..end];
        let text       = frag.lines().take(100).collect::<Vec<_>>().join("\n");
        (text, start_line)
    } else {
        let text = content.lines().take(100).collect::<Vec<_>>().join("\n");
        (text, 1)
    }
}

// ─── HTML template ────────────────────────────────────────────────────────────

const TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>Bilink Graph</title>
<link rel="stylesheet" href="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/github-dark.min.css">
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: 'Courier New', monospace; display: flex; height: 100vh; overflow: hidden; background: #0d1117; color: #c9d1d9; }
#cy { flex: 1; background: #0d1117; }
#panel { width: 420px; padding: 20px; overflow-y: auto; background: #161b22; border-left: 2px solid #30363d; display: flex; flex-direction: column; gap: 12px; }
.hint     { color: #6e7681; font-size: 13px; }
.ntitle   { font-size: 14px; font-weight: bold; color: #58a6ff; word-break: break-all; }
.nlayer   { font-size: 11px; color: #8b949e; margin-top: 2px; }
.open-link { display: inline-block; padding: 5px 14px; background: #1f6feb; color: #fff; text-decoration: none; border-radius: 6px; font-size: 12px; margin-top: 4px; }
.open-link:hover { background: #388bfd; }

/* code view */
.code-wrap { border: 1px solid #30363d; border-radius: 6px; display: flex; max-height: 220px; overflow: hidden; }
.line-nums  { padding: 0.5em 0.6em; background: #161b22; border-right: 1px solid #30363d; text-align: right; color: #6e7681; font-size: 11px; line-height: 1.6; user-select: none; white-space: pre; flex-shrink: 0; overflow: hidden; }
.code-wrap pre  { margin: 0; padding: 0.5em 0; overflow: auto; font-size: 11px; line-height: 1.6; flex: 1; min-width: 0; }
.code-wrap code { display: block; white-space: pre; }

/* markdown view */
.md-wrap { background: #0d1117; border: 1px solid #30363d; border-radius: 6px; padding: 16px; overflow-y: auto; max-height: 220px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 13px; line-height: 1.7; color: #c9d1d9; }
.md-wrap h1,.md-wrap h2,.md-wrap h3,.md-wrap h4 { color: #58a6ff; margin: 16px 0 6px; }
.md-wrap h1 { font-size: 18px; border-bottom: 1px solid #30363d; padding-bottom: 6px; }
.md-wrap h2 { font-size: 15px; }
.md-wrap p  { margin: 8px 0; }
.md-wrap ul,.md-wrap ol { padding-left: 20px; margin: 6px 0; }
.md-wrap code { background: #161b22; padding: 2px 5px; border-radius: 3px; font-family: 'Courier New', monospace; font-size: 11px; color: #a5d6ff; }
.md-wrap pre  { background: #161b22; border: 1px solid #30363d; border-radius: 6px; padding: 12px; overflow-x: auto; margin: 8px 0; }
.md-wrap pre code { background: none; padding: 0; font-size: 11px; }
.md-wrap table { border-collapse: collapse; width: 100%; margin: 8px 0; font-size: 12px; }
.md-wrap td,.md-wrap th { border: 1px solid #30363d; padding: 5px 10px; }
.md-wrap th { background: #1c2938; color: #58a6ff; }
.md-wrap a  { color: #58a6ff; }
.md-wrap blockquote { border-left: 3px solid #30363d; padding-left: 12px; color: #8b949e; margin: 8px 0; }
.md-wrap hr { border: none; border-top: 1px solid #30363d; margin: 12px 0; }

/* bilink divider */
.bilink-sep { display: flex; align-items: center; gap: 10px; margin: 4px 0; }
.bilink-sep-line { flex: 1; border-top: 1px solid #30363d; }
.bilink-sep-label { font-size: 11px; color: #58a6ff; font-family: 'Courier New', monospace; white-space: nowrap; }
.frag-label { font-size: 11px; color: #8b949e; margin-bottom: 4px; }

/* subgraph */
.sg-header { font-size: 12px; font-weight: bold; color: #8b949e; margin: 10px 0 4px; letter-spacing: 0.05em; }
.sg-list { display: flex; flex-direction: column; gap: 3px; }
.sg-item { display: flex; align-items: center; gap: 8px; font-size: 11px; font-family: 'Courier New', monospace; padding: 3px 6px; border-radius: 4px; background: #0d1117; }
.sg-sym  { flex: 1; color: #c9d1d9; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.sg-file { font-size: 10px; color: #6e7681; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 120px; }
.sg-badge { font-size: 10px; font-weight: bold; padding: 1px 6px; border-radius: 3px; white-space: nowrap; }
.sg-ok    { background: #1a3a1a; color: #3fb950; }
.sg-bad   { background: #3a1a1a; color: #f85149; }
</style>
</head>
<body>
<div id="cy"></div>
<div id="panel"><div class="hint">← Click a node or edge to view details</div></div>
<script src="https://cdnjs.cloudflare.com/ajax/libs/cytoscape/3.28.1/cytoscape.min.js"></script>
<script src="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/highlight.min.js"></script>
<script src="https://cdnjs.cloudflare.com/ajax/libs/marked/9.1.6/marked.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js"></script>
<script>
mermaid.initialize({ startOnLoad: false, theme: 'dark' });

// Override marked renderer to render mermaid blocks as diagrams
const renderer = new marked.Renderer();
const origCode = renderer.code.bind(renderer);
renderer.code = function(code, lang) {
  if (lang === 'mermaid') {
    return `<div class="mermaid">${code}</div>`;
  }
  return origCode(code, lang);
};
marked.use({ renderer });

const G   = GRAPH_DATA_PLACEHOLDER;
const COL = 540, ROW = 90;
const elements = [];

G.layers.forEach(l =>
  elements.push({ data: { id: l.id, label: l.label, type: 'layer' } })
);

G.file_groups.forEach(fg =>
  elements.push({ data: { id: fg.id, label: fg.label, parent: fg.layer_id, type: 'file-group' } })
);

// Group layers by depth and assign vertical bands
const layerOrder = {};
const depthGroups = {};
G.layers.forEach(l => {
  if (!depthGroups[l.depth]) depthGroups[l.depth] = [];
  depthGroups[l.depth].push(l.id);
});

// Count nodes per layer to compute band heights
const nodesPerLayer = {};
G.nodes.forEach(n => { nodesPerLayer[n.layer_id] = (nodesPerLayer[n.layer_id] || 0) + 1; });

// Assign a y-band start offset to each layer within its depth column
const layerYStart = {};
Object.keys(depthGroups).sort().forEach(depth => {
  let offset = 0;
  depthGroups[depth].forEach(lid => {
    layerYStart[lid] = offset;
    offset += ((nodesPerLayer[lid] || 1) + 2) * ROW; // +2 rows gap between layers
  });
});

const yIdx = {};
G.nodes.forEach(n => {
  const k = n.layer_id;
  yIdx[k] = yIdx[k] || 0;
  const y = (layerYStart[n.layer_id] || 0) + yIdx[k]++ * ROW;
  elements.push({
    data: { id: n.id, label: n.label, parent: n.file_group_id, type: 'file',
            abs_path: n.abs_path, content: n.content, layer: n.layer,
            start_line: n.start_line, lang: n.lang,
            subgraph_sym: n.subgraph_sym, subgraph_ok: n.subgraph_ok,
            sciplinks: n.sciplinks },
    position: { x: n.xi * COL, y: y }
  });
});

G.edges.forEach(e =>
  elements.push({ data: { id: e.id, source: e.source, target: e.target,
                          label: e.label + '\n' + e.states,
                          uuid: e.label, states: e.states,
                          link0: e.link0, link1: e.link1,
                          type: 'bilink-edge' } })
);

// Add scip nodes (deduplicated)
const SCIP_COL = 220, SCIP_ROW = 56;
const scipNodeMap = {};
(G.scip_nodes || []).forEach(sn => {
  scipNodeMap[sn.id] = sn;
  elements.push({ data: {
    id: sn.id, label: sn.short, type: 'scip',
    sym: sn.sym, file: sn.file, state: sn.state, ok: sn.ok,
    content: sn.content, start_line: sn.start_line, lang: sn.lang
  }});
});
// Add scip edges (one per caller→callee relationship, unique id by src+tgt)
(G.scip_edges || []).forEach((e, i) => {
  const ok = scipNodeMap[e.t] ? scipNodeMap[e.t].ok : true;
  elements.push({ data: {
    id: 'se_' + e.s.replace(/[^a-zA-Z0-9]/g,'_') + '_' + e.t.replace(/[^a-zA-Z0-9]/g,'_'),
    source: e.s, target: e.t, type: 'scip-edge', ok
  }});
});

const cy = cytoscape({
  container: document.getElementById('cy'),
  elements,
  style: [
    { selector: 'node[type="layer"]', style: {
        'background-color': 'rgba(255,255,255,0.03)', 'background-opacity': 1,
        'border-color': '#30363d', 'border-style': 'dashed', 'border-width': 2,
        'label': 'data(label)', 'text-valign': 'top', 'text-halign': 'center',
        'color': '#6e7681', 'font-family': 'Courier New', 'font-size': 12, 'padding': 28 }},
    { selector: 'node[type="file-group"]', style: {
        'background-color': 'rgba(31,111,235,0.07)', 'background-opacity': 1,
        'border-color': '#1f6feb', 'border-style': 'solid', 'border-width': 1,
        'label': 'data(label)', 'text-valign': 'top', 'text-halign': 'center',
        'color': '#58a6ff', 'font-family': 'Courier New', 'font-size': 10, 'padding': 14 }},
    { selector: 'node[type="file"]', style: {
        'shape': 'round-rectangle', 'background-color': '#161b22',
        'border-color': '#1f6feb', 'border-width': 1.5,
        'label': 'data(label)', 'text-valign': 'center',
        'color': '#c9d1d9', 'font-family': 'Courier New', 'font-size': 11,
        'padding': 10, 'width': 'label', 'cursor': 'pointer' }},
    { selector: 'node[type="file"][subgraph_ok=false]', style: {
        'border-color': '#f85149', 'border-width': 2.5, 'background-color': '#1c1010' }},
    { selector: 'node[type="file"]:selected', style: {
        'border-color': '#58a6ff', 'border-width': 2.5, 'background-color': '#1c2938' }},
    { selector: 'node[type="file"][subgraph_ok=false]:selected', style: {
        'border-color': '#ff7b7b', 'border-width': 3, 'background-color': '#2a1515' }},
    { selector: 'node[type="scip"]', style: {
        'shape': 'round-rectangle', 'background-color': '#0d2a1f',
        'border-color': '#3fb950', 'border-width': 1.5,
        'label': 'data(label)', 'text-valign': 'center',
        'color': '#adbac7', 'font-family': 'Courier New', 'font-size': 9,
        'padding': 6, 'width': 'label', 'cursor': 'pointer',
        'display': 'none' }},
    { selector: 'node[type="scip"][ok=false]', style: {
        'background-color': '#2a0d0d', 'border-color': '#f85149' }},
    { selector: 'node[type="scip"]:selected', style: {
        'border-color': '#58a6ff', 'border-width': 2.5 }},
    { selector: 'edge[type="scip-edge"]', style: {
        'curve-style': 'bezier', 'target-arrow-shape': 'triangle',
        'line-color': '#3fb950', 'target-arrow-color': '#3fb950',
        'line-style': 'dashed', 'width': 1, 'opacity': 0.6,
        'display': 'none' }},
    { selector: 'edge[type="scip-edge"][ok=false]', style: {
        'line-color': '#f85149', 'target-arrow-color': '#f85149' }},
    { selector: 'edge', style: {
        'curve-style': 'bezier', 'target-arrow-shape': 'triangle', 'source-arrow-shape': 'triangle',
        'label': 'data(label)', 'color': '#8b949e', 'font-family': 'Courier New', 'font-size': 9,
        'text-background-color': '#0d1117', 'text-background-opacity': 0.85, 'text-background-padding': 3,
        'line-color': '#30363d', 'target-arrow-color': '#30363d', 'source-arrow-color': '#30363d',
        'width': 1.5, 'text-wrap': 'wrap' }},
    { selector: 'edge:selected', style: {
        'line-color': '#1f6feb', 'target-arrow-color': '#1f6feb', 'source-arrow-color': '#1f6feb' }}
  ],
  layout: { name: 'preset' }
});

cy.fit(undefined, 40);

// Position scip nodes relative to the first bilink parent that references them
(function positionScipNodes() {
  // Build depth map: scip_id → depth from nearest bilink parent
  const scipDepth = {};
  function assignDepth(nodeId, d) {
    (G.scip_edges || []).filter(e => e.s === nodeId).forEach(e => {
      if (scipDepth[e.t] === undefined) {
        scipDepth[e.t] = d;
        assignDepth(e.t, d + 1);
      }
    });
  }
  // Seed from bilink nodes
  G.nodes.forEach(n => { if (n.subgraph_sym) assignDepth(n.id, 1); });

  // Count siblings per (first-bilink-parent, depth)
  const sibCount = {}, sibIdx = {};
  (G.scip_edges || []).forEach(e => {
    const d = scipDepth[e.t] || 1;
    const key = e.s + ':' + d;
    sibCount[key] = (sibCount[key] || 0) + 1;
  });

  // Position each scip node based on its first referenced bilink parent
  const positioned = new Set();
  (G.scip_edges || []).forEach(e => {
    if (positioned.has(e.t)) return;
    const pNode = cy.getElementById(e.s);
    if (!pNode || pNode.length === 0) return;
    const pp = pNode.position();
    const d  = scipDepth[e.t] || 1;
    const key = e.s + ':' + d;
    const idx = sibIdx[key] = (sibIdx[key] || 0);
    sibIdx[key]++;
    const total = sibCount[key] || 1;
    const cyNode = cy.getElementById(e.t);
    if (!cyNode || cyNode.length === 0) return;
    cyNode.position({
      x: pp.x + d * SCIP_COL,
      y: pp.y + (idx - (total - 1) / 2) * SCIP_ROW
    });
    positioned.add(e.t);
  });
  cy.fit(undefined, 40);
})();

function renderScipNode(n) {
  const badge = n.ok
    ? `<span class="sg-badge sg-ok">OK</span>`
    : `<span class="sg-badge sg-bad">${esc(n.state)}</span>`;
  const txt = n.content || '(no content)';
  let contentHtml;
  if (n.lang === 'markdown') {
    contentHtml = `<div class="md-wrap">${marked.parse(txt)}</div>`;
    setTimeout(() => mermaid.run({ querySelector: '#panel .mermaid' }), 50);
  } else {
    const lang  = n.lang || 'plaintext';
    const hl    = hljs.highlight(txt, { language: lang, ignoreIllegals: true });
    const count = txt.split('\n').length;
    const start = n.start_line || 1;
    const nums  = Array.from({ length: count }, (_, i) => start + i).join('\n');
    contentHtml = `<div class="code-wrap"><div class="line-nums">${nums}</div><pre><code class="hljs language-${lang}">${hl.value}</code></pre></div>`;
  }
  return `
    <div class="ntitle">${esc(n.label)}</div>
    <div class="nlayer" style="word-break:break-all;font-size:10px">${esc(n.sym)}</div>
    <div class="nlayer">${esc(n.file)}</div>
    <div style="margin-top:6px">${badge}</div>
    ${contentHtml}`;
}

function renderNode(n) {
  if (!n) return '<div class="hint">(no content)</div>';
  const rel = n.abs_path ? relUrl(n.abs_path, n.start_line) : '';
  const url = rel ? `<a class="open-link" href="${rel}" target="_blank">Open file</a>` : '';
  const txt = n.content || '(no content)';
  let contentHtml;
  if (n.lang === 'markdown') {
    contentHtml = `<div class="md-wrap">${marked.parse(txt)}</div>`;
    setTimeout(() => mermaid.run({ querySelector: '#panel .mermaid' }), 50);
  } else {
    const lang  = n.lang || 'plaintext';
    const hl    = hljs.highlight(txt, { language: lang, ignoreIllegals: true });
    const count = txt.split('\n').length;
    const start = n.start_line || 1;
    const nums  = Array.from({ length: count }, (_, i) => start + i).join('\n');
    contentHtml = `<div class="code-wrap"><div class="line-nums">${nums}</div><pre><code class="hljs language-${lang}">${hl.value}</code></pre></div>`;
  }
  let sgHtml = '';
  if (n.sciplinks && n.sciplinks.length > 0) {
    const items = n.sciplinks.map(sl => {
      const badge = sl.ok
        ? `<span class="sg-badge sg-ok">OK</span>`
        : `<span class="sg-badge sg-bad">${esc(sl.state)}</span>`;
      return `<div class="sg-item">${badge}<span class="sg-sym" title="${esc(sl.symbol)}">${esc(sl.symbol_short)}</span><span class="sg-file" title="${esc(sl.file)}">${esc(sl.file)}</span></div>`;
    }).join('');
    sgHtml = `<div class="sg-header">SUBGRAPH</div><div class="sg-list">${items}</div>`;
  }
  return `<div class="ntitle">${esc(n.label)}</div><div class="nlayer">${esc(n.layer)}</div>${url}${contentHtml}${sgHtml}`;
}

// ── Subgraph hover show/hide ──────────────────────────────────────────────────
// Build parent → children map from scip_edges (covers multiple parents per node)
const scipChildren = {};
(G.scip_edges || []).forEach(e => {
  (scipChildren[e.s] = scipChildren[e.s] || []).push(e.t);
});

function allScipDescendants(nodeId) {
  const result = [];
  const queue  = [nodeId];
  while (queue.length) {
    const id = queue.shift();
    (scipChildren[id] || []).forEach(cid => { result.push(cid); queue.push(cid); });
  }
  return result;
}

let hideTimer  = null;
let shownForId = null;

function showScipSubgraph(bilink_id) {
  if (hideTimer) { clearTimeout(hideTimer); hideTimer = null; }
  if (shownForId === bilink_id) return;
  if (shownForId) hideScipNow(shownForId);
  shownForId = bilink_id;
  allScipDescendants(bilink_id).forEach(id => {
    cy.getElementById(id).style('display', 'element');
  });
  // Show all scip-edges that start from bilink_id or from any descendant
  const shown = new Set([bilink_id, ...allScipDescendants(bilink_id)]);
  cy.edges('[type="scip-edge"]').forEach(e => {
    if (shown.has(e.data('source'))) e.style('display', 'element');
  });
}

function scheduleHide(bilink_id) {
  hideTimer = setTimeout(() => { hideScipNow(bilink_id); shownForId = null; }, 250);
}

function hideScipNow(bilink_id) {
  allScipDescendants(bilink_id).forEach(id => {
    cy.getElementById(id).style('display', 'none');
  });
  const shown = new Set([bilink_id, ...allScipDescendants(bilink_id)]);
  cy.edges('[type="scip-edge"]').forEach(e => {
    if (shown.has(e.data('source'))) e.style('display', 'none');
  });
}

cy.on('mouseover', 'node[type="file"]', evt => {
  const n = evt.target.data();
  if (n.subgraph_sym) showScipSubgraph(n.id);
});
cy.on('mouseout', 'node[type="file"]', evt => {
  const n = evt.target.data();
  if (n.subgraph_sym) scheduleHide(n.id);
});
cy.on('mouseover', 'node[type="scip"]', evt => {
  if (shownForId) { if (hideTimer) { clearTimeout(hideTimer); hideTimer = null; } }
});
cy.on('mouseout', 'node[type="scip"]', evt => {
  if (shownForId) scheduleHide(shownForId);
});

// ── Click handlers ────────────────────────────────────────────────────────────
cy.on('tap', 'node[type="file"]', evt => {
  const n = evt.target.data();
  document.getElementById('panel').innerHTML = renderNode(n);
});

cy.on('tap', 'node[type="scip"]', evt => {
  const n = evt.target.data();
  document.getElementById('panel').innerHTML = renderScipNode(n);
});

cy.on('tap', 'edge', evt => {
  const e      = evt.target.data();
  const src    = cy.getElementById(e.source).data();
  const tgt    = cy.getElementById(e.target).data();
  const uuid   = e.uuid   || e.label || '';
  const states = e.states || '';
  const link0  = e.link0  || '';
  const link1  = e.link1  || '';
  document.getElementById('panel').innerHTML = `
    <div class="frag-label">link.0 — <code style="color:#a5d6ff;font-size:10px;word-break:break-all">${esc(link0)}</code></div>
    ${renderNode(src)}
    <div class="bilink-sep">
      <div class="bilink-sep-line"></div>
      <div class="bilink-sep-label">${esc(uuid)} · ${esc(states)}</div>
      <div class="bilink-sep-line"></div>
    </div>
    <div class="frag-label">link.1 — <code style="color:#a5d6ff;font-size:10px;word-break:break-all">${esc(link1)}</code></div>
    ${renderNode(tgt)}`;
  setTimeout(() => mermaid.run({ querySelector: '#panel .mermaid' }), 50);
});

function esc(s) {
  return (s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}

function relUrl(absPath, startLine) {
  const htmlDir = window.location.pathname.replace(/\/[^\/]*$/, '');
  const h = htmlDir.split('/').filter(Boolean);
  const f = absPath.split('/').filter(Boolean);
  let i = 0;
  while (i < h.length && i < f.length && h[i] === f[i]) i++;
  const rel = '../'.repeat(h.length - i) + f.slice(i).join('/');
  const frag = startLine > 1 ? '#L' + startLine : '';
  return rel + frag;
}
</script>
</body>
</html>"#;
