use std::fmt;
use std::str::FromStr;
use anyhow::{bail, Context};
use estrato::EstratPath;

#[derive(Debug, Clone, PartialEq)]
pub enum EndpointState {
    Ok,
    Moved,
    Displaced,
    Reanchored,
    Expanded,
    Unanchored,
    Altered,
    Deleted,
    Broken,
    ChainDirty,
}

impl fmt::Display for EndpointState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok          => write!(f, "OK"),
            Self::Moved       => write!(f, "MOVED"),
            Self::Displaced   => write!(f, "DISPLACED"),
            Self::Reanchored  => write!(f, "REANCHORED"),
            Self::Expanded    => write!(f, "EXPANDED"),
            Self::Unanchored  => write!(f, "UNANCHORED"),
            Self::Altered     => write!(f, "ALTERED"),
            Self::Deleted     => write!(f, "DELETED"),
            Self::Broken      => write!(f, "BROKEN"),
            Self::ChainDirty  => write!(f, "CHAIN_DIRTY"),
        }
    }
}

impl FromStr for EndpointState {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "OK"           => Ok(Self::Ok),
            "MOVED"        => Ok(Self::Moved),
            "DISPLACED"    => Ok(Self::Displaced),
            "REANCHORED"   => Ok(Self::Reanchored),
            "EXPANDED"     => Ok(Self::Expanded),
            "UNANCHORED"   => Ok(Self::Unanchored),
            "ALTERED"      => Ok(Self::Altered),
            "DELETED"      => Ok(Self::Deleted),
            "BROKEN"       => Ok(Self::Broken),
            "CHAIN_DIRTY"  => Ok(Self::ChainDirty),
            other          => bail!("estado desconocido: '{other}'"),
        }
    }
}

/// A parsed bilink endpoint: `file [:: query [:: start~end]]`
/// or an estrato path pointing to a layer directory.
///
/// Disambiguation: if the string contains `::` it is always Structural.
/// If it has no `::`, it is Structural when the last path component has a
/// file extension (e.g. `spec.md`, `src/Foo.java`); otherwise it is a Layer.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEndpoint {
    Structural(StructuralRef),
    Layer(EstratPath),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructuralRef {
    pub file: String,
    pub query: Option<String>,
    pub range: Option<ByteRange>,
}

/// A bilink connects exactly two endpoints.
#[derive(Debug, Clone)]
pub struct BiLink {
    pub id: String,
    pub link0: LinkEndpoint,
    pub link1: LinkEndpoint,
    pub hash0: Option<String>,
    pub hash1: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl FromStr for LinkEndpoint {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains("::") {
            return Ok(LinkEndpoint::Structural(s.parse()?));
        }
        // No `::`: check if the last path component has a file extension.
        let last = std::path::Path::new(s.trim())
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let looks_like_file = last.contains('.') && last != "." && last != "..";
        if looks_like_file {
            return Ok(LinkEndpoint::Structural(StructuralRef {
                file:  s.trim().to_string(),
                query: None,
                range: None,
            }));
        }
        let tokens = estrato::parse_path(s.trim())
            .map_err(|e| anyhow::anyhow!("invalid estrato path '{s}': {e}"))?;
        Ok(LinkEndpoint::Layer(tokens))
    }
}

impl FromStr for StructuralRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(3, "::").map(str::trim).collect();
        match parts.as_slice() {
            [file] => Ok(Self {
                file: file.to_string(),
                query: None,
                range: None,
            }),
            [file, query] => Ok(Self {
                file: file.to_string(),
                query: Some(query.to_string()),
                range: None,
            }),
            [file, query, range] => Ok(Self {
                file: file.to_string(),
                query: Some(query.to_string()),
                range: Some(range.parse().context("invalid start~end range")?),
            }),
            _ => bail!("expected `file [:: query [:: start~end]]`, got: {s}"),
        }
    }
}

impl fmt::Display for LinkEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkEndpoint::Structural(r) => write!(f, "{r}"),
            LinkEndpoint::Layer(tokens) => {
                use estrato::PathToken;
                for token in tokens {
                    match token {
                        PathToken::Down(name) => write!(f, ">{name}")?,
                        PathToken::Up => write!(f, "<")?,
                        PathToken::Simple(p) => write!(f, "{}", p.display())?,
                    }
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for StructuralRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(q) = &self.query {
            write!(f, " :: {q}")?;
            if let Some(r) = &self.range {
                write!(f, " :: {r}")?;
            }
        }
        Ok(())
    }
}

impl FromStr for ByteRange {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (start, end) = s.split_once('~').context("range must be `start~end`")?;
        Ok(Self {
            start: start.trim().parse().context("invalid start offset")?,
            end: end.trim().parse().context("invalid end offset")?,
        })
    }
}

impl fmt::Display for ByteRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}~{}", self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_structural_without_range() {
        let ep: LinkEndpoint = "Persona.java :: (class_declaration name:#eq?Persona)".parse().unwrap();
        assert!(matches!(ep, LinkEndpoint::Structural(_)));
    }

    #[test]
    fn parse_structural_with_range() {
        let ep: LinkEndpoint = "docs/architecture.md :: (paragraph) @target :: 42~87".parse().unwrap();
        if let LinkEndpoint::Structural(r) = ep {
            assert_eq!(r.range, Some(ByteRange { start: 42, end: 87 }));
        } else {
            panic!("expected Structural");
        }
    }

    #[test]
    fn parse_layer_simple_path() {
        let ep: LinkEndpoint = "persona-voting-impl".parse().unwrap();
        assert!(matches!(ep, LinkEndpoint::Layer(_)));
    }

    #[test]
    fn parse_layer_estrato_down() {
        let ep: LinkEndpoint = ">tech-decisions>impl".parse().unwrap();
        assert!(matches!(ep, LinkEndpoint::Layer(_)));
    }

    #[test]
    fn roundtrip_structural() {
        let s = "docs/architecture.md :: (paragraph) @target :: 42~87";
        let ep: LinkEndpoint = s.parse().unwrap();
        assert_eq!(ep.to_string(), s);
    }

    #[test]
    fn parse_whole_file_endpoint() {
        let ep: LinkEndpoint = "docs/architecture.md".parse().unwrap();
        if let LinkEndpoint::Structural(r) = ep {
            assert_eq!(r.file, "docs/architecture.md");
            assert!(r.query.is_none());
            assert!(r.range.is_none());
        } else {
            panic!("expected Structural");
        }
    }

    #[test]
    fn roundtrip_whole_file() {
        let s = "docs/architecture.md";
        let ep: LinkEndpoint = s.parse().unwrap();
        assert_eq!(ep.to_string(), s);
    }
}
