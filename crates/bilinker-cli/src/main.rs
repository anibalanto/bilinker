mod html_graph;


use clap::{ArgAction, Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "bilinker", about = "Universal bidirectional structural references")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Capture a bilink endpoint from a file selection
    ///
    /// FILE is relative to the project root (.bilinker.toml).
    /// START and END are line:col positions (1-based).
    Capture {
        file: String,
        start: String,
        end: String,
    },

    /// Print content or list bilinks referencing a file/position
    ///
    /// Forms:
    ///   get <UUID>.<N>          — show fragment content at endpoint N
    ///   get <file>:<line>:<col> — list bilinks whose range covers that position
    ///   get <file>              — list all bilinks referencing that file
    Get {
        target: String,
        #[arg(short = 'B', value_name = "rows:cols")]
        before: Option<String>,
        #[arg(short = 'A', value_name = "rows:cols")]
        after: Option<String>,
    },

    /// Verify bilinks in a .bilink file or directory
    Check {
        path: PathBuf,
    },

    /// Watch for changes in linked files and alert on drift
    Watch,

    /// Apply pending auto-fixes from check
    Apply {
        #[arg(long)]
        dry_run: bool,
        #[arg(short = 'y')]
        yes: bool,
    },

    /// Manage chains of bilinks
    Chain {
        #[command(subcommand)]
        sub: ChainCommand,
    },

    /// Build or check the .bilink/.index
    Index {
        #[command(subcommand)]
        sub: IndexCommand,
    },

    /// Accept bilink endpoints, establishing their hash baseline
    ///
    /// Forms (like git add):
    ///   accept .                 — all PENDING in current .bilink/
    ///   accept commands/         — PENDING endpoints pointing into that directory
    ///   accept commands/check.md — PENDING endpoints pointing to that file
    ///   accept <uuid>            — both endpoints of that UUID
    ///   accept <uuid>.<0|1>      — one specific endpoint
    Accept {
        /// path, UUID, or UUID.N
        target: String,
        /// Override the computed hash
        #[arg(long)]
        hash: Option<String>,
        /// Override the git commit
        #[arg(long)]
        commit: Option<String>,
    },

    /// Show status of all bilinks in the current layer (like git status)
    Status {
        /// Layer directory to inspect (default: current directory)
        path: Option<PathBuf>,
    },

    /// Traverse the bilink graph from a file, position, or UUID
    Graph {
        /// File, file:line:col, or UUID
        selector: String,
        /// Maximum traversal depth (default: unlimited)
        #[arg(long)]
        depth: Option<usize>,
        /// Output format: tree, flat, dot
        #[arg(long, default_value = "tree", value_name = "FORMAT")]
        format: String,
        /// Collect bilinks from all layers under the project root
        #[arg(long)]
        recursive: bool,
        /// Show intermediate bilink nodes as diamonds (default: direct file-to-file edges)
        #[arg(long)]
        bilink_detail: bool,
        /// URL scheme for node links in dot format: line (default), file, none
        #[arg(long, default_value = "line", value_name = "SCHEME")]
        url_scheme: String,
        /// Show AST query in node labels (dot format)
        #[arg(long)]
        show_query: bool,
        /// Show byte range in node labels (dot format)
        #[arg(long)]
        show_range: bool,
        /// Show first and last line of fragment content in node labels (dot format)
        #[arg(long)]
        show_data: bool,
    },
}

#[derive(Subcommand)]
enum ChainCommand {
    /// Create a new chain or direct link
    ///
    /// Each --tip is a stratum path with optional :LINE:COL suffix.
    ///
    /// Examples:
    ///   bilinker chain new --tip commands/capture.md --tip '>impl/crates/bilinker/src/capture.rs:16:1'
    ///   bilinker chain new --tip spec/Foo.java --tip '>impl/src/Foo.java:42:5'
    New {
        /// Tip: STRATUM_PATH[:LINE:COL]  (specify exactly twice)
        #[arg(long = "tip", value_name = "REF", action = ArgAction::Append)]
        tip: Vec<String>,
        /// Intermediate layer (can repeat, order matters)
        #[arg(long = "mid", action = ArgAction::Append)]
        mid: Vec<String>,
    },
    /// Show complete state of a chain
    Status { uuid: String },
    /// List all chains in the project
    List,
}

#[derive(Subcommand)]
enum IndexCommand {
    /// Build .bilink/.index for fast file lookups
    Build {
        path: Option<PathBuf>,
        #[arg(long)]
        recursive: bool,
    },
    /// Show index status without modifying files
    Status {
        path: Option<PathBuf>,
        #[arg(long)]
        recursive: bool,
    },
}

fn parse_pos(s: &str) -> anyhow::Result<(usize, usize)> {
    let (line, col) = s.split_once(':')
        .ok_or_else(|| anyhow::anyhow!("position must be line:col, got: {s}"))?;
    Ok((line.parse()?, col.parse()?))
}

fn project_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let (root, _) = bilinker::config::Config::load_from(cwd)?;
    Ok(root)
}

/// Parses a stratum tip: `STRATUM_PATH[:LINE:COL]`.
///
/// The stratum path encodes both layer navigation and the file.  The last
/// `Simple` token is the file; preceding tokens (`Down`, `Up`) are the layer.
/// Returns `(layer_fs_path, endpoint)` where `layer_fs_path` is relative to
/// the project root and is used both for bilink placement and as the git root.
fn parse_stratum_tip(root: &Path, tip_str: &str) -> anyhow::Result<(PathBuf, bilinker::link::LinkEndpoint)> {
    use bilinker::link::{LinkEndpoint, StructuralRef};
    use stratum::PathToken;

    // Extract optional :line:col suffix
    let parts: Vec<&str> = tip_str.rsplitn(3, ':').collect();
    let (path_str, pos) = if parts.len() == 3
        && parts[0].parse::<usize>().is_ok()
        && parts[1].parse::<usize>().is_ok()
    {
        let col:  usize = parts[0].parse()?;
        let line: usize = parts[1].parse()?;
        (parts[2], Some((line, col)))
    } else {
        (tip_str, None)
    };

    let tokens = stratum::parse_path(path_str)
        .map_err(|e| anyhow::anyhow!("invalid stratum path '{}': {}", path_str, e))?;

    // Last Simple token = file path; preceding tokens = layer navigation.
    let (layer_tokens, file_str) = match tokens.last() {
        Some(PathToken::Simple(p)) => {
            let layer = tokens[..tokens.len() - 1].to_vec();
            let file  = p.strip_prefix("/").unwrap_or(p).to_string_lossy().to_string();
            (layer, file)
        }
        _ => anyhow::bail!("tip must end with a file path, got: '{}'", path_str),
    };

    let layer_fs   = layer_tokens_to_fs_path(&layer_tokens)?;
    let layer_root = root.join(&layer_fs);

    let endpoint = if let Some((line, col)) = pos {
        let result = bilinker::capture::capture(&layer_root, &file_str, (line, col), (line, col))?;
        LinkEndpoint::Structural(result.endpoint)
    } else {
        LinkEndpoint::Structural(StructuralRef { file: file_str, query: None, range: None })
    };

    Ok((layer_fs, endpoint))
}

/// Converts layer navigation tokens (Up / Down only) to a filesystem path
/// relative to the project root. `[]` → `.` (current layer).
fn layer_tokens_to_fs_path(tokens: &[stratum::PathToken]) -> anyhow::Result<PathBuf> {
    use stratum::PathToken;
    if tokens.is_empty() {
        return Ok(PathBuf::from("."));
    }
    let mut path = PathBuf::new();
    for token in tokens {
        match token {
            PathToken::Down(name)  => path = path.join(".stratum").join(name),
            PathToken::Up          => path = path.join("..").join(".."),
            PathToken::TopRoot     => anyhow::bail!("`*` (TopRoot) not supported in chain new tips"),
            other => anyhow::bail!("unexpected token in layer navigation: {other:?}"),
        }
    }
    Ok(path)
}

fn parse_accept_target(target: &str) -> anyhow::Result<(String, u8)> {
    let dot = target.rfind('.')
        .ok_or_else(|| anyhow::anyhow!("target must be <uuid>.<0|1>, got: {target}"))?;
    let n: u8 = target[dot + 1..]
        .parse()
        .map_err(|_| anyhow::anyhow!("endpoint index must be 0 or 1, got: '{}'", &target[dot + 1..]))?;
    if n > 1 {
        anyhow::bail!("endpoint index must be 0 or 1, got: {n}");
    }
    Ok((target[..dot].to_string(), n))
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;

    match cli.command {
        Command::Capture { file, start, end } => {
            let root = project_root(&cwd)?;
            let result = bilinker::capture::capture(
                &root, &file,
                parse_pos(&start)?, parse_pos(&end)?,
            )?;
            println!("{}", result.endpoint);
            eprintln!("hash: {}", result.hash);
        }

        Command::Get { target, before, after } => {
            let uuid_form = {
                let t = target.trim();
                (t.ends_with(".0") || t.ends_with(".1")) && t.contains('-')
            };
            let pos_form = {
                let parts: Vec<&str> = target.rsplitn(3, ':').collect();
                parts.len() == 3
                    && parts[0].parse::<usize>().is_ok()
                    && parts[1].parse::<usize>().is_ok()
            };

            if uuid_form {
                let dot      = target.rfind('.').unwrap();
                let name     = &target[..dot];
                let endpoint: u8 = target[dot + 1..].parse()?;
                let root     = project_root(&cwd)?;
                let before   = before.as_deref().map(parse_pos).transpose()?;
                let after    = after.as_deref().map(parse_pos).transpose()?;
                let result   = bilinker::get::get(&root, name, endpoint, before, after)?;
                eprintln!("# {}  lines {}–{}", result.file, result.start_line, result.end_line);
                println!("{}", result.content);
            } else if pos_form {
                let mut parts = target.rsplitn(3, ':');
                let col:  usize = parts.next().unwrap().parse()?;
                let line: usize = parts.next().unwrap().parse()?;
                let file        = parts.next().unwrap();
                let file_path   = cwd.join(file);
                let root        = project_root(&cwd)?;

                let results = bilinker::check::find_by_file(&root, &file_path)?;
                if results.is_empty() {
                    return Ok(());
                }
                for (bilink_path, n, range) in results {
                    let source = std::fs::read_to_string(&file_path).unwrap_or_default();
                    let byte = line_col_to_byte(&source, line, col);
                    if byte >= range.start && byte < range.end {
                        let uuid  = bilink_path.file_stem()
                            .and_then(|s| s.to_str()).unwrap_or("?");
                        let bl    = bilinker::bilink::BiLinkFile::load(&bilink_path)?;
                        let other = if n == 0 { &bl.link1 } else { &bl.link0 };
                        println!("{uuid}.{n}  {other}");
                    }
                }
            } else {
                let file_path = cwd.join(&target);
                let root      = project_root(&cwd)?;

                let results = bilinker::check::find_by_file(&root, &file_path)?;
                for (bilink_path, n, range) in results {
                    let uuid  = bilink_path.file_stem()
                        .and_then(|s| s.to_str()).unwrap_or("?");
                    let bl    = bilinker::bilink::BiLinkFile::load(&bilink_path)?;
                    let other = if n == 0 { &bl.link1 } else { &bl.link0 };
                    println!("{uuid}.{n}  {other}  bytes {}–{}", range.start, range.end);
                }
            }
        }

        Command::Check { path } => {
            let root = project_root(&cwd)?;
            let check_path = if path.is_absolute() { path } else { cwd.join(path) };
            let results = bilinker::check::check(&root, &check_path)?;

            let mut exit_code = 0;
            for r in &results {
                if !r.is_clean() {
                    exit_code = 1;
                    println!("{}  ({}, {})", &r.uuid[..8], r.state0, r.state1);
                }
            }
            if exit_code == 0 {
                eprintln!("all clean ({} bilink(s))", results.len());
            }
            std::process::exit(exit_code);
        }

        Command::Watch => {
            let root = project_root(&cwd)?;
            watch(&root)?;
        }

        Command::Apply { dry_run, yes: _ } => {
            let pending = cwd.join(".bilink").join(".pending");
            if !pending.exists() {
                eprintln!("no pending fixes");
                std::process::exit(2);
            }
            if dry_run {
                eprintln!("apply --dry-run: not yet implemented");
            } else {
                eprintln!("apply: not yet implemented");
            }
        }

        Command::Index { sub } => match sub {
            IndexCommand::Build { path, recursive } => {
                let root = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                    .unwrap_or_else(|| cwd.clone());
                let layers = if recursive {
                    bilinker::index::layer_roots(&root)
                } else {
                    vec![root]
                };
                for layer in layers {
                    match bilinker::index::build(&layer) {
                        Ok(0) => {}
                        Ok(n) => {
                            let rel = layer.strip_prefix(&cwd).unwrap_or(&layer);
                            println!("index: {}/.bilink/.index  ({n} entries)", rel.display());
                        }
                        Err(e) => eprintln!("error building index for {}: {e}", layer.display()),
                    }
                }
            }

            IndexCommand::Status { path, recursive } => {
                let root = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                    .unwrap_or_else(|| cwd.clone());
                let layers = if recursive {
                    bilinker::index::layer_roots(&root)
                } else {
                    vec![root]
                };
                let mut any_problem = false;
                for layer in layers {
                    let rel = layer.strip_prefix(&cwd).unwrap_or(&layer);
                    match bilinker::index::status(&layer)? {
                        bilinker::index::IndexStatus::Ok =>
                            println!("{}/.bilink/.index  OK", rel.display()),
                        bilinker::index::IndexStatus::Stale { stale_count } => {
                            println!("{}/.bilink/.index  STALE  ({stale_count} bilink(s) newer)", rel.display());
                            any_problem = true;
                        }
                        bilinker::index::IndexStatus::Missing => {
                            println!("{}/.bilink/.index  MISSING", rel.display());
                            any_problem = true;
                        }
                    }
                }
                if any_problem { std::process::exit(1); }
            }
        },

        Command::Accept { target, hash, commit } => {
            // Dispatch: uuid.N  |  uuid (both endpoints)  |  path / "."
            let is_uuid_n = (target.ends_with(".0") || target.ends_with(".1"))
                && target[..target.len()-2].chars().all(|c| c.is_ascii_hexdigit() || c == '-');
            let is_path = target == "." || target.contains('/') || target.contains('\\')
                || std::path::Path::new(&target).exists();

            if is_uuid_n {
                // Single endpoint
                let (uuid, n) = parse_accept_target(&target)?;
                let bilink_path = bilinker::accept::find_bilink_path(&cwd.join(".bilink"), &uuid)?;
                let r = bilinker::accept::accept(&bilink_path, n, hash.as_deref(), commit.as_deref())?;
                print_accept_result(&r);
            } else if is_path {
                // Bulk: all PENDING under path filter
                let filter = if target == "." { None } else { Some(target.trim_end_matches('/')) };
                let results = bilinker::accept::accept_layer(&cwd, filter)?;
                if results.is_empty() {
                    eprintln!("nothing to accept");
                } else {
                    for r in &results {
                        print_accept_result(r);
                    }
                    eprintln!("accepted {} endpoint(s)", results.len());
                }
            } else {
                // UUID prefix: accept both endpoints
                let bilink_dir = cwd.join(".bilink");
                let bilink_path = bilinker::accept::find_bilink_path(&bilink_dir, &target)?;
                let mut count = 0;
                for n in [0u8, 1u8] {
                    match bilinker::accept::accept(&bilink_path, n, hash.as_deref(), commit.as_deref()) {
                        Ok(r) => { print_accept_result(&r); count += 1; }
                        Err(e) => eprintln!("warn .{n}: {e}"),
                    }
                }
                if count > 0 {
                    eprintln!("note: adjacent node will detect CHAIN_DIRTY on next check");
                }
            }
        }

        Command::Graph { selector, depth, format, recursive, bilink_detail, url_scheme, show_query, show_range, show_data } => {
            let root = project_root(&cwd)?;
            let detail = DetailOptions { show_query, show_range, show_data };
            cmd_graph(&root, &cwd, &selector, &format, depth, recursive, bilink_detail, &url_scheme, &detail)?;
        }

        Command::Status { path } => {
            let layer = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                .unwrap_or_else(|| cwd.clone());
            print_status(&layer)?;
        }

        Command::Chain { sub } => match sub {
            ChainCommand::New { tip, mid } => {
                if tip.len() != 2 {
                    anyhow::bail!("chain new requires exactly 2 --tip REF arguments");
                }
                let root = project_root(&cwd)?;
                let (layer0, ep0) = parse_stratum_tip(&root, &tip[0])?;
                let (layer1, ep1) = parse_stratum_tip(&root, &tip[1])?;
                let tips = vec![(layer0, ep0), (layer1, ep1)];
                let mids: Vec<PathBuf> = mid.iter().map(PathBuf::from).collect();

                let result = bilinker::chain::chain_new(&cwd, &tips, &mids)?;

                println!("Created chain: {}", result.uuid);
                println!();
                for f in &result.files {
                    let rel = f.strip_prefix(&cwd).unwrap_or(f);
                    println!("  {}", rel.display());
                }
                println!();
                eprintln!("Run 'bilinker check .' to populate cache.");
            }

            ChainCommand::Status { uuid } => {
                let root = project_root(&cwd)?;
                print_chain_status(&root, &uuid)?;
            }

            ChainCommand::List => {
                let root = project_root(&cwd)?;
                list_chains(&root)?;
            }
        },
    }
    Ok(())
}

fn print_chain_status(root: &Path, uuid: &str) -> anyhow::Result<()> {
    use bilinker::bilink::walkdir;

    let mut nodes: Vec<(std::path::PathBuf, bilinker::bilink::BiLinkFile)> = Vec::new();
    for entry in walkdir(root)? {
        let stem = entry.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem == uuid {
            if let Ok(bl) = bilinker::bilink::BiLinkFile::load(&entry) {
                nodes.push((entry, bl));
            }
        }
    }

    if nodes.is_empty() {
        anyhow::bail!("chain '{uuid}' not found");
    }

    let overall = chain_overall_state(&nodes);
    println!("Chain: {}  [{}]", uuid, overall);
    println!();

    for (path, bl) in &nodes {
        let layer = path.parent().and_then(|p| p.parent())
            .and_then(|p| p.strip_prefix(root).ok())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".to_string());

        let s0 = bilinker::state_str(&bl.state0);
        let s1 = bilinker::state_str(&bl.state1);

        println!("  {}/  ({s0}, {s1})", layer);
        println!("    link.0  {}", bl.link0);
        println!("    link.1  {}", bl.link1);
    }
    Ok(())
}

fn list_chains(root: &Path) -> anyhow::Result<()> {
    use bilinker::bilink::walkdir;
    use std::collections::HashMap;

    let mut chains: HashMap<String, Vec<bilinker::bilink::BiLinkFile>> = HashMap::new();

    for entry in walkdir(root)? {
        if entry.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
        if entry.ancestors().any(|a| a.ends_with(".pending")) { continue; }
        if let Ok(bl) = bilinker::bilink::BiLinkFile::load(&entry) {
            chains.entry(bl.uuid.clone()).or_default().push(bl);
        }
    }

    if chains.is_empty() {
        println!("(no chains found)");
        return Ok(());
    }

    let mut uuids: Vec<_> = chains.keys().cloned().collect();
    uuids.sort();

    for uuid in uuids {
        let nodes = &chains[&uuid];
        let overall = chain_overall_state_for_bl(nodes);
        println!("{}  [{}]  {} node(s)", &uuid[..8], overall, nodes.len());
    }
    Ok(())
}

fn chain_overall_state(nodes: &[(std::path::PathBuf, bilinker::bilink::BiLinkFile)]) -> &'static str {
    use bilinker::link::EndpointState::*;
    let terminal = |s: &Option<bilinker::link::EndpointState>| matches!(
        s, Some(Altered) | Some(Deleted) | Some(Unanchored) | Some(Broken)
    );
    let dirty = |s: &Option<bilinker::link::EndpointState>| matches!(s, Some(ChainDirty));
    for (_, bl) in nodes {
        if terminal(&bl.state0) || terminal(&bl.state1) { return "BROKEN"; }
    }
    for (_, bl) in nodes {
        if dirty(&bl.state0) || dirty(&bl.state1) { return "DIRTY"; }
    }
    "OK"
}

fn chain_overall_state_for_bl(nodes: &[bilinker::bilink::BiLinkFile]) -> &'static str {
    use bilinker::link::EndpointState::*;
    let terminal = |s: &Option<bilinker::link::EndpointState>| matches!(
        s, Some(Altered) | Some(Deleted) | Some(Unanchored) | Some(Broken)
    );
    let dirty = |s: &Option<bilinker::link::EndpointState>| matches!(s, Some(ChainDirty));
    for bl in nodes {
        if terminal(&bl.state0) || terminal(&bl.state1) { return "BROKEN"; }
    }
    for bl in nodes {
        if dirty(&bl.state0) || dirty(&bl.state1) { return "DIRTY"; }
    }
    "OK"
}

fn watch(root: &Path) -> anyhow::Result<()> {
    use notify::{EventKind, RecursiveMode, Watcher, recommended_watcher};
    use bilinker::bilink::{walkdir, BiLinkFile};
    use bilinker::link::LinkEndpoint;
    use std::sync::mpsc;

    eprintln!("watching {}  (Ctrl-C to stop)", root.display());

    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = recommended_watcher(tx)?;
    watcher.watch(root, RecursiveMode::Recursive)?;

    for res in rx {
        let event = match res {
            Ok(e)  => e,
            Err(e) => { eprintln!("watch error: {e}"); continue; }
        };

        if !matches!(event.kind, EventKind::Modify(_)) { continue; }

        'paths: for path in &event.paths {
            if path.components().any(|c| c.as_os_str() == ".bilink") { continue; }
            if !path.is_file() { continue; }

            let rel = match path.strip_prefix(root) {
                Ok(r)  => r.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            let mut chains: Vec<String> = Vec::new();
            for entry in walkdir(root).unwrap_or_default() {
                if entry.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
                if entry.components().any(|c| c.as_os_str() == ".pending") { continue; }
                let Ok(bl) = BiLinkFile::load(&entry) else { continue };

                let references_file = [&bl.link0, &bl.link1].iter().any(|link| {
                    if let LinkEndpoint::Structural(sref) = link {
                        rel.contains(&sref.file) || sref.file.contains(&rel)
                    } else {
                        false
                    }
                });

                if references_file {
                    chains.push(bl.uuid.clone());
                }
            }

            if !chains.is_empty() {
                for chain in &chains {
                    println!("ALTERED  {rel}  chain {chain}");
                }
            }

            break 'paths;
        }
    }
    Ok(())
}

fn print_accept_result(r: &bilinker::accept::AcceptResult) {
    let commit = if r.commit.is_empty() { "(uncommitted)".to_string() } else { r.commit[..12.min(r.commit.len())].to_string() };
    println!("  {}.{}  {}  {}", &r.uuid[..8.min(r.uuid.len())], r.n, &r.hash[..12.min(r.hash.len())], commit);
}

fn print_status(layer: &Path) -> anyhow::Result<()> {
    use std::collections::BTreeMap;
    use bilinker::bilink::BiLinkFile;
    use bilinker::link::LinkEndpoint;

    let bilink_dir = layer.join(".bilink");
    if !bilink_dir.exists() {
        eprintln!("no .bilink/ in {}", layer.display());
        return Ok(());
    }

    struct Row {
        file_name: String,
        uuid_short: String,
        s0: String,
        s1: String,
    }

    let mut groups: BTreeMap<String, Vec<Row>> = BTreeMap::new();

    for entry in std::fs::read_dir(&bilink_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
        if path.file_name().and_then(|n| n.to_str())
            .map(|n| n.starts_with('.')).unwrap_or(false) { continue; }

        let Ok(bl) = BiLinkFile::load(&path) else { continue };

        // Group by the structural endpoint's parent directory
        let (dir, file_name) = {
            let sref = match (&bl.link0, &bl.link1) {
                (LinkEndpoint::Structural(s), _) => Some(&s.file),
                (_, LinkEndpoint::Structural(s)) => Some(&s.file),
                _ => None,
            };
            match sref {
                Some(f) => {
                    let p = std::path::Path::new(f);
                    let dir = p.parent().and_then(|d| if d.as_os_str().is_empty() { None } else { Some(d) })
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| ".".to_string());
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| f.clone());
                    (dir, name)
                }
                None => ("(layer)".to_string(), bl.uuid.clone()),
            }
        };

        let uuid_short = bl.uuid[..8.min(bl.uuid.len())].to_string();
        let s0 = bilinker::state_str(&bl.state0);
        let s1 = bilinker::state_str(&bl.state1);

        groups.entry(dir).or_default().push(Row { file_name, uuid_short, s0, s1 });
    }

    if groups.is_empty() {
        println!("(no bilinks)");
        return Ok(());
    }

    for (dir, mut rows) in groups {
        println!("{dir}/");
        rows.sort_by(|a, b| a.file_name.cmp(&b.file_name));

        let max_name = rows.iter().map(|r| r.file_name.len()).max().unwrap_or(0);
        let mut prev = String::new();
        for row in &rows {
            let label = if row.file_name != prev {
                format!("{:<width$}", row.file_name, width = max_name)
            } else {
                format!("{:<width$}", "", width = max_name)
            };
            println!("  {}  {}  ({}, {})", label, row.uuid_short, row.s0, row.s1);
            prev = row.file_name.clone();
        }
        println!();
    }

    Ok(())
}

// ─── graph ────────────────────────────────────────────────────────────────────

struct DetailOptions {
    show_query: bool,
    show_range: bool,
    show_data: bool,
}

fn cmd_graph(root: &Path, cwd: &Path, selector: &str, format: &str, max_depth: Option<usize>, recursive: bool, bilink_detail: bool, url_scheme: &str, detail: &DetailOptions) -> anyhow::Result<()> {
    use std::collections::HashSet;

    let starts = find_graph_starts(root, cwd, selector, recursive)?;
    if starts.is_empty() {
        eprintln!("no bilinks found for '{selector}'");
        return Ok(());
    }

    let mut visited: HashSet<String> = HashSet::new();

    match format {
        "flat" => {
            for (bilink_path, layer_root) in &starts {
                let bl = bilinker::bilink::BiLinkFile::load(bilink_path)?;
                visited.insert(visit_key(&bl.uuid, layer_root));
                graph_flat(root, &bl, layer_root, &mut visited, 0, max_depth)?;
            }
        }
        "html" => {
            let mut hg = html_graph::HtmlGraph::new();
            for (bilink_path, layer_root) in &starts {
                let bl = bilinker::bilink::BiLinkFile::load(bilink_path)?;
                visited.insert(visit_key(&bl.uuid, layer_root));
                html_graph::collect(root, &bl, layer_root, &mut visited, &mut hg, url_scheme, 0, max_depth)?;
            }
            print!("{}", hg.emit());
        }
        "dot" => {
            let mut dot = DotGraph::new();
            for (bilink_path, layer_root) in &starts {
                let bl = bilinker::bilink::BiLinkFile::load(bilink_path)?;
                visited.insert(visit_key(&bl.uuid, layer_root));
                if bilink_detail {
                    collect_dot(root, &bl, layer_root, &mut visited, &mut dot, detail, url_scheme, 0, max_depth)?;
                } else {
                    collect_dot_simple(root, &bl, layer_root, &mut visited, &mut dot, detail, url_scheme, 0, max_depth)?;
                }
            }
            dot.emit();
        }
        _ => {
            println!("{selector}");
            if !starts.is_empty() { println!("│"); }
            for (i, (bilink_path, layer_root)) in starts.iter().enumerate() {
                let is_last = i == starts.len() - 1;
                let bl = bilinker::bilink::BiLinkFile::load(bilink_path)?;
                visited.insert(visit_key(&bl.uuid, layer_root));
                graph_tree(root, &bl, layer_root, "", is_last, &mut visited, 0, max_depth)?;
                if !is_last { println!("│"); }
            }
        }
    }
    Ok(())
}

fn find_graph_starts(root: &Path, cwd: &Path, selector: &str, recursive: bool) -> anyhow::Result<Vec<(PathBuf, PathBuf)>> {
    // "." or "*" → all bilinks in current layer (or all layers if --recursive)
    if selector == "." || selector == "*" {
        let layers = if recursive {
            bilinker::index::layer_roots(root)
        } else {
            vec![cwd.to_path_buf()]
        };
        let mut starts = vec![];
        for layer in layers {
            let bilink_dir = layer.join(".bilink");
            if !bilink_dir.exists() { continue; }
            for entry in std::fs::read_dir(&bilink_dir)? {
                let path = entry?.path();
                if path.extension().and_then(|e| e.to_str()) != Some("bilink") { continue; }
                if path.file_name().and_then(|n| n.to_str())
                    .map(|n| n.starts_with('.')).unwrap_or(false) { continue; }
                starts.push((path, layer.clone()));
            }
        }
        starts.sort_by(|a, b| a.0.cmp(&b.0));
        return Ok(starts);
    }

    // UUID or prefix → direct lookup in cwd's .bilink/
    let looks_like_uuid = selector.len() >= 8
        && !selector.contains('/')
        && !selector.contains('.')
        && selector.chars().all(|c| c.is_ascii_hexdigit() || c == '-');

    if looks_like_uuid {
        let bilink_path = bilinker::accept::find_bilink_path(&cwd.join(".bilink"), selector)?;
        return Ok(vec![(bilink_path, cwd.to_path_buf())]);
    }

    let file_str = selector.splitn(2, ':').next().unwrap_or(selector);
    let file_path = cwd.join(file_str);
    let results = bilinker::check::find_by_file(root, &file_path)?;

    Ok(results.into_iter().map(|(bilink_path, _n, _range)| {
        let layer_root = bilink_path.parent().and_then(|p| p.parent())
            .unwrap_or(cwd).to_path_buf();
        (bilink_path, layer_root)
    }).collect())
}

fn visit_key(uuid: &str, layer_root: &Path) -> String {
    format!("{}@{}", uuid, layer_root.display())
}

fn layer_children(bl: &bilinker::bilink::BiLinkFile, layer_root: &Path) -> Vec<(PathBuf, PathBuf)> {
    use bilinker::link::LinkEndpoint;
    let mut children = vec![];
    for endpoint in [&bl.link0, &bl.link1] {
        if let LinkEndpoint::Layer(tokens) = endpoint {
            if let Ok(adj) = stratum::resolve(layer_root, layer_root, tokens) {
                let adj_bilink = adj.join(".bilink").join(format!("{}.bilink", bl.uuid));
                if adj_bilink.exists() {
                    children.push((adj_bilink, adj));
                }
            }
        }
    }
    children
}

fn graph_tree(
    root: &Path,
    bl: &bilinker::bilink::BiLinkFile,
    layer_root: &Path,
    prefix: &str,
    is_last: bool,
    visited: &mut std::collections::HashSet<String>,
    depth: usize,
    max_depth: Option<usize>,
) -> anyhow::Result<()> {
    use bilinker::bilink::BiLinkFile;

    let conn = if is_last { "└── " } else { "├── " };
    let ext  = if is_last { "    " } else { "│   " };
    let child_prefix = format!("{prefix}{ext}");

    let uuid_short = &bl.uuid[..8.min(bl.uuid.len())];
    let s0 = bilinker::state_str(&bl.state0);
    let s1 = bilinker::state_str(&bl.state1);
    let layer_label = if depth > 0 {
        let rel = layer_root.strip_prefix(root).unwrap_or(layer_root);
        format!("  ({})", rel.display())
    } else {
        String::new()
    };

    println!("{prefix}{conn}{uuid_short}  [{s0} ↔ {s1}]{layer_label}");
    println!("{child_prefix}│  link.0  {}", bl.link0);
    println!("{child_prefix}│  link.1  {}", bl.link1);

    let children = if max_depth.map_or(true, |d| depth < d) {
        layer_children(bl, layer_root)
    } else {
        vec![]
    };

    if children.is_empty() {
        println!("{child_prefix}│");
    } else {
        println!("{child_prefix}│");
        for (i, (adj_bilink_path, adj_layer)) in children.iter().enumerate() {
            let key = visit_key(&bl.uuid, adj_layer);
            if visited.contains(&key) {
                let child_conn = if i == children.len() - 1 { "└── " } else { "├── " };
                println!("{child_prefix}{child_conn}{}  [ya visitado]", &bl.uuid[..8.min(bl.uuid.len())]);
                continue;
            }
            visited.insert(key);
            let adj_bl = BiLinkFile::load(adj_bilink_path)?;
            let child_is_last = i == children.len() - 1;
            graph_tree(root, &adj_bl, adj_layer, &child_prefix, child_is_last, visited, depth + 1, max_depth)?;
        }
    }
    Ok(())
}

fn graph_flat(
    root: &Path,
    bl: &bilinker::bilink::BiLinkFile,
    layer_root: &Path,
    visited: &mut std::collections::HashSet<String>,
    depth: usize,
    max_depth: Option<usize>,
) -> anyhow::Result<()> {
    use bilinker::bilink::BiLinkFile;

    let uuid_short = &bl.uuid[..8.min(bl.uuid.len())];
    let s0 = bilinker::state_str(&bl.state0);
    let s1 = bilinker::state_str(&bl.state1);
    let layer_label = {
        let rel = layer_root.strip_prefix(root).unwrap_or(layer_root);
        if rel.as_os_str().is_empty() { ".".to_string() } else { rel.display().to_string() }
    };

    println!("{uuid_short}  {s0} ↔ {s1}  {}  →  {}  [{}]",
        bl.link0, bl.link1, layer_label);

    if max_depth.map_or(true, |d| depth < d) {
        for (adj_bilink_path, adj_layer) in layer_children(bl, layer_root) {
            let key = visit_key(&bl.uuid, &adj_layer);
            if visited.contains(&key) { continue; }
            visited.insert(key);
            let adj_bl = BiLinkFile::load(&adj_bilink_path)?;
            graph_flat(root, &adj_bl, &adj_layer, visited, depth + 1, max_depth)?;
        }
    }
    Ok(())
}

struct DotGraph {
    // layer_label -> list of (node_id, node_def)
    layers: std::collections::BTreeMap<String, Vec<(String, String)>>,
    edges: Vec<String>,
    seen_nodes: std::collections::HashSet<String>,
    seen_edges: std::collections::HashSet<String>,
}

impl DotGraph {
    fn new() -> Self {
        Self {
            layers: std::collections::BTreeMap::new(),
            edges: Vec::new(),
            seen_nodes: std::collections::HashSet::new(),
            seen_edges: std::collections::HashSet::new(),
        }
    }

    fn add_node(&mut self, layer: &str, id: &str, def: &str) {
        if self.seen_nodes.insert(id.to_string()) {
            self.layers.entry(layer.to_string()).or_default()
                .push((id.to_string(), def.to_string()));
        }
    }

    fn add_edge(&mut self, from: &str, to: &str, label: &str, style: Option<&str>) {
        // Deduplicate bidirectional edges using canonical (min, max) key
        let key = if from <= to {
            format!("{from}↔{to}↔{label}")
        } else {
            format!("{to}↔{from}↔{label}")
        };
        if !self.seen_edges.insert(key) { return; }
        let style_attr = style.map(|s| format!(" style={s}")).unwrap_or_default();
        self.edges.push(format!(
            "  \"{from}\" -> \"{to}\" [label=\"{label}\" dir=both{style_attr}];"
        ));
    }

    fn emit(&self) {
        // Group layers by stratum depth
        let mut by_depth: std::collections::BTreeMap<usize, Vec<&String>> =
            std::collections::BTreeMap::new();
        for lbl in self.layers.keys() {
            let depth = if lbl == "." { 0 } else { lbl.matches(".stratum/").count() };
            by_depth.entry(depth).or_default().push(lbl);
        }
        let max_depth = by_depth.keys().max().copied().unwrap_or(0);

        println!("digraph bilinks {{");
        println!("  graph [rankdir=LR newrank=true];");
        println!("  node [fontname=\"monospace\"];");
        println!("  edge [fontname=\"monospace\" fontsize=10];");
        println!();

        // Invisible rank anchors to enforce column ordering
        for d in 0..=max_depth {
            println!("  __rank_{d} [style=invis width=0 height=0];");
        }
        for d in 0..max_depth {
            println!("  __rank_{d} -> __rank_{} [style=invis];", d + 1);
        }
        println!();

        // Clusters per layer
        for (i, (layer, nodes)) in self.layers.iter().enumerate() {
            println!("  subgraph cluster_{i} {{");
            println!("    label=\"{layer}\";");
            println!("    style=dashed;");
            println!("    color=gray;");
            for (_, def) in nodes {
                println!("    {def}");
            }
            println!("  }}");
            println!();
        }

        // rank=same groups: same depth → same column
        for (depth, labels) in &by_depth {
            let ids: Vec<String> = labels.iter()
                .flat_map(|lbl| self.layers[*lbl].iter().map(|(id, _)| format!("\"{id}\"")))
                .collect();
            println!("  {{ rank=same; __rank_{depth}; {} }}", ids.join("; "));
        }
        println!();

        for edge in &self.edges {
            println!("{edge}");
        }
        println!("}}");
    }
}

fn layer_label(root: &Path, layer_root: &Path) -> String {
    let rel = layer_root.strip_prefix(root).unwrap_or(layer_root);
    if rel.as_os_str().is_empty() { ".".to_string() } else { rel.display().to_string() }
}

fn node_url(layer_root: &Path, file: &str, range: Option<&bilinker::link::ByteRange>, scheme: &str) -> String {
    if scheme == "none" { return String::new(); }
    let abs = layer_root.join(file);
    let abs_str = abs.display().to_string();
    if scheme == "line" {
        if let Some(r) = range {
            if let Ok(content) = std::fs::read_to_string(&abs) {
                let line = content[..r.start.min(content.len())]
                    .chars().filter(|&c| c == '\n').count() + 1;
                return format!("file://{abs_str}#L{line}");
            }
        }
    }
    format!("file://{abs_str}")
}

fn add_structural_node(
    bl: &bilinker::bilink::BiLinkFile,
    layer_root: &std::path::Path,
    lbl: &str,
    dot: &mut DotGraph,
    detail: &DetailOptions,
    url_scheme: &str,
) -> Option<String> {
    use bilinker::link::LinkEndpoint;
    let (sref, range) = match (&bl.link0, &bl.link1) {
        (LinkEndpoint::Structural(s), _) => (s, bl.range0.as_ref()),
        (_, LinkEndpoint::Structural(s)) => (s, bl.range1.as_ref()),
        _ => return None,
    };
    // Compute start line to differentiate fragments of the same file
    let start_line = range.and_then(|r| {
        std::fs::read_to_string(layer_root.join(&sref.file)).ok().map(|c| {
            c[..r.start.min(c.len())].chars().filter(|&ch| ch == '\n').count() + 1
        })
    }).unwrap_or(1);
    let file_id  = format!("{}@{lbl}#L{start_line}", sref.file);
    let node_lbl = structural_node_label(layer_root, sref, range, detail);
    let url      = node_url(layer_root, &sref.file, range, url_scheme);
    let url_attr = if url.is_empty() { String::new() } else { format!(" URL=\"{url}\" target=\"_blank\"") };
    let file_def = format!("\"{file_id}\" [label=\"{node_lbl}\" shape=box{url_attr}];");
    dot.add_node(lbl, &file_id, &file_def);
    Some(file_id)
}

fn collect_dot_simple(
    root: &Path,
    bl: &bilinker::bilink::BiLinkFile,
    layer_root: &Path,
    visited: &mut std::collections::HashSet<String>,
    dot: &mut DotGraph,
    detail: &DetailOptions,
    url_scheme: &str,
    depth: usize,
    max_depth: Option<usize>,
) -> anyhow::Result<()> {
    use bilinker::bilink::BiLinkFile;

    let uuid_short = &bl.uuid[..8.min(bl.uuid.len())];
    let s0 = bilinker::state_str(&bl.state0);
    let s1 = bilinker::state_str(&bl.state1);
    let lbl = layer_label(root, layer_root);

    let local_id = add_structural_node(bl, layer_root, &lbl, dot, detail, url_scheme);

    if max_depth.map_or(true, |d| depth < d) {
        for (adj_bilink_path, adj_layer) in layer_children(bl, layer_root) {
            let key = visit_key(&bl.uuid, &adj_layer);
            let already = visited.contains(&key);
            if !already { visited.insert(key); }

            let adj_bl  = BiLinkFile::load(&adj_bilink_path)?;
            let adj_lbl = layer_label(root, &adj_layer);
            let adj_id  = add_structural_node(&adj_bl, &adj_layer, &adj_lbl, dot, detail, url_scheme);

            if let (Some(ref lid), Some(ref aid)) = (&local_id, &adj_id) {
                let edge_lbl = format!("{uuid_short}\\n{s0}↔{s1}");
                dot.add_edge(lid, aid, &edge_lbl, None);
            }

            if !already {
                collect_dot_simple(root, &adj_bl, &adj_layer, visited, dot, detail, url_scheme, depth + 1, max_depth)?;
            }
        }
    }
    Ok(())
}

fn structural_node_label(
    layer_root: &Path,
    sref: &bilinker::link::StructuralRef,
    range: Option<&bilinker::link::ByteRange>,
    detail: &DetailOptions,
) -> String {
    let mut parts = vec![sref.file.clone()];

    if detail.show_query {
        if let Some(q) = &sref.query {
            let short = q.split_whitespace().take(6).collect::<Vec<_>>().join(" ");
            let short = if q.split_whitespace().count() > 6 { format!("{short}…") } else { short };
            parts.push(short);
        }
    }

    if detail.show_range {
        if let Some(r) = range {
            parts.push(format!("bytes {}~{}", r.start, r.end));
        }
    }

    if detail.show_data {
        if let Some(r) = range {
            if let Ok(content) = std::fs::read_to_string(layer_root.join(&sref.file)) {
                let frag = content.get(r.start..r.end.min(content.len())).unwrap_or("");
                let mut non_empty = frag.lines().filter(|l| !l.trim().is_empty());
                if let Some(first) = non_empty.next() {
                    let first = first.trim();
                    let last  = frag.lines().filter(|l| !l.trim().is_empty()).last()
                                    .map(|l| l.trim()).unwrap_or(first);
                    if first == last {
                        parts.push(first.to_string());
                    } else {
                        parts.push(first.to_string());
                        parts.push("…".to_string());
                        parts.push(last.to_string());
                    }
                }
            }
        }
    }

    parts.join("\\l").replace('"', "'") + "\\l"
}

fn collect_dot(
    root: &Path,
    bl: &bilinker::bilink::BiLinkFile,
    layer_root: &Path,
    visited: &mut std::collections::HashSet<String>,
    dot: &mut DotGraph,
    detail: &DetailOptions,
    url_scheme: &str,
    depth: usize,
    max_depth: Option<usize>,
) -> anyhow::Result<()> {
    use bilinker::bilink::BiLinkFile;
    use bilinker::link::LinkEndpoint;

    let uuid_short = &bl.uuid[..8.min(bl.uuid.len())];
    let s0 = bilinker::state_str(&bl.state0);
    let s1 = bilinker::state_str(&bl.state1);
    let lbl = layer_label(root, layer_root);

    let bilink_id  = format!("{uuid_short}@{lbl}");
    let bilink_def = format!("\"{bilink_id}\" [label=\"{uuid_short}\\n{s0} ↔ {s1}\" shape=diamond];");
    dot.add_node(&lbl, &bilink_id, &bilink_def);

    for (n, endpoint) in [(&bl.link0, "0"), (&bl.link1, "1")] {
        match n {
            LinkEndpoint::Structural(sref) => {
                let range      = if endpoint == "0" { bl.range0.as_ref() } else { bl.range1.as_ref() };
                let file_id    = format!("{}@{lbl}", sref.file);
                let node_label = structural_node_label(layer_root, sref, range, detail);
                let url        = node_url(layer_root, &sref.file, range, url_scheme);
                let url_attr   = if url.is_empty() { String::new() } else { format!(" URL=\"{url}\" target=\"_blank\"") };
                let file_def   = format!("\"{file_id}\" [label=\"{node_label}\" shape=box{url_attr}];");
                dot.add_node(&lbl, &file_id, &file_def);
                dot.add_edge(&file_id, &bilink_id, &format!(".{endpoint}"), None);
            }
            LinkEndpoint::Layer(_) => {}
            LinkEndpoint::Task(id) => {
                let task_id  = format!("task:{id}@{lbl}");
                let task_def = format!("\"{task_id}\" [label=\"task {id}\" shape=note];");
                dot.add_node(&lbl, &task_id, &task_def);
                dot.add_edge(&task_id, &bilink_id, &format!(".{endpoint}"), None);
            }
        }
    }

    if max_depth.map_or(true, |d| depth < d) {
        for (adj_bilink_path, adj_layer) in layer_children(bl, layer_root) {
            let key = visit_key(&bl.uuid, &adj_layer);
            if visited.contains(&key) { continue; }
            visited.insert(key);
            let adj_bl = BiLinkFile::load(&adj_bilink_path)?;
            let adj_lbl = layer_label(root, &adj_layer);
            let adj_bilink_id = format!("{uuid_short}@{adj_lbl}");
            collect_dot(root, &adj_bl, &adj_layer, visited, dot, detail, url_scheme, depth + 1, max_depth)?;
            dot.add_edge(&bilink_id, &adj_bilink_id, "chain", Some("dashed"));
        }
    }
    Ok(())
}

fn line_col_to_byte(source: &str, line: usize, col: usize) -> usize {
    let mut cur_line = 1;
    let mut byte = 0;
    for (i, c) in source.char_indices() {
        if cur_line == line {
            return i + (col - 1).min(source.len() - i);
        }
        if c == '\n' { cur_line += 1; }
        byte = i;
    }
    byte
}
