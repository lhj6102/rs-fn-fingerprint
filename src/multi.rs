use std::{collections::BTreeMap, fs, io, path::PathBuf};

use sha2::{Digest, Sha256};

use crate::{Error, Fingerprints, Hash, combine, json, rust, tables};

/// A source parser, or raw bytes without parsing.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Mode {
    Rust,
    Json,
    Toml,
    File,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::File => "file",
        }
    }
}

/// One ordered source group. Paths resolve against the process working directory.
#[derive(Clone, Debug)]
pub struct SourceGroup {
    pub mode: Mode,
    /// Preserved verbatim in output and the combined digest.
    pub file: String,
    /// Empty for `Mode::File`; otherwise at least one query is required.
    pub queries: Vec<String>,
}

/// A group's tagged results, in the same order as the input groups.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroupFingerprint {
    pub mode: Mode,
    pub file: String,
    pub results: Fingerprints,
}

/// Read each distinct file once and parse it once per requested parser mode.
/// Repeated groups share parsing but have independent result-key namespaces.
/// Only NotFound is treated as absence: empty TOML, an empty JSON object, or
/// empty raw bytes. Missing Rust requires `allow_missing`, then uses empty Rust.
pub fn fingerprint_multi(
    groups: &[SourceGroup],
    allow_missing: bool,
) -> Result<Vec<GroupFingerprint>, Error> {
    fingerprint_multi_with(groups, allow_missing, |path| fs::read(path))
}

fn fingerprint_multi_with(
    groups: &[SourceGroup],
    allow_missing: bool,
    mut read: impl FnMut(&PathBuf) -> io::Result<Vec<u8>>,
) -> Result<Vec<GroupFingerprint>, Error> {
    if groups.is_empty() {
        return Err(Error::Group("at least one source group is required".into()));
    }
    let mut sources = BTreeMap::new();
    let mut batches: BTreeMap<(PathBuf, Mode), Vec<usize>> = BTreeMap::new();
    let mut output = Vec::with_capacity(groups.len());
    for (i, group) in groups.iter().enumerate() {
        if (group.mode == Mode::File) != group.queries.is_empty() {
            return Err(Error::Group(format!(
                "{} requires {}",
                group.mode.as_str(),
                if group.mode == Mode::File {
                    "only a file, no queries"
                } else {
                    "a file and at least one query"
                }
            )));
        }
        let source_error = |source| Error::Source {
            file: group.file.clone(),
            source,
        };
        // Canonicalization also shares reads/parsing across relative, absolute,
        // and symlink spellings, without changing the path tagged in output.
        let path = match fs::canonicalize(&group.file) {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => std::env::current_dir()
                .map_err(source_error)?
                .join(&group.file),
            Err(error) => return Err(source_error(error)),
        };
        if !sources.contains_key(&path) {
            let bytes = match read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(source_error(error)),
            };
            sources.insert(path.clone(), bytes);
        }
        if sources[&path].is_none() && group.mode == Mode::Rust && !allow_missing {
            return Err(source_error(io::Error::new(
                io::ErrorKind::NotFound,
                "missing Rust file (use --allow-missing to query empty Rust)",
            )));
        }
        batches.entry((path, group.mode)).or_default().push(i);
        output.push(GroupFingerprint {
            mode: group.mode,
            file: group.file.clone(),
            results: Fingerprints::new(),
        });
    }
    for ((path, mode), indices) in batches {
        let bytes = sources[&path].as_deref();
        if mode == Mode::File {
            let digest = format!("sha256:{:x}", Sha256::digest(bytes.unwrap_or_default()));
            for i in indices {
                output[i]
                    .results
                    .insert("bytes".into(), Some(digest.clone()));
            }
            continue;
        }
        let source = std::str::from_utf8(
            bytes.unwrap_or_else(|| if mode == Mode::Json { b"{}" } else { b"" }),
        )
        .map_err(|error| Error::Source {
            file: groups[indices[0]].file.clone(),
            source: io::Error::new(io::ErrorKind::InvalidData, error),
        })?;
        let queries: Vec<_> = indices
            .iter()
            .map(|&i| groups[i].queries.as_slice())
            .collect();
        let results = match mode {
            Mode::Rust => rust::fingerprint_rust_groups(source, &queries, allow_missing)?,
            Mode::Json => json::fingerprint_json_groups(source, &queries)?,
            Mode::Toml => tables::fingerprint_toml_groups(source, &queries)?,
            Mode::File => unreachable!(),
        };
        for (i, results) in indices.into_iter().zip(results) {
            output[i].results = results;
        }
    }
    Ok(output)
}

/// Hash mode, verbatim path, and results of every group in input order.
/// Uses a new domain; existing single-mode digests are unchanged.
pub fn combine_multi(groups: &[GroupFingerprint]) -> String {
    let mut hash = Hash::new("multi:combined");
    for group in groups {
        hash.field(group.mode.as_str());
        hash.field(&group.file);
        hash.field(&combine(&group.results));
    }
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_tags_modes_paths_results_and_order_even_with_identical_results() {
        let first = GroupFingerprint {
            mode: Mode::Rust,
            file: "source".into(),
            results: Fingerprints::from([("key".into(), Some("sha256:value".into()))]),
        };
        let mut second = first.clone();
        second.mode = Mode::Json;
        assert_ne!(
            combine_multi(std::slice::from_ref(&first)),
            combine_multi(std::slice::from_ref(&second))
        );
        assert_ne!(
            combine_multi(&[first.clone(), second.clone()]),
            combine_multi(&[second, first.clone()])
        );
        let mut moved = first.clone();
        moved.file = "other".into();
        assert_ne!(
            combine_multi(std::slice::from_ref(&first)),
            combine_multi(&[moved])
        );
        let mut changed = first.clone();
        changed.results.insert("key".into(), None);
        assert_ne!(
            combine_multi(std::slice::from_ref(&first)),
            combine_multi(&[changed])
        );
        let empty = GroupFingerprint {
            results: Fingerprints::new(),
            ..first.clone()
        };
        assert_ne!(
            combine_multi(&[first.clone(), empty.clone()]),
            combine_multi(&[empty, first])
        );
    }

    #[test]
    fn byte_reads_are_shared_between_parsers_and_raw_groups() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("source");
        fs::write(&file, "").unwrap();
        let groups = [Mode::Rust, Mode::Toml, Mode::File, Mode::Toml].map(|mode| SourceGroup {
            mode,
            file: file.to_str().unwrap().into(),
            queries: if mode == Mode::File {
                vec![]
            } else if mode == Mode::Rust {
                vec!["branch:*:42".into()]
            } else {
                vec!["*".into()]
            },
        });
        let mut reads = 0;
        let results = fingerprint_multi_with(&groups, false, |path| {
            reads += 1;
            fs::read(path)
        })
        .unwrap();
        assert_eq!(reads, 1);
        assert_eq!(results[1].results, results[3].results);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_spellings_share_reads_but_keep_their_tags() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("source.rs");
        let alias = dir.path().join("alias.rs");
        fs::write(&file, "fn f() {}").unwrap();
        std::os::unix::fs::symlink(&file, &alias).unwrap();
        let groups = [file, alias].map(|file| SourceGroup {
            mode: Mode::Rust,
            file: file.to_str().unwrap().into(),
            queries: vec!["fn:f".into()],
        });
        let mut reads = 0;
        let results = fingerprint_multi_with(&groups, false, |path| {
            reads += 1;
            fs::read(path)
        })
        .unwrap();
        assert_eq!(reads, 1);
        assert_eq!(results[0].results, results[1].results);
        assert_ne!(results[0].file, results[1].file);
    }

    #[test]
    fn repeated_groups_read_once_and_keep_independent_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("source.rs");
        fs::write(&file, "fn first() {} fn second() {}").unwrap();
        let groups = ["same=fn:first", "same=fn:second"].map(|query| SourceGroup {
            mode: Mode::Rust,
            file: file.to_str().unwrap().into(),
            queries: vec![query.into()],
        });
        let mut reads = 0;
        let results = fingerprint_multi_with(&groups, false, |path| {
            reads += 1;
            fs::read(path)
        })
        .unwrap();
        assert_eq!(reads, 1);
        assert_ne!(results[0].results, results[1].results);
    }
}
