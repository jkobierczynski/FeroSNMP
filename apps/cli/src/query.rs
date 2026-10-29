//! The `get`, `next`, `bulk` and `walk` commands: ask an SNMP agent.

use std::io::Write;
use std::time::Duration;

use clap::{Args, ValueEnum};
use mib_core::{MibSet, Oid, Strictness};
use snmp_core::{
    Engine, EngineConfig, Error, Event, Formatter, Operation, Target, Version, raw_line,
};

use crate::sources::MibSources;
use crate::{CliError, Status};

/// Options every query command takes.
#[derive(Debug, Clone, Args)]
pub struct QueryArgs {
    /// The agent: host name or address, optionally with `:port` (default
    /// 161). IPv6 with a port: `[2001:db8::1]:161`.
    #[arg(value_name = "AGENT")]
    pub agent: String,

    /// SNMP version.
    #[arg(short = 'v', long = "snmp-version", value_enum, default_value_t = SnmpVersion::V2c)]
    pub version: SnmpVersion,

    /// Community string.
    #[arg(short = 'c', long, default_value = "public")]
    pub community: String,

    /// Seconds to wait for each response.
    #[arg(short = 't', long, default_value_t = 2.0, value_name = "SECONDS",
          value_parser = parse_timeout)]
    pub timeout: f64,

    /// How often to send a request again when no response arrives.
    #[arg(short = 'r', long, default_value_t = 2)]
    pub retries: u32,

    /// Print OIDs as numbers instead of names.
    #[arg(short = 'n', long)]
    pub numeric: bool,

    /// Print numeric OIDs, ASN.1 tags and the bytes of each value in hex.
    #[arg(long)]
    pub raw: bool,

    /// Print every request to standard error, with its timing.
    #[arg(long)]
    pub log: bool,

    #[command(flatten)]
    pub sources: MibSources,
}

/// The SNMP versions on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SnmpVersion {
    /// SNMPv1
    #[value(name = "1")]
    V1,
    /// SNMPv2c
    #[value(name = "2c")]
    V2c,
}

fn parse_timeout(text: &str) -> Result<f64, String> {
    match text.parse::<f64>() {
        Ok(seconds) if seconds > 0.0 && seconds <= 3600.0 => Ok(seconds),
        _ => Err("a number of seconds above 0, such as 2 or 0.5".to_string()),
    }
}

/// The OID `walk` starts at when none is given: mib-2.
pub const DEFAULT_WALK_ROOT: &str = "1.3.6.1.2.1";

/// What to ask for, with the objects still as typed.
#[derive(Debug)]
pub enum Query {
    Get(Vec<String>),
    Next(Vec<String>),
    Bulk {
        objects: Vec<String>,
        non_repeaters: u32,
        max_repetitions: u32,
    },
    Walk {
        root: Option<String>,
        max_repetitions: u32,
        bulk: bool,
    },
}

/// Run a query command and print its results as they arrive.
pub fn run(args: &QueryArgs, query: Query, out: &mut impl Write) -> Result<Status, CliError> {
    let mibs = args.sources.load_with(Strictness::Permissive)?;
    let operation = operation(&mibs, query)?;
    let target = Target::parse(&args.agent)
        .map_err(|e| CliError::Message(e.to_string()))?
        .with_version(match args.version {
            SnmpVersion::V1 => Version::V1,
            SnmpVersion::V2c => Version::V2c,
        })
        .with_community(args.community.as_bytes())
        .with_timeout(Duration::from_secs_f64(args.timeout))
        .with_retries(args.retries);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Message(format!("cannot start the network runtime: {e}")))?;
    let mut formatter = Formatter::new(Some(&mibs)).with_numeric_oids(args.numeric);

    runtime.block_on(async {
        let engine = Engine::start(EngineConfig::default());
        let mut running = engine
            .submit(target, operation)
            .map_err(|e| CliError::Message(e.to_string()))?;
        let mut status = Status::Success;
        while let Some(event) = running.next_event().await {
            match event {
                Event::Varbinds(varbinds) => {
                    for varbind in &varbinds {
                        if varbind.value.is_exception() {
                            status = Status::Problems;
                        }
                        let line = if args.raw {
                            raw_line(varbind)
                        } else {
                            formatter.line(varbind)
                        };
                        writeln!(out, "{line}")?;
                    }
                    // Show results as they come, also when piped.
                    out.flush()?;
                }
                Event::Log(entry) => {
                    if args.log {
                        eprintln!("ferosnmp: {entry}");
                    }
                }
                Event::Notice(text) => eprintln!("ferosnmp: note: {text}"),
                Event::Finished(summary) => {
                    if args.log {
                        eprintln!(
                            "ferosnmp: done: {} varbind{}, {} request{}, {:.1} ms",
                            summary.varbinds,
                            if summary.varbinds == 1 { "" } else { "s" },
                            summary.requests,
                            if summary.requests == 1 { "" } else { "s" },
                            summary.elapsed.as_secs_f64() * 1000.0
                        );
                    }
                }
                Event::Failed(error) => {
                    out.flush()?;
                    return match error {
                        // The agent answered, but said no: like "not found".
                        Error::Agent { .. } | Error::NotIncreasing { .. } => {
                            eprintln!("ferosnmp: {}", describe(&error, &formatter));
                            Ok(Status::Problems)
                        }
                        other => Err(CliError::Message(other.to_string())),
                    };
                }
            }
        }
        Ok(status)
    })
}

fn operation(mibs: &MibSet, query: Query) -> Result<Operation, CliError> {
    let oids = |objects: &[String]| -> Result<Vec<Oid>, CliError> {
        objects.iter().map(|o| resolve_object(mibs, o)).collect()
    };
    Ok(match query {
        Query::Get(objects) => Operation::Get(oids(&objects)?),
        Query::Next(objects) => Operation::GetNext(oids(&objects)?),
        Query::Bulk {
            objects,
            non_repeaters,
            max_repetitions,
        } => Operation::GetBulk {
            oids: oids(&objects)?,
            non_repeaters,
            max_repetitions,
        },
        Query::Walk {
            root,
            max_repetitions,
            bulk,
        } => Operation::Walk {
            root: resolve_object(mibs, root.as_deref().unwrap_or(DEFAULT_WALK_ROOT))?,
            max_repetitions,
            bulk,
        },
    })
}

/// The OID for what the user typed: a numeric OID (`1.3.6.1.2.1.1.1.0`), a
/// name (`sysDescr`), `MODULE::name`, each optionally followed by an
/// instance suffix (`ifDescr.3`, `IF-MIB::ifDescr.3`).
pub fn resolve_object(mibs: &MibSet, text: &str) -> Result<Oid, CliError> {
    let text = text.trim();
    if let Ok(oid) = text.parse::<Oid>() {
        return Ok(oid);
    }
    let (module, rest) = match text.split_once("::") {
        Some((module, rest)) => (Some(module), rest),
        None => (None, text),
    };
    let (name, suffix) = match rest.split_once('.') {
        Some((name, suffix)) => (name, Some(suffix)),
        None => (rest, None),
    };
    let lookup = match module {
        Some(module) => format!("{module}::{name}"),
        None => name.to_string(),
    };
    let node = mibs.find(&lookup).ok_or_else(|| {
        CliError::Message(format!(
            "unknown object {text:?}: no loaded MIB defines {lookup} \
             (add MIB folders with -M, or give a numeric OID)"
        ))
    })?;
    let mut arcs = node.oid().arcs().to_vec();
    if let Some(suffix) = suffix {
        let suffix: Oid = suffix.parse().map_err(|_| {
            CliError::Message(format!(
                "{text:?}: the instance after the name must be numbers separated by dots, \
                 such as ifDescr.3"
            ))
        })?;
        arcs.extend_from_slice(suffix.arcs());
    }
    Ok(Oid::new(arcs))
}

/// An error, with OIDs shown by name.
fn describe(error: &Error, formatter: &Formatter<'_>) -> String {
    match error {
        Error::Agent {
            target,
            status,
            oid: Some(oid),
            ..
        } => format!("{target} answered {status} for {}", formatter.oid(oid)),
        Error::NotIncreasing { previous, next } => format!(
            "the agent returned {} after {}; OIDs must increase, so the walk stopped \
             here to avoid an endless loop (the agent is broken)",
            formatter.oid(next),
            formatter.oid(previous)
        ),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mib_core::MibLibrary;

    #[test]
    fn objects_resolve_to_oids() {
        let mibs = MibLibrary::new().with_system_folders(false).load().unwrap();
        let oid = |text: &str| resolve_object(&mibs, text).map(|o| o.to_string());
        assert_eq!(oid("1.3.6.1.2.1.1.1.0").unwrap(), "1.3.6.1.2.1.1.1.0");
        assert_eq!(oid(".1.3.6").unwrap(), "1.3.6");
        assert_eq!(oid("enterprises").unwrap(), "1.3.6.1.4.1");
        assert_eq!(oid("enterprises.9.1").unwrap(), "1.3.6.1.4.1.9.1");
        assert_eq!(oid("SNMPv2-SMI::mib-2.1").unwrap(), "1.3.6.1.2.1.1");
        assert!(oid("noSuchThing.0").is_err());
        assert!(oid("enterprises.x").is_err());
        assert!(oid("NO-SUCH-MIB::enterprises").is_err());
    }
}
