use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use chrono::Utc;

use crate::link::ByteRange;

#[derive(Debug, Clone, PartialEq)]
pub enum ScipLinkState {
    Ok,
    Altered,
    Renamed,
    Deleted,
}

impl ScipLinkState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok      => "OK",
            Self::Altered => "ALTERED",
            Self::Renamed => "RENAMED",
            Self::Deleted => "DELETED",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "OK"      => Some(Self::Ok),
            "ALTERED" => Some(Self::Altered),
            "RENAMED" => Some(Self::Renamed),
            "DELETED" => Some(Self::Deleted),
            _         => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScipLink {
    pub symbol: String,
    pub file: String,
    pub range: ByteRange,
    pub hash: Option<String>,
    pub commit: Option<String>,
    pub state: Option<ScipLinkState>,
    pub resolved_at: Option<String>,
}

impl ScipLink {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let mut symbol = None;
        let mut file = None;
        let mut range = None;
        let mut hash = None;
        let mut commit = None;
        let mut state = None;
        let mut resolved_at = None;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            if let Some(v) = strip_key(line, "symbol")      { symbol      = Some(v.to_string()); }
            else if let Some(v) = strip_key(line, "file")   { file        = Some(v.to_string()); }
            else if let Some(v) = strip_key(line, "range")  { range       = Some(v.parse::<ByteRange>().context("parsing range")?); }
            else if let Some(v) = strip_key(line, "hash")   { hash        = Some(v.to_string()); }
            else if let Some(v) = strip_key(line, "commit") { commit      = Some(v.to_string()); }
            else if let Some(v) = strip_key(line, "state")  { state       = ScipLinkState::parse(v); }
            else if let Some(v) = strip_key(line, "resolved_at") { resolved_at = Some(v.to_string()); }
        }

        Ok(ScipLink {
            symbol:      symbol.context("missing 'symbol' field")?,
            file:        file.context("missing 'file' field")?,
            range:       range.context("missing 'range' field")?,
            hash,
            commit,
            state,
            resolved_at,
        })
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::new();
        out.push_str(&format!("symbol: {}\n", self.symbol));
        out.push_str(&format!("file:   {}\n", self.file));
        out.push_str(&format!("range:  {}\n", self.range));
        if let Some(h) = &self.hash        { out.push_str(&format!("hash:   {h}\n")); }
        if let Some(c) = &self.commit      { out.push_str(&format!("commit: {c}\n")); }
        if let Some(s) = &self.state       { out.push_str(&format!("state:  {}\n", s.as_str())); }
        if let Some(t) = &self.resolved_at { out.push_str(&format!("resolved_at: {t}\n")); }
        std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))
    }

    pub fn with_state(mut self, state: ScipLinkState) -> Self {
        self.state = Some(state);
        self.resolved_at = Some(Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
        self
    }
}

/// Derives the `.sciplink` filename from a SCIP symbol ID.
///
/// `scip://rust . voting/Repo#save().` → `rust.voting..Repo.save.sciplink`
pub fn normalize_symbol_id(symbol: &str) -> String {
    let s = symbol
        .trim_start_matches("scip://")
        .replace('/', ".")
        .replace('#', "..")
        .replace("()", "")
        .replace(' ', "");
    let s = s.trim_end_matches('.');
    format!("{s}.sciplink")
}

/// Returns the path of a `.sciplink` file for the given symbol in the given layer.
pub fn sciplink_path(bilink_dir: &Path, symbol: &str) -> PathBuf {
    bilink_dir.join("sciplink").join(normalize_symbol_id(symbol))
}

fn strip_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}:");
    line.strip_prefix(&prefix).map(|v| v.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_symbol_rust() {
        assert_eq!(
            normalize_symbol_id("scip://rust . voting/Repo#save()."),
            "rust.voting.Repo..save.sciplink"
        );
    }

    #[test]
    fn roundtrip() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.sciplink");
        let sl = ScipLink {
            symbol:      "scip://rust . x/Foo#bar().".into(),
            file:        "src/foo.rs".into(),
            range:       ByteRange { start: 10, end: 50 },
            hash:        Some("abc123".into()),
            commit:      Some("def456".into()),
            state:       Some(ScipLinkState::Ok),
            resolved_at: Some("2026-06-04T00:00:00Z".into()),
        };
        sl.write(&path).unwrap();
        let loaded = ScipLink::load(&path).unwrap();
        assert_eq!(loaded.symbol, sl.symbol);
        assert_eq!(loaded.range, sl.range);
        assert_eq!(loaded.state, sl.state);
    }
}
