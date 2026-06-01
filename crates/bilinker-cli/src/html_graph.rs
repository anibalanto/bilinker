use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use bilinker::link::{LinkEndpoint, ByteRange};
use bilinker::bilink::BiLinkFile;

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

pub struct HtmlNode {
    pub id:         String,
    pub label:      String,
    pub layer:      String,
    pub url:        String,
    pub content:    String,
    pub start_line: usize,
    pub lang:       &'static str,
}

pub struct HtmlEdge {
    pub id:     String,
    pub source: String,
    pub target: String,
    pub label:  String,
    pub states: String,
}

#[derive(Default)]
pub struct HtmlGraph {
    layers:     BTreeMap<String, usize>,
    nodes:      Vec<HtmlNode>,
    edges:      Vec<HtmlEdge>,
    seen_nodes: HashSet<String>,
    seen_edges: HashSet<String>,
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

        let mut depth_counters: HashMap<usize, usize> = HashMap::new();
        let nodes_json = self.nodes.iter().map(|n| {
            let depth = *self.layers.get(&n.layer).unwrap_or(&0);
            let idx   = { let c = depth_counters.entry(depth).or_insert(0); let v = *c; *c += 1; v };
            format!(
                r#"{{"id":"{}","label":"{}","layer_id":"{}","layer":"{}","url":"{}","content":"{}","start_line":{},"lang":"{}","xi":{},"yi":{}}}"#,
                esc_json(&n.id), esc_json(&n.label),
                esc_json(&layer_id(&n.layer)), esc_json(&n.layer),
                esc_json(&n.url), esc_json(&n.content),
                n.start_line, n.lang, depth, idx
            )
        }).collect::<Vec<_>>().join(",");

        let edges_json = self.edges.iter().map(|e| {
            format!(
                r#"{{"id":"{}","source":"{}","target":"{}","label":"{}","states":"{}"}}"#,
                esc_json(&e.id), esc_json(&e.source), esc_json(&e.target),
                esc_json(&e.label), esc_json(&e.states)
            )
        }).collect::<Vec<_>>().join(",");

        let data = format!(r#"{{"layers":[{layers_json}],"nodes":[{nodes_json}],"edges":[{edges_json}]}}"#);
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
    let s0 = bl.state0.as_ref().map(|s| s.to_string()).unwrap_or_else(|| "-".into());
    let s1 = bl.state1.as_ref().map(|s| s.to_string()).unwrap_or_else(|| "-".into());
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
                hg.add_edge(HtmlEdge {
                    id:     format!("e_{uuid_short}_{}", &lid[..8.min(lid.len())]),
                    source: lid.clone(),
                    target: aid.clone(),
                    label:  uuid_short.to_string(),
                    states: format!("{s0}↔{s1}"),
                });
            }

            if !already {
                collect(root, &adj_bl, &adj_layer, visited, hg, url_scheme, depth + 1, max_depth)?;
            }
        }
    }
    Ok(())
}

fn add_structural(
    bl: &BiLinkFile,
    layer_root: &Path,
    lbl: &str,
    hg: &mut HtmlGraph,
    url_scheme: &str,
) -> Option<String> {
    let (sref, range) = match (&bl.link0, &bl.link1) {
        (LinkEndpoint::Structural(s), _) => (s, bl.range0.as_ref()),
        (_, LinkEndpoint::Structural(s)) => (s, bl.range1.as_ref()),
        _ => return None,
    };
    let url                   = crate::node_url(layer_root, &sref.file, range, url_scheme);
    let (content, start_line) = file_content(layer_root, &sref.file, range);
    let lang                  = lang_from_file(&sref.file);

    // Include start_line in ID so different fragments of the same file are distinct nodes
    let id    = format!("{}@{lbl}#L{start_line}", sref.file);
    let label = if start_line > 1 {
        format!("{}#L{start_line}", sref.file)
    } else {
        sref.file.clone()
    };

    hg.add_node(HtmlNode {
        id: id.clone(), label, layer: lbl.to_string(),
        url, content, start_line, lang,
    });
    Some(id)
}

fn file_content(layer_root: &Path, file: &str, range: Option<&ByteRange>) -> (String, usize) {
    let Ok(content) = std::fs::read_to_string(layer_root.join(file)) else {
        return (String::new(), 1);
    };
    if let Some(r) = range {
        let start  = r.start.min(content.len());
        let end    = r.end.min(content.len());
        let before = &content[..start];
        let start_line = before.chars().filter(|&c| c == '\n').count() + 1;
        let frag   = content.get(start..end).unwrap_or("");
        let text   = frag.lines().take(100).collect::<Vec<_>>().join("\n");
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
.code-wrap { border: 1px solid #30363d; border-radius: 6px; display: flex; max-height: 460px; overflow: hidden; }
.line-nums  { padding: 1em 0.6em; background: #161b22; border-right: 1px solid #30363d; text-align: right; color: #6e7681; font-size: 11px; line-height: 1.6; user-select: none; white-space: pre; flex-shrink: 0; overflow: hidden; }
.code-wrap pre  { margin: 0; overflow: auto; font-size: 11px; line-height: 1.6; flex: 1; min-width: 0; }
.code-wrap code { display: block; white-space: pre; }

/* markdown view */
.md-wrap { background: #0d1117; border: 1px solid #30363d; border-radius: 6px; padding: 16px; overflow-y: auto; max-height: 460px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 13px; line-height: 1.7; color: #c9d1d9; }
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
</style>
</head>
<body>
<div id="cy"></div>
<div id="panel"><div class="hint">← Click a node to view details</div></div>
<script src="https://cdnjs.cloudflare.com/ajax/libs/cytoscape/3.28.1/cytoscape.min.js"></script>
<script src="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/highlight.min.js"></script>
<script src="https://cdnjs.cloudflare.com/ajax/libs/marked/9.1.6/marked.min.js"></script>
<script>
const G   = GRAPH_DATA_PLACEHOLDER;
const COL = 540, ROW = 90;
const elements = [];

G.layers.forEach(l =>
  elements.push({ data: { id: l.id, label: l.label, type: 'layer' } })
);

const yIdx = {};
G.nodes.forEach(n => {
  const k = n.xi;
  yIdx[k] = yIdx[k] || 0;
  elements.push({
    data: { id: n.id, label: n.label, parent: n.layer_id, type: 'file',
            url: n.url, content: n.content, layer: n.layer,
            start_line: n.start_line, lang: n.lang },
    position: { x: n.xi * COL, y: yIdx[k]++ * ROW }
  });
});

G.edges.forEach(e =>
  elements.push({ data: { id: e.id, source: e.source, target: e.target,
                          label: e.label + '\n' + e.states } })
);

const cy = cytoscape({
  container: document.getElementById('cy'),
  elements,
  style: [
    { selector: 'node[type="layer"]', style: {
        'background-color': 'rgba(255,255,255,0.03)', 'background-opacity': 1,
        'border-color': '#30363d', 'border-style': 'dashed', 'border-width': 2,
        'label': 'data(label)', 'text-valign': 'top', 'text-halign': 'center',
        'color': '#6e7681', 'font-family': 'Courier New', 'font-size': 12, 'padding': 28 }},
    { selector: 'node[type="file"]', style: {
        'shape': 'round-rectangle', 'background-color': '#161b22',
        'border-color': '#1f6feb', 'border-width': 1.5,
        'label': 'data(label)', 'text-valign': 'center',
        'color': '#c9d1d9', 'font-family': 'Courier New', 'font-size': 11,
        'padding': 10, 'width': 'label', 'cursor': 'pointer' }},
    { selector: 'node[type="file"]:selected', style: {
        'border-color': '#58a6ff', 'border-width': 2.5, 'background-color': '#1c2938' }},
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

cy.on('tap', 'node[type="file"]', evt => {
  const n   = evt.target.data();
  const url = n.url ? `<a class="open-link" href="${n.url}" target="_blank">Open file</a>` : '';
  const txt = n.content || '(no content)';

  let contentHtml;
  if (n.lang === 'markdown') {
    contentHtml = `<div class="md-wrap">${marked.parse(txt)}</div>`;
  } else {
    const lang    = n.lang || 'plaintext';
    const hl      = hljs.highlight(txt, { language: lang, ignoreIllegals: true });
    const count   = txt.split('\n').length;
    const start   = n.start_line || 1;
    const nums    = Array.from({ length: count }, (_, i) => start + i).join('\n');
    contentHtml   = `
      <div class="code-wrap">
        <div class="line-nums">${nums}</div>
        <pre><code class="hljs language-${lang}">${hl.value}</code></pre>
      </div>`;
  }

  document.getElementById('panel').innerHTML = `
    <div class="ntitle">${esc(n.label)}</div>
    <div class="nlayer">${esc(n.layer)}</div>
    ${url}
    ${contentHtml}`;
});

function esc(s) {
  return (s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}
</script>
</body>
</html>"#;
