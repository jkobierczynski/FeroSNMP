//! The `get`, `next`, `bulk` and `walk` commands: ask an SNMP agent.

use std::io::Write;
use std::time::Duration;

use clap::{Args, ValueEnum};
use mib_core::{MibSet, Oid, Strictness};
use snmp_core::{
    AuthProtocol, Engine, EngineConfig, Error, Event, Formatter, ObjectError, Operation,
    PrivProtocol, SecurityLevel, Target, Usm, Version, hex, parse_hex, raw_line,
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

    /// SNMP version [default: 3 with --user, else 2c].
    #[arg(short = 'v', long = "snmp-version", value_enum)]
    pub version: Option<SnmpVersion>,

    /// Community string (SNMPv1 and SNMPv2c).
    #[arg(short = 'c', long, default_value = "public")]
    pub community: String,

    #[command(flatten)]
    pub v3: V3Args,

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

/// SNMPv3 options, named as in net-snmp's tools.
#[derive(Debug, Clone, Args)]
#[command(next_help_heading = "SNMPv3 options")]
pub struct V3Args {
    /// SNMPv3 user name.
    #[arg(short = 'u', long)]
    pub user: Option<String>,

    /// Security level: noAuthNoPriv, authNoPriv or authPriv [default: from
    /// the passwords given].
    #[arg(short = 'l', long = "level", value_parser = parse_from_str::<SecurityLevel>)]
    pub level: Option<SecurityLevel>,

    /// Authentication protocol: MD5, SHA (SHA-1), SHA-224, SHA-256,
    /// SHA-384 or SHA-512.
    #[arg(short = 'a', long, default_value = "SHA", value_parser = parse_from_str::<AuthProtocol>)]
    pub auth_protocol: AuthProtocol,

    /// Authentication password (at least 8 characters). To keep it out of
    /// the shell history, set FEROSNMP_AUTH_PASSWORD instead.
    #[arg(
        short = 'A',
        long,
        env = "FEROSNMP_AUTH_PASSWORD",
        hide_env_values = true
    )]
    pub auth_password: Option<String>,

    /// Privacy protocol: DES, AES (AES-128), AES-192, AES-256, or AES-192C
    /// and AES-256C for Cisco's key extension.
    #[arg(short = 'x', long, default_value = "AES", value_parser = parse_from_str::<PrivProtocol>)]
    pub priv_protocol: PrivProtocol,

    /// Privacy password [default: the authentication password]. Or set
    /// FEROSNMP_PRIV_PASSWORD.
    #[arg(
        short = 'X',
        long,
        env = "FEROSNMP_PRIV_PASSWORD",
        hide_env_values = true
    )]
    pub priv_password: Option<String>,

    /// Context name.
    #[arg(long, default_value = "")]
    pub context: String,

    /// Context engine ID in hex, for agents that proxy others [default: the
    /// agent's own engine ID].
    #[arg(long, value_name = "HEX", value_parser = parse_hex)]
    pub context_engine_id: Option<Vec<u8>>,
}

fn parse_from_str<T: std::str::FromStr<Err = String>>(text: &str) -> Result<T, String> {
    text.parse()
}

impl V3Args {
    /// The SNMPv3 user settings.
    pub fn usm(&self) -> Result<Usm, CliError> {
        let user = self.user.clone().ok_or_else(|| {
            CliError::Message("SNMPv3 needs a user name; give it with -u".to_string())
        })?;
        let level = self
            .level
            .unwrap_or(match (&self.auth_password, &self.priv_password) {
                (Some(_), Some(_)) => SecurityLevel::AuthPriv,
                (Some(_), None) => SecurityLevel::AuthNoPriv,
                _ => SecurityLevel::NoAuthNoPriv,
            });
        if level.has_auth() && self.auth_password.is_none() {
            return Err(CliError::Message(format!(
                "security level {level} needs an authentication password; give it with -A \
                 or FEROSNMP_AUTH_PASSWORD"
            )));
        }
        let usm = Usm {
            user,
            level,
            auth_protocol: self.auth_protocol,
            auth_password: self.auth_password.clone().unwrap_or_default(),
            priv_protocol: self.priv_protocol,
            priv_password: self.priv_password.clone().unwrap_or_default(),
            context_name: self.context.clone(),
            context_engine_id: self.context_engine_id.clone(),
        };
        usm.check().map_err(CliError::Message)?;
        Ok(usm)
    }
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
    /// SNMPv3
    #[value(name = "3")]
    V3,
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
    /// SNMPv3 engine discovery.
    Discover,
}

/// Run a query command and print its results as they arrive.
pub fn run(args: &QueryArgs, query: Query, out: &mut impl Write) -> Result<Status, CliError> {
    let mibs = args.sources.load_with(Strictness::Permissive)?;
    let discover = matches!(query, Query::Discover);
    let operation = operation(&mibs, query)?;
    let version = match args.version {
        Some(version) => version,
        None if args.v3.user.is_some() || discover => SnmpVersion::V3,
        None => SnmpVersion::V2c,
    };
    let mut target = Target::parse(&args.agent)
        .map_err(|e| CliError::Message(e.to_string()))?
        .with_community(args.community.as_bytes())
        .with_timeout(Duration::from_secs_f64(args.timeout))
        .with_retries(args.retries);
    target = match version {
        SnmpVersion::V1 => target.with_version(Version::V1),
        SnmpVersion::V2c => target.with_version(Version::V2c),
        // Discovery works without a user; any name will do.
        SnmpVersion::V3 if discover && args.v3.user.is_none() => {
            target.with_usm(Usm::new("ferosnmp"))
        }
        SnmpVersion::V3 => target.with_usm(args.v3.usm()?),
    };

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
                Event::Engine(info) => {
                    if discover {
                        writeln!(out, "engine ID:    {}", hex(&info.engine_id))?;
                        writeln!(out, "engine boots: {}", info.boots)?;
                        writeln!(out, "engine time:  {} ({} s)", uptime(info.time), info.time)?;
                    } else if args.log {
                        eprintln!("ferosnmp: {info}");
                    }
                }
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
                        Error::Agent { .. }
                        | Error::NotIncreasing { .. }
                        | Error::Security { .. } => {
                            eprintln!("ferosnmp: {}", describe(&error, &formatter));
                            if let Some(hint) = error.hint() {
                                eprintln!("ferosnmp: hint: {hint}");
                            }
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
        Query::Discover => Operation::Discover,
    })
}

/// Seconds as `3 d 4 h 5 min 6 s`, leaving out leading zero units.
fn uptime(seconds: u32) -> String {
    let parts = [
        (seconds / 86_400, "d"),
        (seconds / 3600 % 24, "h"),
        (seconds / 60 % 60, "min"),
        (seconds % 60, "s"),
    ];
    let first = parts.iter().position(|(n, _)| *n > 0).unwrap_or(3);
    parts[first..]
        .iter()
        .map(|(n, unit)| format!("{n} {unit}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The OID for what the user typed; see [`snmp_core::resolve_object`].
pub fn resolve_object(mibs: &MibSet, text: &str) -> Result<Oid, CliError> {
    snmp_core::resolve_object(mibs, text).map_err(|error| {
        CliError::Message(match error {
            ObjectError::Unknown { .. } => {
                format!("{error} (add MIB folders with -M, or give a numeric OID)")
            }
            other => other.to_string(),
        })
    })
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

    #[test]
    fn uptime_text() {
        assert_eq!(uptime(0), "0 s");
        assert_eq!(uptime(61), "1 min 1 s");
        assert_eq!(uptime(90_061), "1 d 1 h 1 min 1 s");
    }
}
