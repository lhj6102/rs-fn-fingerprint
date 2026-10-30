use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use rs_fn_fingerprint::{Mode, SourceGroup, combine, combine_multi, fingerprint_multi};

#[derive(Parser)]
#[command(
    version,
    about = "Fingerprint selected Rust, JSON, TOML or raw file sources without resolving dependencies",
    after_help = "Relative paths resolve against the current working directory. Missing TOML/JSON/file inputs are empty; missing Rust requires --allow-missing."
)]
struct Cli {
    /// Print one digest instead of a JSON object (multi preserves group order).
    #[arg(long, global = true)]
    combine: bool,
    /// Allow missing Rust files and return null for unmatched fn/arm queries.
    #[arg(long, global = true)]
    allow_missing: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Select functions (fn:), match arms (arm:) or literal-owned branches (branch:).
    Rust(Input),
    /// Select tables by dotted key glob. Missing files are empty TOML.
    Toml(Input),
    /// Select JSON objects by field equality or dotted path glob. Missing files are {}.
    Json(Input),
    /// Hash raw bytes as-is. Missing files are empty bytes.
    File(FileInput),
    /// Fingerprint ordered groups in one process, sharing reads and parsing.
    #[command(
        after_help = "GROUPS: --rust FILE QUERY... | --json FILE QUERY... | --toml FILE QUERY... | --file FILE\nRepeat group markers in any order. Queries support name= aliases and globs.\n--combine and --allow-missing apply globally, anywhere in the invocation.\nOutput: {\"groups\":[{\"mode\":\"rust\",\"file\":\"...\",\"results\":{...}}, ...]}\n--combine tags each group with its mode, verbatim path and results in input order.\nPaths resolve against the current working directory; prefix filenames starting with -- with ./."
    )]
    Multi(MultiInput),
}

#[derive(Args)]
struct Input {
    file: String,
    /// One or more queries, optionally prefixed with name= (quote shell globs).
    #[arg(required = true, num_args = 1..)]
    queries: Vec<String>,
}

#[derive(Args)]
struct FileInput {
    file: String,
}

#[derive(Args)]
struct MultiInput {
    /// Ordered --rust/--json/--toml FILE QUERY... or --file FILE groups.
    #[arg(required = true, num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true, value_name = "GROUPS")]
    groups: Vec<String>,
}

fn multi_groups(
    args: Vec<String>,
    combine: &mut bool,
    allow_missing: &mut bool,
) -> Result<Vec<SourceGroup>, String> {
    let mut groups: Vec<SourceGroup> = vec![];
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mode = match arg.as_str() {
            "--combine" => {
                *combine = true;
                continue;
            }
            "--allow-missing" => {
                *allow_missing = true;
                continue;
            }
            "--rust" => Some(Mode::Rust),
            "--json" => Some(Mode::Json),
            "--toml" => Some(Mode::Toml),
            "--file" => Some(Mode::File),
            _ => None,
        };
        if let Some(mode) = mode {
            let file = args
                .next()
                .filter(|file| !file.starts_with("--"))
                .ok_or_else(|| format!("{arg} requires a file"))?;
            groups.push(SourceGroup {
                mode,
                file,
                queries: vec![],
            });
        } else if arg.starts_with("--") {
            return Err(format!("unknown multi option: {arg}"));
        } else {
            let group = groups
                .last_mut()
                .ok_or("expected --rust, --json, --toml or --file")?;
            group.queries.push(arg);
        }
    }
    Ok(groups)
}

fn run(mut cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let is_multi = matches!(cli.command, Command::Multi(_));
    let groups = match cli.command {
        Command::Multi(input) => {
            multi_groups(input.groups, &mut cli.combine, &mut cli.allow_missing)?
        }
        command => {
            let (mode, file, queries) = match command {
                Command::Rust(input) => (Mode::Rust, input.file, input.queries),
                Command::Json(input) => (Mode::Json, input.file, input.queries),
                Command::Toml(input) => (Mode::Toml, input.file, input.queries),
                Command::File(input) => (Mode::File, input.file, vec![]),
                Command::Multi(_) => unreachable!(),
            };
            vec![SourceGroup {
                mode,
                file,
                queries,
            }]
        }
    };
    let results = fingerprint_multi(&groups, cli.allow_missing)?;
    let output = if is_multi {
        if cli.combine {
            combine_multi(&results)
        } else {
            let groups: Vec<_> = results
                .iter()
                .map(|group| {
                    serde_json::json!({
                        "mode": group.mode.as_str(),
                        "file": group.file,
                        "results": group.results,
                    })
                })
                .collect();
            serde_json::to_string_pretty(&serde_json::json!({"groups": groups}))?
        }
    } else if cli.combine {
        combine(&results[0].results)
    } else {
        serde_json::to_string_pretty(&results[0].results)?
    };
    use std::io::Write;
    writeln!(std::io::stdout().lock(), "{output}")?;
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
