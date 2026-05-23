use std::fmt;
use std::str::FromStr;
use anyhow::{bail, Context};

/// A parsed bilink endpoint: `workspace :: file :: query [:: start~end]`
/// or a reference to another bilink by id.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEndpoint {
    Structural(StructuralRef),
    BiLinkRef(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructuralRef {
    pub workspace: String,
    pub file: String,
    pub query: String,
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
        // If it contains "::", it's a structural ref; otherwise a bilink id.
        if !s.contains("::") {
            return Ok(LinkEndpoint::BiLinkRef(s.trim().to_string()));
        }
        Ok(LinkEndpoint::Structural(s.parse()?))
    }
}

impl FromStr for StructuralRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(4, "::").map(str::trim).collect();
        match parts.as_slice() {
            [ws, file, query] => Ok(Self {
                workspace: ws.to_string(),
                file: file.to_string(),
                query: query.to_string(),
                range: None,
            }),
            [ws, file, query, range] => Ok(Self {
                workspace: ws.to_string(),
                file: file.to_string(),
                query: query.to_string(),
                range: Some(range.parse().context("invalid start~end range")?),
            }),
            _ => bail!("expected `workspace :: file :: query [:: start~end]`, got: {s}"),
        }
    }
}

impl fmt::Display for LinkEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkEndpoint::Structural(r) => write!(f, "{r}"),
            LinkEndpoint::BiLinkRef(id) => write!(f, "{id}"),
        }
    }
}

impl fmt::Display for StructuralRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} :: {} :: {}", self.workspace, self.file, self.query)?;
        if let Some(r) = &self.range {
            write!(f, " :: {r}")?;
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
        let ep: LinkEndpoint = "java-demo :: persona/Persona.java :: (class_declaration name:#eq?Persona)".parse().unwrap();
        assert!(matches!(ep, LinkEndpoint::Structural(_)));
    }

    #[test]
    fn parse_structural_with_range() {
        let ep: LinkEndpoint = "docs :: architecture.md :: (paragraph) @target :: 42~87".parse().unwrap();
        if let LinkEndpoint::Structural(r) = ep {
            assert_eq!(r.range, Some(ByteRange { start: 42, end: 87 }));
        } else {
            panic!("expected Structural");
        }
    }

    #[test]
    fn parse_bilink_ref() {
        let ep: LinkEndpoint = "persona-voting-impl".parse().unwrap();
        assert_eq!(ep, LinkEndpoint::BiLinkRef("persona-voting-impl".to_string()));
    }

    #[test]
    fn roundtrip_structural() {
        let s = "docs :: architecture.md :: (paragraph) @target :: 42~87";
        let ep: LinkEndpoint = s.parse().unwrap();
        assert_eq!(ep.to_string(), s);
    }
}
