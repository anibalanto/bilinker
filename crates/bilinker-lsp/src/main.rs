use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use bilinker::check::find_by_file;
use bilinker::config::Config;
use bilinker::get::get;

struct Backend {
    client: Client,
}

impl Backend {
    fn project_root(&self, uri: &Url) -> Option<PathBuf> {
        let path = uri.to_file_path().ok()?;
        let (root, _) = Config::load_from(&path).ok()?;
        Some(root)
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> LspResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                code_lens_provider: Some(CodeLensOptions { resolve_provider: Some(false) }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "bilinker-lsp".into(),
                version: Some("0.1.0".into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "bilinker-lsp ready")
            .await;
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let uri  = &params.text_document_position_params.text_document.uri;
        let pos  = params.text_document_position_params.position;
        let Some(root) = self.project_root(uri) else { return Ok(None) };
        let file_path  = uri.to_file_path().unwrap_or_default();

        self.client.log_message(MessageType::INFO,
            format!("hover: root={} file={}", root.display(), file_path.display())).await;
        let results = find_by_file(&root, &file_path).unwrap_or_default();
        self.client.log_message(MessageType::INFO,
            format!("hover: {} bilinks found", results.len())).await;
        if results.is_empty() { return Ok(None); }

        // Filter bilinks whose range covers the cursor position
        let source = std::fs::read_to_string(&file_path).unwrap_or_default();
        let cursor_byte = line_col_to_byte(&source, pos.line as usize + 1, pos.character as usize + 1);

        let covering: Vec<_> = results.iter()
            .filter(|(_, _, range)| range.start <= cursor_byte && cursor_byte < range.end)
            .collect();

        if covering.is_empty() { return Ok(None); }

        let mut lines = vec!["**bilinks**\n".to_string()];
        for (bilink_path, n, range) in &covering {
            let uuid = bilink_path.file_stem()
                .and_then(|s| s.to_str()).unwrap_or("?");
            let uuid_short = &uuid[..8.min(uuid.len())];

            // Try to get the content of the other side
            let other_side = if *n == 0 { 1u8 } else { 0u8 };
            let content = match get(&root, uuid, other_side, None, None) {
                Ok(r) => format!("`{}` lines {}–{}\n```{}\n{}\n```",
                    r.file, r.start_line, r.end_line,
                    lang_from_file(&r.file), r.content),
                Err(e) => {
                    self.client.log_message(
                        MessageType::ERROR,
                        format!("get {uuid}.{other_side}: {e:#}"),
                    ).await;
                    format!("bytes {}–{}", range.start, range.end)
                }
            };

            lines.push(format!("**{}** (`.{}`)\n{}", uuid_short, n, content));
        }

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: lines.join("\n\n---\n\n"),
            }),
            range: None,
        }))
    }

    async fn code_lens(&self, params: CodeLensParams) -> LspResult<Option<Vec<CodeLens>>> {
        let uri = &params.text_document.uri;
        let Some(root) = self.project_root(uri) else { return Ok(None) };
        let file_path  = uri.to_file_path().unwrap_or_default();
        let source     = std::fs::read_to_string(&file_path).unwrap_or_default();

        let results = find_by_file(&root, &file_path).unwrap_or_default();
        if results.is_empty() { return Ok(Some(vec![])); }

        // Group bilinks by start line
        use std::collections::BTreeMap;
        let mut by_line: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (bilink_path, n, range) in &results {
            let line = byte_to_line(&source, range.start);
            let uuid = bilink_path.file_stem()
                .and_then(|s| s.to_str()).unwrap_or("?");
            by_line.entry(line)
                .or_default()
                .push(format!("{}.{}", &uuid[..8.min(uuid.len())], n));
        }

        let lenses = by_line.into_iter().map(|(line, ids)| {
            let label = format!("⬡ {} bilink{}", ids.len(), if ids.len() == 1 { "" } else { "s" });
            CodeLens {
                range: Range {
                    start: Position { line: line as u32, character: 0 },
                    end:   Position { line: line as u32, character: 0 },
                },
                command: Some(Command {
                    title: label,
                    command: "bilinker.showBilinks".into(),
                    arguments: Some(vec![
                        serde_json::json!(uri.to_string()),
                        serde_json::json!(ids),
                    ]),
                }),
                data: None,
            }
        }).collect();

        Ok(Some(lenses))
    }

}

fn byte_to_line(source: &str, byte: usize) -> usize {
    source[..byte.min(source.len())].chars().filter(|&c| c == '\n').count()
}

fn line_col_to_byte(source: &str, line: usize, col: usize) -> usize {
    let mut cur_line = 1;
    for (i, c) in source.char_indices() {
        if cur_line == line {
            return i + (col - 1).min(source.len().saturating_sub(i));
        }
        if c == '\n' { cur_line += 1; }
    }
    source.len()
}

fn lang_from_file(file: &str) -> &'static str {
    match std::path::Path::new(file).extension().and_then(|e| e.to_str()) {
        Some("rs")           => "rust",
        Some("java")         => "java",
        Some("yaml" | "yml") => "yaml",
        Some("md")           => "markdown",
        Some("ts" | "tsx")   => "typescript",
        Some("js" | "jsx")   => "javascript",
        Some("py")           => "python",
        _                    => "",
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let stdin  = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Arc::new(Backend { client }));
    Server::new(stdin, stdout, socket).serve(service).await;
    Ok(())
}
