use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "bilinker", about = "Universal bidirectional structural references")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Capture a bilink endpoint from a file selection
    Capture {
        /// Workspace name (defined in .bilinker.toml)
        workspace: String,
        /// File path relative to the workspace root
        file: String,
        /// Start position as line:col (1-based)
        start: String,
        /// End position as line:col (1-based)
        end: String,
    },
    /// Print the code referenced by a bilink endpoint
    Get {
        /// Bilink id (as declared in the .bilink file)
        name: String,
        /// Endpoint to resolve: 0 or 1
        endpoint: u8,
        /// Lines of context before the fragment
        #[arg(short = 'B', value_name = "rows:cols")]
        before: Option<String>,
        /// Lines of context after the fragment
        #[arg(short = 'A', value_name = "rows:cols")]
        after: Option<String>,
    },
    /// Verify all bilinks in a .bilink file or .bilinker/ directory
    Check {
        path: PathBuf,
    },
    /// Show all bilinks that reference a given file position
    Refs {
        /// file:line
        location: String,
    },
}

fn parse_pos(s: &str) -> anyhow::Result<(usize, usize)> {
    let (line, col) = s.split_once(':')
        .ok_or_else(|| anyhow::anyhow!("position must be line:col, got: {s}"))?;
    Ok((line.parse()?, col.parse()?))
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;

    match cli.command {
        Command::Capture { workspace, file, start, end } => {
            let (root, config) = bilinker::config::Config::load_from(&cwd)?;
            let result = bilinker::capture::capture(
                &config, &root, &workspace, &file,
                parse_pos(&start)?, parse_pos(&end)?,
            )?;
            println!("{}", result.link);
            eprintln!("hash: {}", result.hash);
        }

        Command::Get { name, endpoint, before, after } => {
            let (root, config) = bilinker::config::Config::load_from(&cwd)?;
            let before = before.as_deref().map(parse_pos).transpose()?;
            let after  = after.as_deref().map(parse_pos).transpose()?;
            let result = bilinker::get::get(&config, &root, &name, endpoint, before, after)?;
            eprintln!("# {}  lines {}–{}", result.file, result.start_line, result.end_line);
            println!("{}", result.content);
        }

        Command::Check { path } => {
            eprintln!("check: {path:?} (not yet implemented)");
        }

        Command::Refs { location } => {
            eprintln!("refs: {location} (not yet implemented)");
        }
    }
    Ok(())
}
