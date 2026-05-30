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
}

#[derive(Subcommand)]
enum ChainCommand {
    /// Create a new chain or direct link
    ///
    /// Examples:
    ///   bilinker chain new --tip . spec/file.md --tip .stratum/impl src/file.rs
    ///   bilinker chain new --tip . spec/file.md --tip .stratum/impl src/Foo.java:42:5
    New {
        /// Tip: LAYER FILE[:LINE:COL]  (specify exactly twice)
        #[arg(long = "tip", num_args = 2, value_names = ["LAYER", "FILE"], action = ArgAction::Append)]
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

/// Parse a tip REF string: `path/to/file` or `path/to/file:line:col`.
/// Returns a structural endpoint (whole-file or AST-anchored).
fn parse_tip_ref(root: &Path, ref_str: &str) -> anyhow::Result<bilinker::link::LinkEndpoint> {
    use bilinker::link::{LinkEndpoint, StructuralRef};

    // Try to split off :line:col suffix
    let parts: Vec<&str> = ref_str.rsplitn(3, ':').collect();
    if parts.len() == 3
        && parts[0].parse::<usize>().is_ok()
        && parts[1].parse::<usize>().is_ok()
    {
        let col:  usize = parts[0].parse()?;
        let line: usize = parts[1].parse()?;
        let file        = parts[2];
        let result = bilinker::capture::capture(root, file, (line, col), (line, col))?;
        Ok(LinkEndpoint::Structural(result.endpoint))
    } else {
        Ok(LinkEndpoint::Structural(StructuralRef {
            file: ref_str.to_string(),
            query: None,
            range: None,
        }))
    }
}

fn project_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let (root, _) = bilinker::config::Config::load_from(cwd)?;
    Ok(root)
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

        Command::Status { path } => {
            let layer = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                .unwrap_or_else(|| cwd.clone());
            print_status(&layer)?;
        }

        Command::Chain { sub } => match sub {
            ChainCommand::New { tip, mid } => {
                if tip.len() != 4 {
                    anyhow::bail!("chain new requires exactly 2 --tip LAYER FILE pairs");
                }
                let root = project_root(&cwd)?;
                let tips = vec![
                    (PathBuf::from(&tip[0]), parse_tip_ref(&root, &tip[1])?),
                    (PathBuf::from(&tip[2]), parse_tip_ref(&root, &tip[3])?),
                ];
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

        let s0 = bl.state0.as_ref().map(|s| s.to_string()).unwrap_or_else(|| "?".into());
        let s1 = bl.state1.as_ref().map(|s| s.to_string()).unwrap_or_else(|| "?".into());

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
        let s0 = bl.state0.as_ref().map(|s| s.to_string()).unwrap_or_else(|| "-".to_string());
        let s1 = bl.state1.as_ref().map(|s| s.to_string()).unwrap_or_else(|| "-".to_string());

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
