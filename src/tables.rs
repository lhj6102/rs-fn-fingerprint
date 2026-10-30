use std::collections::BTreeMap;

use crate::{Error, Fingerprints, Hash, insert, matcher, named_query};

/// Hash sorted `(dotted table key, canonical JSON value)` pairs per query.
///
/// Wildcards follow `globset` syntax and may span dots. Implicit tables are
/// included. Arrays of tables are selected as a single ordered array at their
/// dotted key. Empty matches are valid and have a stable, non-null digest.
pub fn fingerprint_toml(source: &str, queries: &[impl AsRef<str>]) -> Result<Fingerprints, Error> {
    fingerprint_toml_groups(source, &[queries]).map(|mut groups| groups.remove(0))
}

pub(crate) fn fingerprint_toml_groups(
    source: &str,
    groups: &[&[impl AsRef<str>]],
) -> Result<Vec<Fingerprints>, Error> {
    let value: toml::Table = toml::from_str(source)?;
    let mut tables = BTreeMap::new();
    collect(&value, "", &mut tables);
    groups
        .iter()
        .map(|queries| {
            let mut results = Fingerprints::new();
            for raw in *queries {
                let raw = raw.as_ref();
                let (alias, pattern) = named_query(raw, false)?;
                if pattern.is_empty() {
                    return Err(Error::Query {
                        query: raw.into(),
                        reason: "table glob must not be empty".into(),
                    });
                }
                let glob = matcher(pattern, raw)?;
                let mut hash = Hash::new("toml:tables");
                for (key, value) in &tables {
                    if glob.is_match(key) {
                        hash.field(key);
                        hash.field(&serde_json::to_string(&canonical(value)?)?);
                    }
                }
                insert(
                    &mut results,
                    alias.unwrap_or(pattern).into(),
                    Some(hash.finish()),
                )?;
            }
            Ok(results)
        })
        .collect()
}

fn collect<'a>(
    table: &'a toml::Table,
    prefix: &str,
    tables: &mut BTreeMap<String, &'a toml::Value>,
) {
    for (key, value) in table {
        // Quote non-bare segments so ["a.b"] cannot collide with [a.b].
        let segment = if !key.is_empty()
            && key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        {
            key.clone()
        } else {
            serde_json::to_string(key).expect("strings serialize to JSON")
        };
        let path = if prefix.is_empty() {
            segment
        } else {
            format!("{prefix}.{segment}")
        };
        match value {
            toml::Value::Table(nested) => {
                tables.insert(path.clone(), value);
                collect(nested, &path, tables);
            }
            toml::Value::Array(values)
                if !values.is_empty() && values.iter().all(toml::Value::is_table) =>
            {
                tables.insert(path, value);
            }
            _ => {}
        }
    }
}

fn canonical(value: &toml::Value) -> Result<serde_json::Value, Error> {
    Ok(match value {
        toml::Value::Table(table) => {
            let mut sorted = BTreeMap::new();
            for (key, value) in table {
                sorted.insert(key, canonical(value)?);
            }
            serde_json::to_value(sorted)?
        }
        toml::Value::Array(values) => values
            .iter()
            .map(canonical)
            .collect::<Result<Vec<_>, _>>()?
            .into(),
        // JSON cannot represent these; keep them distinct rather than silently
        // collapsing NaN and both infinities to null.
        toml::Value::Float(value) if !value.is_finite() => {
            serde_json::json!({"$float": value.to_string()})
        }
        toml::Value::Datetime(value) => serde_json::json!({"$datetime": value.to_string()}),
        value => serde_json::to_value(value)?,
    })
}
