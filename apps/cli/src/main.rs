//! `ferosnmp` command-line tool.
//!
//! Commands:
//!
//! - `ferosnmp mib show <name|oid>...`: everything known about a node.
//! - `ferosnmp mib tree [<root>]`: the OID tree below a node.
//! - `ferosnmp mib search <pattern>`: find nodes by name, with `*` and `?`.
//! - `ferosnmp lint <files|dirs>...`: check MIB files and report problems
//!   with file, line and column.
//!
//! Exit codes, so scripts and CI jobs can rely on them:
//!
//! - 0: success, or no problems found;
//! - 1: nothing found (`mib show`, `mib search`), or `lint` found problems;
//! - 2: the command could not run (bad arguments, unreadable files).

#![forbid(unsafe_code)]

mod lint;
mod mib;
mod sources;
mod wildcard;

use std::fmt;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::lint::{FailOn, Format};
use crate::sources::MibSources;

/// FeroSNMP: browse SNMP MIBs and check MIB files for problems.
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
            let set = sources.load()?;
            match command {
                MibCommand::Show { queries } => mib::show(&set, &queries, out),
                MibCommand::Tree { root, depth } => mib::tree(&set, root.as_deref(), depth, out),
                MibCommand::Search { pattern } => mib::search(&set, &pattern, out),
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
