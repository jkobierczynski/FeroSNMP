//! `ferosnmp` command-line tool.
//!
//! Commands:
//!
//! - `ferosnmp mib show <name|oid>...`: everything known about a node.
//! - `ferosnmp mib tree [<root>]`: the OID tree below a node.
//! - `ferosnmp mib search <pattern>`: find nodes by name, with `*` and `?`.
//! - `ferosnmp mib why <symbol>`: how a name resolves, or why it does not.
//! - `ferosnmp lint <files|dirs>...`: check MIB files and report problems
//!   with file, line and column.
//! - `ferosnmp get|next|bulk <agent> <objects>...` and
//!   `ferosnmp walk <agent> [<root>]`: ask an SNMP agent (v1, v2c or v3).
//! - `ferosnmp discover <agent>`: an SNMPv3 agent's engine ID, boots and
//!   time.
//!
//! Exit codes, so scripts and CI jobs can rely on them:
//!
//! - 0: success, or no problems found;
//! - 1: nothing found (`mib show`, `mib search`), a name does not resolve
//!   (`mib why`), `lint` found problems, or the agent answered with an
//!   error or with "no such object" (`get`, `next`, `bulk`, `walk`);
//! - 2: the command could not run (bad arguments, unreadable files, no
//!   response from the agent).

#![forbid(unsafe_code)]

mod lint;
mod mib;
mod query;
mod sources;

use std::fmt;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use mib_core::Strictness;

use crate::lint::{FailOn, Format};
use crate::query::{Query, QueryArgs};
use crate::sources::MibSources;

/// FeroSNMP: browse SNMP MIBs, check MIB files for problems and query SNMP
/// agents.
#[derive(Debug, Parser)]
#[command(name = "ferosnmp", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Look up, browse and search the loaded MIBs.
    Mib {
        #[command(flatten)]
        sources: MibSources,

        #[command(subcommand)]
        command: MibCommand,
    },

    /// Check MIB files for problems and report them with file, line and column.
    ///
    /// Exits with 1 when a problem at or above the --fail-on level is found,
    /// so it can be used in CI.
    Lint {
        /// MIB files, or folders to check every MIB file in (recursively).
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// No guessing: report every rule violation, including style problems.
        #[arg(long)]
        strict: bool,

        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,

        /// Lowest severity that makes the exit code 1.
        #[arg(long, value_enum, default_value_t = FailOn::Error, value_name = "SEVERITY")]
        fail_on: FailOn,

        #[command(flatten)]
        sources: MibSources,
    },

    /// Read objects from an agent with one GET request.
    ///
    /// Example: ferosnmp get 192.0.2.1 sysDescr.0 sysUpTime.0
    Get {
        #[command(flatten)]
        query: QueryArgs,

        /// Instances to read: `sysDescr.0`, `IF-MIB::ifDescr.3` or a numeric
        /// OID.
        #[arg(required = true, value_name = "OBJECT")]
        objects: Vec<String>,
    },

    /// Read the instance that follows each object (GETNEXT).
    ///
    /// Example: ferosnmp next 192.0.2.1 sysDescr ifDescr.3
    Next {
        #[command(flatten)]
        query: QueryArgs,

        /// Objects or instances to start after.
        #[arg(required = true, value_name = "OBJECT")]
        objects: Vec<String>,
    },

    /// Read many instances in one GETBULK request (SNMPv2c).
    ///
    /// The first --non-repeaters objects get one successor each, like
    /// `next`; each other object gets up to --max-repetitions successors.
    Bulk {
        #[command(flatten)]
        query: QueryArgs,

        /// Objects or instances to start after.
        #[arg(required = true, value_name = "OBJECT")]
        objects: Vec<String>,

        /// How many objects, from the first, get only one successor.
        #[arg(long, default_value_t = 0, value_name = "N")]
        non_repeaters: u32,

        /// Successors for each of the other objects.
        #[arg(long, default_value_t = snmp_core::DEFAULT_MAX_REPETITIONS, value_name = "N")]
        max_repetitions: u32,
    },

    /// Read every instance below an object (GETBULK with SNMPv2c, GETNEXT
    /// with SNMPv1).
    ///
    /// Example: ferosnmp walk 192.0.2.1 system
    Walk {
        #[command(flatten)]
        query: QueryArgs,

        /// Where to start; without it, all of mib-2 (1.3.6.1.2.1).
        #[arg(value_name = "OBJECT")]
        root: Option<String>,

        /// Instances per GETBULK request.
        #[arg(long, default_value_t = snmp_core::DEFAULT_MAX_REPETITIONS, value_name = "N")]
        max_repetitions: u32,

        /// Use GETNEXT also with SNMPv2c, for agents with a broken GETBULK.
        #[arg(long)]
        no_bulk: bool,
    },

    /// Show an SNMPv3 agent's engine ID, boots and time.
    ///
    /// Needs no user; with -u and passwords, the credentials are not
    /// checked yet (use `get` for that).
    ///
    /// Example: ferosnmp discover 192.0.2.1
    Discover {
        #[command(flatten)]
        query: QueryArgs,
    },
}

#[derive(Debug, Subcommand)]
enum MibCommand {
    /// Show everything known about a node: OID, type, access, description...
    Show {
        /// Name (sysDescr), qualified name (SNMPv2-MIB::sysDescr), numeric
        /// OID (1.3.6.1.2.1.1.1) or instance (ifDescr.7).
        #[arg(required = true, value_name = "NAME|OID")]
        queries: Vec<String>,
    },

    /// Print the OID tree, starting at a node or at the top.
    Tree {
        /// Node to start at, as for `mib show`. Without it, the whole tree.
        #[arg(value_name = "ROOT")]
        root: Option<String>,

        /// Only print this many levels below the start.
        #[arg(long, value_name = "LEVELS")]
        depth: Option<usize>,
    },

    /// Find nodes by name. `*` matches any text and `?` one character;
    /// without wildcards, every name containing the pattern matches.
    /// Case is ignored.
    Search {
        /// Pattern such as `if*Octets`, or `IF-MIB::*` to search one module.
        pattern: String,
    },

    /// Explain how a name resolves, or why it does not: the module's own
    /// definitions, its IMPORTS, the fallbacks the strictness allows, and
    /// every module that defines the name.
    Why {
        /// `MODULE::name` to follow a reference written in MODULE (as the
        /// loader does), or just `name` to list every definition.
        #[arg(value_name = "[MODULE::]SYMBOL")]
        symbol: String,

        /// Resolve as `ferosnmp lint` does (normal strictness) instead of
        /// permissively, as the browser does.
        #[arg(long)]
        lint: bool,

        /// Resolve with strict rules: no fallbacks at all.
        #[arg(long, conflicts_with = "lint")]
        strict: bool,
    },
}

/// How a command ended, when it did run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Everything asked for was found, or no problems were found.
    Success,
    /// Something was not found, or `lint` found problems.
    Problems,
}

/// Why a command could not run.
#[derive(Debug)]
pub enum CliError {
    /// Writing the output failed.
    Io(io::Error),
    /// Anything else, as a message for the user.
    Message(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Io(error) => write!(f, "cannot write output: {error}"),
            CliError::Message(message) => f.write_str(message),
        }
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        CliError::Io(error)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Output is buffered and written in one go at the end; that is much faster
    // than printing line by line when a tree has thousands of lines.
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let result = run(cli.command, &mut out).and_then(|status| {
        out.flush()?;
        Ok(status)
    });

    match result {
        Ok(Status::Success) => ExitCode::from(0),
        Ok(Status::Problems) => ExitCode::from(1),
        // `ferosnmp mib tree | head` closes the pipe early; that is not an
        // error worth reporting.
        Err(CliError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::from(0),
        Err(error) => {
            // Show what was printed before the error, then the error itself.
            let _ = out.flush();
            eprintln!("ferosnmp: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(command: Command, out: &mut impl Write) -> Result<Status, CliError> {
    match command {
        Command::Mib { sources, command } => {
            let strictness = match &command {
                MibCommand::Why { strict: true, .. } => Strictness::Strict,
                MibCommand::Why { lint: true, .. } => Strictness::Normal,
                _ => Strictness::Permissive,
            };
            let set = sources.load_with(strictness)?;
            match command {
                MibCommand::Show { queries } => mib::show(&set, &queries, out),
                MibCommand::Tree { root, depth } => mib::tree(&set, root.as_deref(), depth, out),
                MibCommand::Search { pattern } => mib::search(&set, &pattern, out),
                MibCommand::Why { symbol, .. } => mib::why(&set, &symbol, out),
            }
        }
        Command::Lint {
            paths,
            strict,
            format,
            fail_on,
            sources,
        } => {
            let options = lint::Options {
                paths,
                strict,
                format,
                fail_on,
                sources,
            };
            lint::run(&options, out)
        }
        Command::Get { query, objects } => query::run(&query, Query::Get(objects), out),
        Command::Next { query, objects } => query::run(&query, Query::Next(objects), out),
        Command::Bulk {
            query,
            objects,
            non_repeaters,
            max_repetitions,
        } => query::run(
            &query,
            Query::Bulk {
                objects,
                non_repeaters,
                max_repetitions,
            },
            out,
        ),
        Command::Walk {
            query,
            root,
            max_repetitions,
            no_bulk,
        } => query::run(
            &query,
            Query::Walk {
                root,
                max_repetitions,
                bulk: !no_bulk,
            },
            out,
        ),
        Command::Discover { query } => query::run(&query, Query::Discover, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_line_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
