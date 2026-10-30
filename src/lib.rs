//! Query-based, same-file fingerprints without compiler or dependency resolution.
//!
//! See [`fingerprint_rust`], [`fingerprint_toml`], and [`combine`].

mod json;
mod multi;
mod rust;
mod tables;

use std::collections::BTreeMap;

use globset::{Glob, GlobMatcher};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub use json::fingerprint_json;
pub use multi::{GroupFingerprint, Mode, SourceGroup, combine_multi, fingerprint_multi};
pub use rust::fingerprint_rust;
pub use tables::fingerprint_toml;

/// Every query/combined digest is domain-separated and starts with this tag.
pub const FORMAT_TAG: &str = "rs-fn-fingerprint/v1";

/// Sorted output keys; `None` represents an explicitly allowed missing Rust query.
pub type Fingerprints = BTreeMap<String, Option<String>>;

/// Source I/O, invalid source/query/group, or conflicting output keys.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid source group: {0}")]
    Group(String),
    #[error("cannot read {file}: {source}")]
    Source {
        file: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid Rust source: {0}")]
    Rust(#[from] syn::Error),
    #[error("invalid TOML source: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid query {query:?}: {reason}")]
    Query { query: String, reason: String },
    #[error("Rust query matched no items: {0}")]
    Missing(String),
    #[error("queries produce conflicting values for output key {0:?}; use name= aliases")]
    Collision(String),
    #[error("cannot serialize canonical value: {0}")]
    Json(#[from] serde_json::Error),
}

/// Hash all sorted output keys and values, including nulls. Query order is irrelevant.
pub fn combine(results: &Fingerprints) -> String {
    let mut hash = Hash::new("combined");
    for (key, value) in results {
        hash.field(key);
        match value {
            Some(value) => {
                hash.field("hash");
                hash.field(value);
            }
            None => hash.field("null"),
        }
    }
    hash.finish()
}

struct Hash(Sha256);

impl Hash {
    fn new(domain: &str) -> Self {
        let mut hash = Self(Sha256::new());
        hash.0.update(FORMAT_TAG.as_bytes());
        hash.0.update([0]);
        hash.field(domain);
        hash
    }

    fn field(&mut self, value: &str) {
        self.0.update((value.len() as u64).to_le_bytes());
        self.0.update(value.as_bytes());
    }

    fn finish(self) -> String {
        format!("sha256:{:x}", self.0.finalize())
    }
}

fn matcher(pattern: &str, query: &str) -> Result<GlobMatcher, Error> {
    Glob::new(pattern)
        .map(|glob| glob.compile_matcher())
        .map_err(|error| Error::Query {
            query: query.into(),
            reason: error.to_string(),
        })
}

fn named_query(query: &str, rust: bool) -> Result<(Option<&str>, &str), Error> {
    if rust
        && (query.starts_with("fn:") || query.starts_with("arm:") || query.starts_with("branch:"))
    {
        return Ok((None, query));
    }
    let mut quote = None;
    let mut escaped = false;
    let mut class = false;
    let mut braces = 0_usize;
    let separator = query.char_indices().find_map(|(index, ch)| {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else {
            match ch {
                '[' => class = true,
                ']' => class = false,
                '{' if !class => braces += 1,
                '}' if !class => braces = braces.saturating_sub(1),
                '=' if !class && braces == 0 => return Some(index),
                _ => {}
            }
        }
        None
    });
    let parts = separator.map(|index| (&query[..index], &query[index + 1..]));
    match parts {
        Some(("", _)) | Some((_, "")) => Err(Error::Query {
            query: query.into(),
            reason: "an alias and its query must both be nonempty".into(),
        }),
        Some((name, pattern)) => Ok((Some(name), pattern)),
        None => Ok((None, query)),
    }
}

fn insert(results: &mut Fingerprints, key: String, value: Option<String>) -> Result<(), Error> {
    if let Some(previous) = results.get(&key) {
        if previous != &value {
            return Err(Error::Collision(key));
        }
    } else {
        results.insert(key, value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_tagged_framed_and_domain_separated() {
        let mut first = Hash::new("a");
        first.field("bc");
        let mut second = Hash::new("ab");
        second.field("c");
        assert_ne!(first.finish(), second.finish());
        let mut expected = Sha256::new();
        expected.update(b"rs-fn-fingerprint/v1\0");
        expected.update(1_u64.to_le_bytes());
        expected.update(b"x");
        assert_eq!(
            Hash::new("x").finish(),
            format!("sha256:{:x}", expected.finalize())
        );
    }

    #[test]
    fn combined_null_and_absence_differ() {
        assert_ne!(
            combine(&Fingerprints::new()),
            combine(&BTreeMap::from([("x".into(), None)]))
        );
    }
}
