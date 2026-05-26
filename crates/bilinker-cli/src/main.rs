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
}

#[derive(Subcommand)]
enum ChainCommand {
    /// Create a new chain or direct link
    ///
    /// Examples:
    ///   bilinker chain new --tip . spec/file.md --tip .estrato/impl src/file.rs
    ///   bilinker chain new --tip . spec/file.md --tip .estrato/impl src/Foo.java:42:5
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
            // Ignore writes to .bilink cache files
            if path.components().any(|c| c.as_os_str() == ".bilink") { continue; }
            if !path.is_file() { continue; }

            let rel = match path.strip_prefix(root) {
                Ok(r)  => r.to_string_lossy().to_string(),
                Err(_) => continue,
            };

            // Find every chain that references this file
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
                    println!("ALTERED  {rel}  chain {chain}.0  {chain}.1");
                }
            }

            break 'paths;
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
