
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
    /// Crea un capture a partir de una selección, y escribe el .capture
    ///
    /// FILE es relativo a la raíz de la capa. START y END son posiciones línea:col (1-based).
    /// Imprime el UUID del capture por stdout, listo para referenciar desde un link.N.
    #[command(args_conflicts_with_subcommands = true)]
    Capture {
        #[command(subcommand)]
        sub: Option<CaptureCommand>,
        file:  Option<String>,
        start: Option<String>,
        end:   Option<String>,
        /// Mostrar el capture que se crearía sin escribir nada
        #[arg(long)]
        dry_run: bool,
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
        /// Mostrar diff entre el fragmento aceptado y el actual
        #[arg(long)]
        diff: bool,
    },

    /// Repunta un endpoint a otro fragmento
    ///
    /// Para endpoints en UNANCHORED o REANCHORED sin fix automático: una sección
    /// renombrada, un test reescrito. Crea el capture, valida y repunta link.N.
    /// No acepta — correr `bilinker accept` después de revisar.
    Recapture {
        /// Endpoint a repuntar: UUID.N
        target: String,
        /// Archivo del fragmento nuevo, relativo a la capa
        file: String,
        /// Posición línea:col. Omitir para capturar el archivo completo
        pos: Option<String>,
        /// Fin de la selección línea:col. Default: igual que pos
        end: Option<String>,
    },

    /// Verify bilinks in a .bilink file or directory
    Check {
        /// Path a un .bilink o a una capa. Default: capa actual.
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Watch for changes in linked files and alert on drift
    Watch,

    /// Apply auto-fixes for bilinks in MOVED/DISPLACED/EXPANDED/REANCHORED state
    Apply {
        #[arg(long)]
        dry_run: bool,
        #[arg(short = 'y')]
        yes: bool,
        /// Only apply fixes of this state (e.g. --filter MOVED)
        #[arg(long, value_name = "ESTADO")]
        filter: Option<String>,
    },

    /// Migra los metadatos de bilinker al formato actual
    Migrate {
        /// Capa a migrar (default: directorio actual)
        path: Option<PathBuf>,
        /// Migrar también todas las capas descendientes
        #[arg(long)]
        recursive: bool,
        /// Mostrar qué haría sin escribir nada
        #[arg(long)]
        dry_run: bool,
        /// Ejecutar el corte: regenerar, verificar, y cambiar .bilink/ por lo migrado
        #[arg(long)]
        cut: bool,
        /// Deshacer el corte: restaurar .bilink/ desde el backup
        #[arg(long)]
        rollback: bool,
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
    /// Registra el estado actual de un endpoint como aprobado
    Accept {
        /// path, UUID, o UUID.N
        target: String,
        /// Aprueba sólo la ubicación: escribe accepted.link y deja el contenido
        #[arg(long)]
        place: bool,
        /// Aprueba sólo el contenido: escribe accepted.hash
        #[arg(long)]
        content: bool,
    },

    /// Show status of all bilinks in the current layer (like git status)
    Status {
        /// Layer directory to inspect (default: current directory)
        path: Option<PathBuf>,
    },

    /// Remove a bilink file from the current layer
    Remove {
        /// UUID or 8-char prefix of the bilink to remove
        uuid: String,
    },



    /// Traverse the bilink graph from a file, position, or UUID
    Graph {
        /// File, file:line:col, or UUID
        selector: String,
        /// Maximum traversal depth (default: unlimited)
        #[arg(long)]
        depth: Option<usize>,
        /// Output format: tree, flat, json
        #[arg(long, default_value = "tree", value_name = "FORMAT")]
        format: String,
        /// Collect bilinks from all layers under the project root
        #[arg(long)]
        recursive: bool,
    },
}

#[derive(Subcommand)]
enum CaptureCommand {
    /// Elimina los captures de la capa que ningún .bilink referencia
    Prune {
        path: Option<PathBuf>,
        #[arg(short = 'y')]
        yes: bool,
    },
    /// Elimina un capture puntual
    Remove {
        /// UUID o prefijo del capture
        uuid: String,
        /// Eliminarlo aunque tenga referentes
        #[arg(long)]
        force: bool,
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
    use bilinker::link::LinkEndpoint;
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

    let (uuid, _, _reused) = if let Some((line, col)) = pos {
        bilinker::capture::capture_to_file(&layer_root, &file_str, (line, col), (line, col))?
    } else {
        bilinker::capture::capture_file_whole(&layer_root, &file_str)?
    };
    let endpoint = LinkEndpoint::Capture(uuid);

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
        Command::Capture { sub, file, start, end, dry_run } => {
            match sub {
                Some(CaptureCommand::Remove { uuid, force }) => {
                    let dir = bilink_format::Capture::dir(&cwd);
                    let all = bilink_format::Capture::all_in(&cwd)?;
                    let hits: Vec<_> = all.iter().filter(|(id, _)| id.starts_with(&uuid)).collect();
                    let cap = match hits.as_slice() {
                        []  => anyhow::bail!("no hay capture que empiece con '{uuid}'"),
                        [c] => *c,
                        _   => anyhow::bail!("'{uuid}' es ambiguo: {} captures coinciden", hits.len()),
                    };

                    // Un capture con referentes deja bilinks apuntando a la nada.
                    let orphan = bilinker::capture::orphans(&cwd)?.iter().any(|(id, _)| *id == cap.0);
                    if !orphan && !force {
                        anyhow::bail!(
                            "el capture {} tiene referentes — usar `bilinker recapture` para repuntarlos, o --force",
                            &cap.0[..8.min(cap.0.len())]
                        );
                    }

                    std::fs::remove_file(dir.join(format!("{}.yaml", cap.0)))?;
                    eprintln!("eliminado: {}  {}", &cap.0[..8.min(cap.0.len())], cap.1.file);
                    if !orphan {
                        eprintln!("warn: tenía referentes — correr `bilinker check .`");
                    }
                }

                Some(CaptureCommand::Prune { path, yes }) => {
                    let layer = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                        .unwrap_or_else(|| cwd.clone());
                    let orphans = bilinker::capture::orphans(&layer)?;
                    if orphans.is_empty() {
                        eprintln!("no hay captures sin referentes");
                        return Ok(());
                    }
                    println!("{} capture(s) sin referentes:", orphans.len());
                    for c in &orphans {
                        // La query es multilínea; en una lista de confirmación
                        // previa a borrar archivos, una línea por capture.
                        let anchor = match c.1.query.as_deref() {
                            None => "archivo completo".to_string(),
                            Some(q) => {
                                let kind = q.split_whitespace().next().unwrap_or("")
                                    .trim_start_matches('(');
                                let name = q.split("#eq?").nth(1)
                                    .and_then(|t| t.split('"').nth(1)).unwrap_or("");
                                format!("{kind} {name}").trim().to_string()
                            }
                        };
                        println!("  {}…  {}  [{anchor}]",
                                 &c.0[..8.min(c.0.len())], c.1.file);
                    }
                    if !yes {
                        eprint!("
Eliminar? [y/N] ");
                        use std::io::Write;
                        std::io::stderr().flush().ok();
                        let mut input = String::new();
                        std::io::stdin().read_line(&mut input)?;
                        if input.trim().to_lowercase() != "y" {
                            eprintln!("abortado");
                            return Ok(());
                        }
                    }
                    for c in &orphans {
                        std::fs::remove_file(
                            bilink_format::Capture::path_in(&layer, &c.0))?;
                    }
                    eprintln!("eliminados {} capture(s)", orphans.len());
                }

                None => {
                    let (Some(file), Some(start), Some(end)) = (file, start, end) else {
                        anyhow::bail!("uso: bilinker capture <file> <start> <end>");
                    };
                    let root = project_root(&cwd)?;
                    let (s, e) = (parse_pos(&start)?, parse_pos(&end)?);

                    if dry_run {
                        let result = bilinker::capture::capture(&root, &file, s, e)?;
                        eprintln!("[dry-run] no se escribió nada");
                        eprintln!("file:   {}", result.capture.file);
                        if let Some(q) = &result.capture.query {
                            eprintln!("query:  {q}");
                        }
                        return Ok(());
                    }
                    let (uuid, path, reused) =
                        bilinker::capture::capture_to_file(&root, &file, s, e)?;
                    println!("{uuid}");
                    if reused {
                        eprintln!("reusado: {}", path.display());
                        eprintln!("  ya existía un capture con esta misma referencia");
                    } else {
                        eprintln!("creado: {}", path.display());
                    }
                }
            }
        }

        Command::Get { target, before, after, diff } => {
            let uuid_form = {
                let t = target.trim();
                if let Some(dot) = t.rfind('.') {
                    (t.ends_with(".0") || t.ends_with(".1"))
                        && t[..dot].chars().all(|c| c.is_ascii_hexdigit() || c == '-')
                        && !t[..dot].is_empty()
                } else {
                    false
                }
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

                if diff {
                    let result = bilinker::get::get_diff(&root, name, endpoint)?;
                    eprintln!("# {}  lines {}–{}", result.file, result.start_line, result.end_line);
                    match &result.diff {
                        Some(d) => print!("{d}"),
                        None    => eprintln!("[sin cambios]"),
                    }
                } else {
                    let before   = before.as_deref().map(parse_pos).transpose()?;
                    let after    = after.as_deref().map(parse_pos).transpose()?;
                    let result   = bilinker::get::get(&root, name, endpoint, before, after)?;
                    eprintln!("# {}  lines {}–{}", result.file, result.start_line, result.end_line);
                    println!("{}", result.content);

                }
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
                        let bl    = bilink_format::BiLink::load(&bilink_path)?;
                        let other = &bl.endpoint.get(1 - n).link;
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
                    let bl    = bilink_format::BiLink::load(&bilink_path)?;
                    let other = &bl.endpoint.get(1 - n).link;
                    println!("{uuid}.{n}  {other}  bytes {}–{}", range.start, range.end);
                }
            }
        }

        Command::Recapture { target, file, pos, end } => {
            let (uuid, n) = target.rsplit_once('.')
                .and_then(|(u, n)| n.parse::<u8>().ok().map(|n| (u, n)))
                .ok_or_else(|| anyhow::anyhow!("el target debe ser UUID.N, se recibió '{target}'"))?;
            if n > 1 { anyhow::bail!("el endpoint debe ser 0 o 1"); }

            let (bilink_path, _) =
                (bilinker::accept::find_bilink_path(&cwd, uuid)?, ());

            let range = match (pos.as_deref(), end.as_deref()) {
                (None, _)          => None,
                (Some(p), None)    => { let p = parse_pos(p)?; Some((p, p)) }
                (Some(p), Some(e)) => Some((parse_pos(p)?, parse_pos(e)?)),
            };
            let r = bilinker::capture::recapture(&cwd, &bilink_path, n, &file, range)?;

            println!("{}", r.new_uuid);
            eprintln!("link.{n} → capture {}{}",
                      &r.new_uuid[..8.min(r.new_uuid.len())],
                      if r.reused { "  (reusado)" } else { "" });
            if let Some(old) = &r.old_uuid {
                eprintln!("  antes: {}{}", &old[..8.min(old.len())],
                          if r.orphaned { "  (quedó sin referentes)" } else { "" });
            }
            eprintln!("\nrevisar con `bilinker get {target}` y aceptar con `bilinker accept {target}`");
        }

        Command::Check { path } => {
            let root = project_root(&cwd)?;
            let check_path = if path.is_absolute() { path } else { cwd.join(path) };
            let results = bilinker::check::check(&root, &check_path)?;

            // Se imprime todo lo que no está OK; solo falla lo que no tiene auto-fix.
            let mut exit_code = 0;
            let mut shown     = 0;
            for r in &results {
                if !r.all_ok() {
                    shown += 1;
                    println!("{}  ({}, {})", &r.uuid[..8], r.state0, r.state1);
                }
                if !r.is_clean() {
                    exit_code = 1;
                }
            }
            if shown == 0 {
                eprintln!("all clean ({} bilink(s))", results.len());
            }
            std::process::exit(exit_code);
        }

        Command::Watch => {
            let root = project_root(&cwd)?;
            watch(&root)?;
        }

        Command::Apply { dry_run, yes, filter } => {
            let root   = project_root(&cwd)?;
            let mut fixes = bilinker::apply::scan_fixeable(&cwd)?;

            if let Some(ref state) = filter {
                let state_up = state.to_uppercase();
                fixes.retain(|f| f.reason == state_up);
            }

            if fixes.is_empty() {
                eprintln!("no hay bilinks en estado auto-fixeable");
                std::process::exit(2);
            }

            // Collect state names for commit message summary
            let mut state_set: Vec<&str> = fixes.iter().map(|f| f.reason).collect();
            state_set.dedup();
            let states_label = state_set.join(" + ");

            // Print summary
            let max_state = fixes.iter().map(|f| f.reason.len()).max().unwrap_or(0);
            println!("Pending fixes ({}):", fixes.len());
            for f in &fixes {
                println!("  {:<width$}  {}…  link.{}  {}",
                    f.reason, f.short(), f.n,
                    f.description(),
                    width = max_state,
                );
            }

            if dry_run {
                return Ok(());
            }

            // Confirm
            if !yes {
                eprint!("\nApply? [y/N] ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                if input.trim().to_lowercase() != "y" {
                    eprintln!("abortado");
                    std::process::exit(1);
                }
            }

            // Apply each fix
            let mut applied: Vec<std::path::PathBuf> = Vec::new();
            let mut bullet_lines = Vec::new();
            let mut errors  = 0usize;

            for f in &fixes {
                match bilinker::apply::apply_fix(&cwd, f) {
                    Ok(paths) => {
                        applied.extend(paths);
                        bullet_lines.push(format!(
                            "- {}… endpoint.{}: {} {}",
                            f.short(), f.n, f.reason, f.description(),
                        ));
                    }
                    Err(e) => {
                        eprintln!("error  {}.{}: {e}", f.short(), f.n);
                        errors += 1;
                    }
                }
            }

            if applied.is_empty() {
                eprintln!("ningún fix aplicado");
                std::process::exit(if errors > 0 { 1 } else { 2 });
            }

            // Commit
            let date    = chrono::Utc::now().format("%Y-%m-%d");
            let summary = format!("bilinker: repuntar {states_label} ({date})");
            let body    = bullet_lines.join("\n");
            let message = format!("{summary}\n\n{body}");

            match git_commit(&root, &applied, &message) {
                Ok(hash) => {
                    let n = fixes.len() - errors;
                    println!("\nRepuntados {n} endpoint(s). Los {n} quedan en RELOCATED.");
                    println!("  Revisar con `bilinker get <uuid>.<N>` y aprobar con `bilinker accept --place`.");
                    let needs_accept: Vec<String> = Vec::new();
                    if !needs_accept.is_empty() {
                        println!("  {} requiere(n) `bilinker accept` — el contenido cambió: {}",
                                 needs_accept.len(), needs_accept.join(", "));
                    }
                    println!("Committed: {hash} \"{summary}\"");
                    if errors > 0 {
                        eprintln!("{errors} fix(es) fallaron — ejecutar 'bilinker check .' para ver el estado actual");
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("error al commitear: {e}");
                    std::process::exit(1);
                }
            }
        }

        Command::Migrate { path, recursive, dry_run, cut, rollback } => {
            let base = path.map(|p| if p.is_absolute() { p } else { cwd.join(p) })
                .unwrap_or_else(|| cwd.clone());
            let layers = if recursive {
                bilinker::index::layer_roots(&base)
            } else {
                vec![base.clone()]
            };

            // Las carpetas transitorias nunca se commitean.
            for layer in &layers {
                bilink_migrate::cut::exclude_in(&accreta_migrate::repo_root_of(layer))?;
            }

            if rollback {
                for layer in &layers {
                    bilink_migrate::cut::rollback(layer)?;
                    println!("  restaurado  {}", layer.display());
                }
                // El ledger vuelve atrás con los archivos: si quedara escrito, el
                // repo diría estar migrado con el formato viejo en disco.
                let repos: std::collections::BTreeSet<PathBuf> =
                    layers.iter().map(|l| accreta_migrate::repo_root_of(l)).collect();
                for repo in &repos {
                    accreta_migrate::forget(repo, &bilink_migrate::all())?;
                }
                eprintln!("\ncorte deshecho en {} capa(s).", layers.len());
                return Ok(());
            }

            if cut {
                if dry_run {
                    anyhow::bail!("--cut y --dry-run se excluyen: el corte escribe");
                }
                let mut cuts = Vec::new();
                // Se planifican **todas** antes de mover ninguna: si una capa no
                // verifica, no se corta nada. Un corte a medias deja el repo con
                // dos formatos y ningún binario que entienda los dos.
                for layer in &layers {
                    match bilink_migrate::cut::plan_cut(layer) {
                        Ok(c) => cuts.push(c),
                        Err(e) => anyhow::bail!("{layer:?}: {e}\n\nNo se cortó ninguna capa."),
                    }
                }
                for c in &cuts {
                    println!("  {}  {} bilink(s), {} capture(s)",
                             c.layer.display(), c.bilinks, c.captures);
                }
                for c in &cuts {
                    bilink_migrate::cut::execute(c)?;
                }
                // El ledger va acá: el estado recién ahora es verdadero.
                let written = accreta_migrate::record(&layers, &bilink_migrate::all())?;
                println!();
                for l in &written { println!("ledger: {}", l.display()); }
                eprintln!("\ncorte hecho en {} capa(s). Lo anterior queda en .bilink-formato-1/", cuts.len());
                eprintln!("Revisar con `bilinker check .` y commitear.");
                return Ok(());
            }

            let report = accreta_migrate::generate(&layers, &bilink_migrate::all(), dry_run)?;

            for id in &report.skipped {
                eprintln!("ya aplicada: {id}");
            }
            if report.is_noop() {
                eprintln!("nada que migrar ({} capa(s) revisada(s))", layers.len());
                return Ok(());
            }
            let mut last_repo: Option<&std::path::Path> = None;
            for a in &report.applied {
                if last_repo != Some(a.repo.as_path()) {
                    println!("\nrepo {}", a.repo.display());
                    last_repo = Some(a.repo.as_path());
                }
                println!("  {}{}", a.id, if dry_run { "  [dry-run]" } else { "" });
                for note in &a.notes {
                    println!("    {note}");
                }
                println!("    {} archivo(s) afectado(s)", a.changed.len());
            }
            if dry_run {
                eprintln!("\ndry-run: no se escribió nada");
            } else {
                eprintln!("\ngenerado. Revisar, y cortar con `bilinker migrate --cut`.");
                eprintln!("El ledger se escribe en el corte, no ahora.");
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

        Command::Accept { target, place, content } => {
            // Dispatch: uuid.N  |  uuid (both endpoints)  |  path / "."
            let is_uuid_n = (target.ends_with(".0") || target.ends_with(".1"))
                && target[..target.len()-2].chars().all(|c| c.is_ascii_hexdigit() || c == '-');
            let is_path = target == "." || target.contains('/') || target.contains('\\')
                || std::path::Path::new(&target).exists();

            // Qué dimensiones aprueba. Sin flags, las dos.
            let what = match (place, content) {
                (true, false) => bilinker::accept::What::place_only(),
                (false, true) => bilinker::accept::What::content_only(),
                _             => bilinker::accept::What::default(),
            };

            if is_uuid_n {
                // Un endpoint
                let (uuid, n) = parse_accept_target(&target)?;
                let r = bilinker::accept::accept(&cwd, &uuid, n, what)?;
                print_accept_result(&r);
            } else if is_path {
                // Bulk: all PENDING under path filter
                let filter = if target == "." { None } else { Some(target.trim_end_matches('/')) };
                let _ = filter;
                let results = bilinker::accept::accept_all(&cwd)?;
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
                let mut count = 0;
                for n in [0u8, 1u8] {
                    match bilinker::accept::accept(&cwd, &target, n, what) {
                        Ok(r) => { print_accept_result(&r); count += 1; }
                        Err(e) => eprintln!("warn .{n}: {e}"),
                    }
                }
                if count > 0 {
                    eprintln!("note: adjacent node will detect CHAIN_DIRTY on next check");
                }
            }
        }

        Command::Remove { uuid } => {
            let bilink_dir = cwd.join(".bilink");
            let path = bilinker::accept::find_bilink_path(&bilink_dir, &uuid)?;
            std::fs::remove_file(&path)?;
            let rel = path.strip_prefix(&cwd).unwrap_or(&path);
            eprintln!("removed: {}", rel.display());
            eprintln!("note: nodos adyacentes detectarán BROKEN en el próximo check");
        }


        Command::Graph { selector, depth, format, recursive } => {
            let root = project_root(&cwd)?;
            cmd_graph(&root, &cwd, &selector, &format, depth, recursive)?;
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

/// El estado de una cadena, recorriendo sus nodos.
///
/// Los estados salen de la cache de cada capa: no están en los archivos. Con la
/// cache fría se dice, en vez de inventar un OK.
fn print_chain_status(root: &Path, uuid: &str) -> anyhow::Result<()> {
    let mut nodes: Vec<(PathBuf, bilink_format::BiLink)> = Vec::new();
    for (layer, _) in layers_with(root, uuid) {
        let path = bilink_format::BiLink::path_in(&layer, uuid);
        if let Ok(bl) = bilink_format::BiLink::load(&path) {
            nodes.push((layer, bl));
        }
    }
    if nodes.is_empty() {
        anyhow::bail!("no existe la cadena '{uuid}'");
    }

    println!("Cadena: {uuid}  [{}]", chain_overall_state(root, uuid, &nodes));
    println!();
    for (layer, bl) in &nodes {
        let cache = bilinker::cache::Cache::load(layer);
        let label = layer.strip_prefix(root).ok()
            .map(|p| if p.as_os_str().is_empty() { ".".into() } else { p.display().to_string() })
            .unwrap_or_else(|| layer.display().to_string());

        println!("  {label}/  ({}, {})", state_label(&cache, uuid, 0), state_label(&cache, uuid, 1));
        println!("    endpoint.0  {}", bl.endpoint.zero.link);
        println!("    endpoint.1  {}", bl.endpoint.one.link);
    }
    Ok(())
}

fn state_label(cache: &bilinker::cache::Cache, uuid: &str, n: u8) -> String {
    cache.endpoint_state(uuid, n).map(|s| s.to_string()).unwrap_or_else(|| "—".into())
}

/// Las capas que tienen un bilink con este uuid.
fn layers_with(root: &Path, uuid: &str) -> Vec<(PathBuf, PathBuf)> {
    bilinker::index::layer_roots(root).into_iter()
        .map(|l| (l.clone(), bilink_format::BiLink::path_in(&l, uuid)))
        .filter(|(_, p)| p.exists())
        .collect()
}

fn list_chains(root: &Path) -> anyhow::Result<()> {
    use std::collections::BTreeMap;
    let mut chains: BTreeMap<String, usize> = BTreeMap::new();

    for layer in bilinker::index::layer_roots(root) {
        for path in bilink_format::bilink::bilink_files(&layer.join(".bilink")) {
            if let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) {
                *chains.entry(uuid.to_string()).or_default() += 1;
            }
        }
    }
    if chains.is_empty() {
        println!("(no hay cadenas)");
        return Ok(());
    }
    for (uuid, n) in chains {
        let nodes: Vec<(PathBuf, bilink_format::BiLink)> = layers_with(root, &uuid).into_iter()
            .filter_map(|(l, p)| bilink_format::BiLink::load(&p).ok().map(|bl| (l, bl)))
            .collect();
        println!("{}  [{}]  {n} nodo(s)", &uuid[..8.min(uuid.len())],
                 chain_overall_state(root, &uuid, &nodes));
    }
    Ok(())
}

/// El peor estado de la cadena.
fn chain_overall_state(_root: &Path, uuid: &str, nodes: &[(PathBuf, bilink_format::BiLink)]) -> &'static str {
    use bilinker::state::EndpointState::*;
    let mut seen = Vec::new();
    for (layer, _) in nodes {
        let cache = bilinker::cache::Cache::load(layer);
        for n in [0u8, 1u8] {
            if let Some(s) = cache.endpoint_state(uuid, n) { seen.push(s); }
        }
    }
    if seen.is_empty() { return "—"; }
    if seen.iter().any(|s| matches!(s, Altered | Unresolved | Broken)) { return "BROKEN"; }
    if seen.iter().any(|s| matches!(s, ChainDirty)) { return "DIRTY"; }
    if seen.iter().any(|s| !s.is_ok()) { return "PENDIENTE"; }
    "OK"
}

fn watch(root: &Path) -> anyhow::Result<()> {
    use notify::{EventKind, RecursiveMode, Watcher, recommended_watcher};
    use bilink_format::bilink::bilink_files;
    
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
            for entry in bilink_files(&root.join(".bilink")) {
                let Ok(bl) = bilink_format::BiLink::load(&entry) else { continue };
                let Some(uuid) = entry.file_stem().and_then(|s| s.to_str()) else { continue };

                let references_file = (0..2u8).any(|n| {
                    match bilinker::capture::capture_of(root, &bl.endpoint.get(n).link) {
                        Ok(Some(cap)) => rel.contains(&cap.file) || cap.file.contains(&rel),
                        _ => false,
                    }
                });
                if references_file { chains.push(uuid.to_string()); }
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
    let commit = match &r.commit {
        Some(c) => c[..12.min(c.len())].to_string(),
        None    => "(sin commit)".to_string(),
    };
    println!("  {}.{}  {}  {}", &r.uuid[..8.min(r.uuid.len())], r.n, &r.hash[..12.min(r.hash.len())], commit);
}

/// El estado de la capa, agrupado por archivo.
///
/// Lee la cache; no resuelve nada. Con la cache fría **no hay estados** y se dice:
/// mostrar OK sin haber verificado sería peor que no mostrar nada.
fn print_status(layer: &Path) -> anyhow::Result<()> {
    use std::collections::BTreeMap;
    use bilink_format::{BiLink, Capture};

    let bilink_dir = layer.join(".bilink");
    if !bilink_dir.exists() {
        eprintln!("no hay .bilink/ en {}", layer.display());
        return Ok(());
    }

    let cache = bilinker::cache::Cache::load(layer);
    let mut cold = true;

    struct Row { file_name: String, uuid_short: String, s0: String, s1: String }
    let mut groups: BTreeMap<String, Vec<Row>> = BTreeMap::new();

    for path in bilink_format::bilink::bilink_files(&bilink_dir) {
        let Ok(bl) = BiLink::load(&path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };

        // Se agrupa por el directorio del endpoint estructural.
        let file = (0..2u8)
            .filter_map(|n| bl.endpoint.get(n).link.capture_id())
            .filter_map(|id| Capture::load_in(layer, id).ok())
            .map(|c| c.file)
            .next();

        let (dir, file_name) = match file.as_ref() {
            Some(f) => {
                let p = std::path::Path::new(f);
                let dir = p.parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|| ".".into());
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| f.clone());
                (dir, name)
            }
            None => ("(layer)".into(), uuid.to_string()),
        };

        let label = |n: u8| match cache.endpoint_state(uuid, n) {
            Some(st) => { st.to_string() }
            None     => "—".to_string(),
        };
        let (s0, s1) = (label(0), label(1));
        if cache.endpoint_state(uuid, 0).is_some() { cold = false; }

        groups.entry(dir).or_default().push(Row {
            file_name, uuid_short: uuid[..8.min(uuid.len())].to_string(), s0, s1,
        });
    }

    if groups.is_empty() {
        println!("(no hay bilinks)");
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

    if cold {
        eprintln!("sin estados: la cache está fría.");
        eprintln!("  Correr `bilinker check .` para calcularlos.");
    }
    Ok(())
}

// ─── graph ────────────────────────────────────────────────────────────────────

fn cmd_graph(root: &Path, cwd: &Path, selector: &str, format: &str, max_depth: Option<usize>, recursive: bool) -> anyhow::Result<()> {
    use std::collections::HashSet;

    let starts = find_graph_starts(root, cwd, selector, recursive)?;
    if starts.is_empty() {
        eprintln!("no bilinks found for '{selector}'");
        return Ok(());
    }

    let mut visited: HashSet<String> = HashSet::new();

    match format {
        "json" => graph_json(root, &starts)?,
        "flat" => {
            for (bilink_path, layer_root) in &starts {
                let bl = bilink_format::BiLink::load(bilink_path)?;
                let uuid = uuid_of(bilink_path);
                visited.insert(visit_key(&uuid, layer_root));
                graph_flat(root, &bl, &uuid, layer_root, &mut visited, 0, max_depth)?;
            }
        }
        _ => {
            println!("{selector}");
            if !starts.is_empty() { println!("│"); }
            for (i, (bilink_path, layer_root)) in starts.iter().enumerate() {
                let is_last = i == starts.len() - 1;
                let bl = bilink_format::BiLink::load(bilink_path)?;
                let uuid = uuid_of(bilink_path);
                visited.insert(visit_key(&uuid, layer_root));
                graph_tree(root, &bl, &uuid, layer_root, "", is_last, &mut visited, 0, max_depth)?;
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

// ─── formato json: contrato de proveedor hacia lattice ───────────────────────

/// La raíz más externa del ecosistema que contiene a `start`.
///
/// La forma canónica de un nodo tiene que ser la misma sin importar desde qué
/// capa se invoque: una cadena que sube a la capa de specs no puede identificar
/// al mismo fragmento distinto según se corra desde impl o desde la raíz. Por eso
/// el label se calcula contra el ancestro más externo que sea repo o capa, y no
/// contra el directorio de invocación.
fn outermost_root(start: &Path) -> PathBuf {
    let mut best = start.to_path_buf();
    let mut cur  = start;
    while let Some(parent) = cur.parent() {
        if parent.join(".git").exists() || parent.join(".bilink").exists() {
            best = parent.to_path_buf();
        }
        cur = parent;
    }
    best
}

/// Un extremo estructural de una cadena, en forma canónica de lattice.
struct TipNode {
    canonical: String,
    state:     String,
    commit:    String,
}

/// Recorre la cadena de `bl` y devuelve sus extremos estructurales.
///
/// Los nodos intermedios son mecanismo interno de bilinker: si lattice los
/// viera, el grafo se llenaría de nodos `.bilink` que no son contenido del
/// proyecto. Por eso una cadena de N nodos emite **una** arista entre sus tips.
fn chain_tips(base: &Path, uuid: &str, layer_root: &Path) -> Vec<TipNode> {
    let mut tips    = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut queue   = vec![layer_root.to_path_buf()];

    while let Some(layer) = queue.pop() {
        if !visited.insert(visit_key(uuid, &layer)) { continue; }
        let Ok(node) = bilink_format::BiLink::load(&bilink_format::BiLink::path_in(&layer, uuid)) else { continue };
        let cache = bilinker::cache::Cache::load(&layer);

        for n in [0u8, 1u8] {
            let Some(id) = node.endpoint.get(n).link.capture_id() else { continue };
            let Ok(cap) = bilink_format::Capture::load_in(&layer, id) else { continue };
            // El rango sale de la cache. Con cache fría no hay nodo canónico que
            // emitir: lattice necesita `check` corrido antes de consultar.
            let Some(range) = cache.capture_range(id) else { continue };
            tips.push(TipNode {
                canonical: format!("{}::{}#{}~{}",
                    layer_label(base, &layer), cap.file, range.start, range.end),
                state:  cache.endpoint_state(uuid, n).map(|s| s.to_string()).unwrap_or_else(|| "—".into()),
                commit: cache.commit(uuid, n).unwrap_or("").to_string(),
            });
        }

        for (_, adj_layer) in layer_children(&node, uuid, &layer) {
            queue.push(adj_layer);
        }
    }
    tips
}

/// Emite las aristas de bilinker en el modelo de lattice.
fn graph_json(root: &Path, starts: &[(PathBuf, PathBuf)]) -> anyhow::Result<()> {
    use bilink_format::LinkEndpoint;
    let base     = outermost_root(root);
    let mut out  = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (path, layer_root) in starts {
        let Ok(bl) = bilink_format::BiLink::load(path) else { continue };
        let Some(uuid) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if !seen.insert(uuid.to_string()) { continue; }

        // El `kind` sale de la semántica declarada; sin `kind`, es un bilink.
        let kind = match (&bl.endpoint.zero.link, &bl.endpoint.one.link) {
            (LinkEndpoint::Issue(_), _) | (_, LinkEndpoint::Issue(_)) => "issue",
            _ => "bilink",
        };

        let tips = chain_tips(&base, uuid, layer_root);
        if tips.len() < 2 { continue; }
        let (a, b) = (&tips[0], &tips[1]);

        out.push(format!(
            r#"  {{"from":"{}","to":"{}","kind":"{}","guarantee":"accepted","provider":"bilinker","directed":false,"ref":"{}","state":["{}","{}"],"commit":["{}","{}"]}}"#,
            esc_json(&a.canonical), esc_json(&b.canonical),
            kind, uuid, a.state, b.state, a.commit, b.commit,
        ));
    }

    println!("[\n{}\n]", out.join(",\n"));
    Ok(())
}

fn layer_label(root: &Path, layer_root: &Path) -> String {
    let rel = layer_root.strip_prefix(root).unwrap_or(layer_root);
    if rel.as_os_str().is_empty() { ".".to_string() } else { rel.display().to_string() }
}

/// Escapa una cadena para embeberla en JSON.
fn esc_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
     .replace('\n', "\\n").replace('\r', "").replace('\t', "\\t")
}

/// El uuid de un bilink es el nombre de su archivo.
fn uuid_of(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string()
}

fn visit_key(uuid: &str, layer_root: &Path) -> String {
    format!("{}@{}", uuid, layer_root.display())
}

fn layer_children(bl: &bilink_format::BiLink, uuid: &str, layer_root: &Path) -> Vec<(PathBuf, PathBuf)> {
    use bilink_format::LinkEndpoint;
    let mut children = vec![];
    for n in [0u8, 1u8] {
        if let LinkEndpoint::Path(p) = &bl.endpoint.get(n).link {
            if let Ok(adj) = stratum::resolve(layer_root, layer_root, p.tokens()) {
                // Subir a la raíz verdadera de la capa vecina (.git o .bilink)
                let true_adj = bilinker::config::Config::load_from(&adj)
                    .map(|(r, _)| r)
                    .unwrap_or(adj);
                let adj_bilink = bilink_format::BiLink::path_in(&true_adj, uuid);
                if adj_bilink.exists() {
                    children.push((adj_bilink, true_adj));
                }
            }
        }
    }
    children
}

fn graph_tree(
    root: &Path,
    bl: &bilink_format::BiLink,
    uuid: &str,
    layer_root: &Path,
    prefix: &str,
    is_last: bool,
    visited: &mut std::collections::HashSet<String>,
    depth: usize,
    max_depth: Option<usize>,
) -> anyhow::Result<()> {
    let conn = if is_last { "└── " } else { "├── " };
    let ext  = if is_last { "    " } else { "│   " };
    let child_prefix = format!("{prefix}{ext}");

    let uuid_short = &uuid[..8.min(uuid.len())];
    let cache = bilinker::cache::Cache::load(layer_root);
    let st = |n: u8| cache.endpoint_state(uuid, n).map(|s| s.to_string()).unwrap_or_else(|| "—".into());
    let (s0, s1) = (st(0), st(1));
    let layer_label = if depth > 0 {
        let rel = layer_root.strip_prefix(root).unwrap_or(layer_root);
        format!("  ({})", rel.display())
    } else {
        String::new()
    };

    println!("{prefix}{conn}{uuid_short}  [{s0} ↔ {s1}]{layer_label}");
    println!("{child_prefix}│  endpoint.0  {}", bl.endpoint.zero.link);
    println!("{child_prefix}│  endpoint.1  {}", bl.endpoint.one.link);

    let children = if max_depth.map_or(true, |d| depth < d) {
        layer_children(bl, uuid, layer_root)
    } else {
        vec![]
    };

    if children.is_empty() {
        println!("{child_prefix}│");
    } else {
        println!("{child_prefix}│");
        for (i, (adj_bilink_path, adj_layer)) in children.iter().enumerate() {
            let key = visit_key(uuid, adj_layer);
            if visited.contains(&key) {
                let child_conn = if i == children.len() - 1 { "└── " } else { "├── " };
                println!("{child_prefix}{child_conn}{uuid_short}  [ya visitado]");
                continue;
            }
            visited.insert(key);
            let adj_bl = bilink_format::BiLink::load(adj_bilink_path)?;
            let child_is_last = i == children.len() - 1;
            graph_tree(root, &adj_bl, uuid, adj_layer, &child_prefix, child_is_last, visited, depth + 1, max_depth)?;
        }
    }
    Ok(())
}

fn graph_flat(
    root: &Path,
    bl: &bilink_format::BiLink,
    uuid: &str,
    layer_root: &Path,
    visited: &mut std::collections::HashSet<String>,
    depth: usize,
    max_depth: Option<usize>,
) -> anyhow::Result<()> {

    let uuid_short = &uuid[..8.min(uuid.len())];
    let cache = bilinker::cache::Cache::load(layer_root);
    let st = |n: u8| cache.endpoint_state(uuid, n).map(|s| s.to_string()).unwrap_or_else(|| "—".into());
    let (s0, s1) = (st(0), st(1));
    let layer_label = {
        let rel = layer_root.strip_prefix(root).unwrap_or(layer_root);
        if rel.as_os_str().is_empty() { ".".to_string() } else { rel.display().to_string() }
    };

    println!("{uuid_short}  {s0} ↔ {s1}  {}  →  {}  [{}]",
        bl.endpoint.zero.link, bl.endpoint.one.link, layer_label);

    if max_depth.map_or(true, |d| depth < d) {
        for (adj_bilink_path, adj_layer) in layer_children(bl, uuid, layer_root) {
            let key = visit_key(uuid, &adj_layer);
            if visited.contains(&key) { continue; }
            visited.insert(key);
            let adj_bl = bilink_format::BiLink::load(&adj_bilink_path)?;
            graph_flat(root, &adj_bl, uuid, &adj_layer, visited, depth + 1, max_depth)?;
        }
    }
    Ok(())
}

// ─── impact ───────────────────────────────────────────────────────────────────

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

/// Stagea y commitea los archivos escritos. Devuelve el hash corto.
fn git_commit(root: &Path, paths: &[PathBuf], message: &str) -> anyhow::Result<String> {
    for path in paths {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let st = std::process::Command::new("git")
            .args(["add", &rel.display().to_string()])
            .current_dir(root)
            .status()?;
        if !st.success() {
            anyhow::bail!("git add falló para {}", path.display());
        }
    }
    let out = std::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(root)
        .output()?;
    if !out.status.success() {
        anyhow::bail!("git commit falló:\n{}", String::from_utf8_lossy(&out.stderr));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout.lines().next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or("?")
        .to_string())
}
