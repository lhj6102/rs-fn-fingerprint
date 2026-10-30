# rs-fn-fingerprint

Fast, query-based SHA-256 fingerprints of selected Rust items, literal-owned branches, TOML tables, JSON records, or raw files.
One invocation can fingerprint ordered groups across files and modes. Each distinct
file is read once and parsed once per requested parser mode, even with repeated groups. No compiler, rust-analyzer, crate graph,
macro expansion, or cross-file dependency resolution is involved.

## Build and install

Rust 1.85 or newer is required.

```sh
cargo build --release
cargo install --path .
```

## Multiple sources in one invocation

```sh
rs-fn-fingerprint multi --combine \
  --rust src/operations.rs 'implementation=fn:compute' \
  --json records.json 'row=field:id:42' \
  --file inputs/request.json \
  --toml config.toml 'settings.100' 'variants.100.*' \
  --rust src/operations.rs 'optional=fn:missing' --allow-missing
```

Groups are **ordered**. Each starts with `--rust FILE QUERY...`,
`--json FILE QUERY...`, `--toml FILE QUERY...`, or `--file FILE` (no queries).
Repeat any group marker, including for the same file. Words starting with `--`
are reserved for options and group markers; prefix a filename starting with `--` with `./`. Query aliases and globs
have exactly the single-mode semantics. Conflicting keys are errors **within** a
group; different groups have independent key namespaces.

Without `--combine`, output is one JSON object whose `groups` array preserves
input order, repeated groups, and file path spellings:

```json
{
  "groups": [
    {"mode": "rust", "file": "src/operations.rs", "results": {"compute": "sha256:..."}},
    {"mode": "file", "file": "inputs/request.json", "results": {"bytes": "sha256:..."}}
  ]
}
```

Each `results` object is identical to the same single-mode invocation.
`file FILE` is also a standalone subcommand: its `bytes` result is ordinary
SHA-256 of the raw bytes **as-is**, including whitespace, line endings, and
non-UTF-8 bytes. No format prefix is added to this raw-byte hash.

`multi --combine` prints one `sha256:` line in the `multi:combined` domain,
hashing, for each group in order, its mode, **file path as given**, and the
single-mode `combine(results)` digest, using the framing described below.
Reordering groups, moving content between groups, changing a path spelling, or
changing a selected result changes the combined digest. Query order within a
group is still irrelevant. This is a different digest scheme from single-mode
`--combine`; existing single-mode digests are unchanged.

Both `--combine` and `--allow-missing` are **global**, and may appear before or
after any group. Output is atomic: errors never write partial JSON or a digest.
Each distinct file is read once. All groups using the same file and parser mode
share one parse/index; requesting another mode uses that mode's parser, while
`file` only reads bytes. Canonicalized paths identify shared files (including
symlinks), but never replace the verbatim paths in the output or digest.

### Paths and missing files (all subcommands)

Relative paths resolve against the **current working directory**, not the
binary, configuration, or caller script's directory. CLI paths must be UTF-8.
Only a file-not-found error is treated as absence; other I/O errors and invalid
source files always fail, even with `--allow-missing`.

| Mode | Missing file |
| --- | --- |
| `rust` | Error unless `--allow-missing`; then queried as empty Rust. Unmatched `fn:`/`arm:` queries return null (using aliases if supplied); `branch:` keeps its normal stable empty-selection digest. |
| `toml` | Queried as an empty TOML document, like `/dev/null`; empty selections have stable non-null digests. |
| `json` | Queried as an empty JSON object `{}`; empty selections have stable non-null digests. An **existing zero-byte file is invalid JSON**, not `{}`. |
| `file` | SHA-256 of empty bytes, identical to an existing zero-byte file. |

Missing inputs still contribute their mode, path, and results in multi mode.
Adding selected content later changes the fingerprint. Malformed queries fail
even for missing inputs. `--allow-missing` also permits unmatched Rust queries in
existing files; TOML/JSON empty selections need no flag.

## Rust queries

```sh
rs-fn-fingerprint rust src/operations.rs 'fn:compute' 'fn:convert*'
rs-fn-fingerprint rust src/operations.rs 'arm:dispatch:42'
rs-fn-fingerprint rust src/operations.rs 'arm:dispatch:"ready"'
rs-fn-fingerprint rust src/operations.rs 'operations=fn:convert*' --combine
rs-fn-fingerprint rust src/operations.rs 'optional=fn:missing' --allow-missing
```

- `fn:<glob>` selects each matching function at file scope, in inline modules,
  or in `impl` blocks. Globs match function names, not qualified paths. Each
  function gets its own output key. Duplicate names use the module/impl path
  (`crate::compute`, `nested::compute`, `Worker::compute`, or
  `<Worker as Trait>::compute`). Repeated definitions at the same path get `#2`,
  `#3`, etc. External modules, trait declarations and block-local functions
  are not function-query candidates.
- `arm:<fn-glob>:<literal>` selects complete match arms inside matching functions,
  including their patterns, guards and bodies. Integer literals are compared
  by value (signed, decimal, hexadecimal, binary, octal and suffixed literals are accepted).
  Strings may be quoted Rust string literals, raw strings, or bare text.
  Or-patterns such as `41 | 42` match either literal; `42 if ready` matches `42`.
  A literal in a guard or body alone does not select an arm. Ranges match only
  explicitly written endpoints, not every value they contain.
- An arm's key is `<function-key>:<literal>`; integers use decimal and strings
  use their decoded value. Multiple matching arms in one function, including
  nested matches, are hashed together in source order.
- Each selection includes directly referenced same-file functions, constants,
  and statics (including block-local definitions), sorted by qualified name. Only single-segment paths count;
  `helper()` and `VALUE` count, while `module::helper()`, `Self::helper()`,
  and method calls do not. Macro input token trees are scanned for bare identifiers
  and matched only against same-file function/constant/static names, without
  expansion. Qualified paths, member calls, macro names and `$` metavariables are
  excluded; ambiguous bare names may conservatively include a same-named helper.
  References are collected syntactically,
  so shadowing is not resolved and all same-named definitions are included.
  Helpers' own references are **not** followed. `mod other;` never reads a file.
- Rust is parsed with `syn` (`full`) and hashed as `quote` / `proc-macro2` token
  strings. Whitespace, ordinary comments, and `#[doc]` attributes do not affect
  hashes. Other attributes, signatures and macro input tokens do.

An unmatched Rust query fails, with no JSON on stdout. With `--allow-missing`,
its output key is the original query (or its alias), with value `null`.

## Literal-owned branches

```sh
rs-fn-fingerprint rust src/rules.rs 'branch:ops:1000'
rs-fn-fingerprint rust src/rules.rs 'rule=branch:*:1000' --combine
```

`branch:<fn-glob>:<literal>` selects match arms whose pattern or guard contains
that literal and `if` / `else if` branches whose condition contains it. This is a
syntactic query, not dependency resolution. Macro condition tokens (`matches!`,
for example) are inspected without expansion. Body-only literals do not select a
branch. An `if` selection includes its condition and then-body, **not** sibling
`else` / `else if` bodies. Nested branches are inspected individually; a matching
branch in the condition takes precedence over the enclosing branch. Same-file
helpers follow the ordinary one-hop rule. Empty matches are valid stable hashes
for data-only rules; an invalid source or an I/O error other than an allowed missing file still fails.

## JSON queries

```sh
rs-fn-fingerprint json records.json 'field:id:42'
rs-fn-fingerprint json records.json 'field:name:"ready"'
rs-fn-fingerprint json records.json 'path:nodes.42' 'rows=path:rows.*'
```

`field:<field-name>:<JSON-value>` selects every object at any depth with that
field equal to the JSON value. `path:<dotted-glob>` selects values at matching
paths, useful for maps keyed by record id. Array indices are decimal path
segments; non-bare object keys are JSON-quoted, as for TOML. The root path is
empty. Each query hashes sorted `(path, canonical JSON value)` pairs. Object key
order and formatting do not matter; array order does. Empty matches have stable,
non-null hashes. Aliases and `--combine` work as in the other modes. Invalid JSON
or queries fail without partial output.

## TOML queries

```sh
rs-fn-fingerprint toml config.toml 'settings.100' 'variants.100.*'
rs-fn-fingerprint toml config.toml 'config=settings.100' 'extras=addons.100.*'
rs-fn-fingerprint --combine toml config.toml 'settings.*'
```

Each query hashes all matching tables as sorted `(dotted key, canonical JSON
value)` pairs. The JSON has recursively sorted object keys and preserves array
order. Comments, formatting, table order and key order do not matter.

Implicit parent tables and inline tables count; selecting a parent includes its
whole subtree. Arrays of tables are hashed as one ordered array at their dotted
key. Scalar keys are not tables. Non-bare key segments are JSON-quoted in dotted
paths, distinguishing `["a.b"]` from `[a.b]`. Datetimes use a
`{"$datetime":"..."}` JSON value; non-finite floats use `{"$float":"..."}`
instead of losing information as JSON null.

**An empty TOML match is valid:** it produces a stable, non-null hash. Adding a
matching table changes that hash. `--allow-missing` is not necessary.

## Output and aliases

```json
{
  "compute": "sha256:5ae7d0c183059d263acd87c94c6341ef915960f55287c6b45b0cb04ae2247251",
  "optional": null
}
```

Output is a JSON object with sorted keys. Quote globs to prevent shell expansion.
Globs support `*`, `?`, character classes and `{a,b}` alternatives; TOML wildcards
can span dots. An optional `name=` prefix chooses a query's output key. In Rust,
a single-match alias only renames the key; an alias with multiple matches
aggregates them into one digest. In TOML, each query already aggregates its
matching tables. Conflicting output keys are errors; repeating an
identical result is harmless.

`--combine` prints one plain `sha256:<hex>` line over all sorted result keys and
values, including nulls. It is independent of query order and suitable for a
fingerprint script. Flags may appear before or after the subcommand.

Every query or combined hash starts with `rs-fn-fingerprint/v1` followed by a NUL, then a
length-prefixed domain and length-prefixed UTF-8 fields (lengths are little-endian
64-bit byte counts). Modes and combined results are domain-separated. Single-mode file paths
are not hashed; multi combined results explicitly include them. Raw `file`
results are ordinary SHA-256, outside this framed format. Keep the tool and its dependency lockfile fixed when caching
fingerprints; token serialization is supplied by the parser/token dependencies.

## Library and checks

The `rs_fn_fingerprint` library exposes `fingerprint_rust`, `fingerprint_toml`, `fingerprint_json`,
`combine`, `Fingerprints`, and `Error`, plus `fingerprint_multi`, `combine_multi`,
`Mode`, `SourceGroup`, and `GroupFingerprint` for ordered source groups.
The single-mode library functions accept source text; missing-file handling belongs
to the CLI and `fingerprint_multi`.

```sh
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

MIT licensed. See [LICENSE](LICENSE).
