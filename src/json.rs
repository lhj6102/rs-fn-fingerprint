use std::collections::BTreeMap;

use serde_json::Value;

use crate::{Error, Fingerprints, Hash, insert, matcher, named_query};

/// Select JSON objects at any depth by `field:<name>:<JSON value>`, or select
/// values by `path:<dotted-glob>` (numeric segments index arrays). Object keys
/// use TOML-style quoted segments. Object order and whitespace do not matter;
/// array order does. Empty matches have a stable, non-null hash.
pub fn fingerprint_json(source: &str, queries: &[impl AsRef<str>]) -> Result<Fingerprints, Error> {
    fingerprint_json_groups(source, &[queries]).map(|mut groups| groups.remove(0))
}

pub(crate) fn fingerprint_json_groups(
    source: &str,
    groups: &[&[impl AsRef<str>]],
) -> Result<Vec<Fingerprints>, Error> {
    let root: Value = serde_json::from_str(source)?;
    let mut values = BTreeMap::new();
    collect(&root, "", &mut values);
    groups
        .iter()
        .map(|queries| {
            let mut results = Fingerprints::new();
            for raw in *queries {
                let raw = raw.as_ref();
                // '=' in JSON string values is not an alias separator.
                let (alias, query) = if raw.starts_with("field:") || raw.starts_with("path:") {
                    (None, raw)
                } else {
                    named_query(raw, false)?
                };
                let mut hash = Hash::new("json:values");
                if let Some(pattern) = query.strip_prefix("path:") {
                    if pattern.is_empty() {
                        return Err(invalid(raw));
                    }
                    let glob = matcher(pattern, raw)?;
                    for (path, value) in &values {
                        if glob.is_match(path) {
                            add(&mut hash, path, value)?;
                        }
                    }
                } else if let Some(field) = query.strip_prefix("field:") {
                    let (field, expected) = field.split_once(':').ok_or_else(|| invalid(raw))?;
                    if field.is_empty() {
                        return Err(invalid(raw));
                    }
                    let expected: Value = serde_json::from_str(expected)?;
                    for (path, value) in &values {
                        if value.as_object().and_then(|object| object.get(field)) == Some(&expected)
                        {
                            add(&mut hash, path, value)?;
                        }
                    }
                } else {
                    return Err(invalid(raw));
                }
                insert(
                    &mut results,
                    alias.unwrap_or(query).into(),
                    Some(hash.finish()),
                )?;
            }
            Ok(results)
        })
        .collect()
}
fn invalid(query: &str) -> Error {
    Error::Query {
        query: query.into(),
        reason: "expected path:<dotted-glob> or field:<name>:<JSON-value>".into(),
    }
}
fn add(hash: &mut Hash, path: &str, value: &Value) -> Result<(), Error> {
    hash.field(path);
    hash.field(&serde_json::to_string(&canonical(value))?);
    Ok(())
}
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted: BTreeMap<_, _> = object
                .iter()
                .map(|(key, value)| (key.clone(), canonical(value)))
                .collect();
            serde_json::to_value(sorted).expect("JSON values serialize")
        }
        Value::Array(array) => array.iter().map(canonical).collect(),
        value => value.clone(),
    }
}
fn collect<'a>(value: &'a Value, path: &str, values: &mut BTreeMap<String, &'a Value>) {
    values.insert(path.into(), value);
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let segment = if !key.is_empty()
                    && key
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
                {
                    key.clone()
                } else {
                    serde_json::to_string(key).expect("JSON strings serialize")
                };
                collect(value, &join(path, &segment), values);
            }
        }
        Value::Array(array) => {
            for (i, value) in array.iter().enumerate() {
                collect(value, &join(path, &i.to_string()), values);
            }
        }
        _ => {}
    }
}
fn join(path: &str, segment: &str) -> String {
    if path.is_empty() {
        segment.into()
    } else {
        format!("{path}.{segment}")
    }
}
