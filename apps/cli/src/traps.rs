//! The `traps listen` command: receive traps and INFORMs and print them,
//! decoded with the MIBs.

use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::{Args, ValueEnum};
use mib_core::Strictness;
use snmp_core::{AuthProtocol, NotificationReceiver, PrivProtocol, TrapUser, Usm, hex, parse_hex};
use trap_rx::{Filter, Incoming, Listener, TRAP_PORT, describe, read_users};

use crate::query::Bytes;
use crate::sources::MibSources;
use crate::{CliError, Status};

/// Options of `traps listen`.
#[derive(Debug, Clone, Args)]
pub struct ListenArgs {
    /// UDP port to listen on. 162 is the standard port; it needs extra
    /// rights on Linux, and on Windows the SNMP Trap service must not be
    /// running. 1162 needs neither.
    #[arg(short = 'p', long, default_value_t = TRAP_PORT)]
    pub port: u16,

    /// Address to listen on: 0.0.0.0 for every IPv4 interface, :: for IPv6,
    /// or one interface's address.
    #[arg(long, default_value = "0.0.0.0")]
    pub address: IpAddr,

    /// Output format: text, or JSON Lines (one JSON object per line).
    #[arg(long, value_enum, default_value_t = TrapFormat::Text)]
    pub format: TrapFormat,

    /// Stop after this many notifications (after filtering).
    #[arg(long, value_name = "N")]
    pub count: Option<u64>,

    /// Only show notifications from this address.
    #[arg(long, value_name = "ADDRESS")]
    pub from: Option<IpAddr>,

    /// Only show notifications whose name matches, as in `mib search`
    /// (`link*`, `IF-MIB::*`), or whose OID is below a numeric OID.
    #[arg(long, value_name = "PATTERN")]
    pub notification: Option<String>,

    /// Print OIDs as numbers instead of names.
    #[arg(short = 'n', long)]
    pub numeric: bool,

    /// Do not report datagrams that are not valid notifications or fail
    /// the SNMPv3 checks.
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// File with SNMPv3 trap users as `createUser` lines, as in net-snmp's
    /// snmptrapd.conf (which can be given as it is); repeat for more files.
    #[arg(long = "users", value_name = "FILE")]
    pub users_files: Vec<PathBuf>,

    /// This receiver's SNMPv3 engine ID in hex, which INFORMs are sent to
    /// [default: a new random one at each start; senders discover it].
    #[arg(long, value_name = "HEX", value_parser = parse_engine_id)]
    pub local_engine_id: Option<Bytes>,

    #[command(flatten)]
    pub user: TrapUserArgs,

    #[command(flatten)]
    pub sources: MibSources,
}

/// One SNMPv3 trap user on the command line, named as in net-snmp's tools.
#[derive(Debug, Clone, Args)]
#[command(next_help_heading = "SNMPv3 trap user (or use --users)")]
pub struct TrapUserArgs {
    /// SNMPv3 user name.
    #[arg(short = 'u', long)]
    pub user: Option<String>,

    /// The sender's engine ID in hex. Without it the user is accepted from
    /// any sender.
    #[arg(short = 'e', long, value_name = "HEX", value_parser = parse_engine_id, requires = "user")]
    pub engine_id: Option<Bytes>,

    /// Authentication protocol: MD5, SHA (SHA-1), SHA-224, SHA-256,
    /// SHA-384 or SHA-512.
    #[arg(short = 'a', long, default_value = "SHA", value_parser = parse_from_str::<AuthProtocol>)]
    pub auth_protocol: AuthProtocol,

    /// Authentication password; without it, the user sends noAuthNoPriv.
    /// Or set FEROSNMP_AUTH_PASSWORD.
    #[arg(
        short = 'A',
        long,
        env = "FEROSNMP_AUTH_PASSWORD",
        hide_env_values = true
    )]
    pub auth_password: Option<String>,

    /// Privacy protocol: DES, AES (AES-128), AES-192, AES-256, AES-192C or
    /// AES-256C.
    #[arg(short = 'x', long, default_value = "AES", value_parser = parse_from_str::<PrivProtocol>)]
    pub priv_protocol: PrivProtocol,

    /// Privacy password; without it, the user sends at most authNoPriv. Or
    /// set FEROSNMP_PRIV_PASSWORD.
    #[arg(
        short = 'X',
        long,
        env = "FEROSNMP_PRIV_PASSWORD",
        hide_env_values = true
    )]
    pub priv_password: Option<String>,
}

/// How notifications are printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TrapFormat {
    /// A header line and one line per object.
    Text,
    /// JSON Lines: one JSON object per notification.
    Json,
}

fn parse_from_str<T: std::str::FromStr<Err = String>>(text: &str) -> Result<T, String> {
    text.parse()
}

/// Hex, with or without `0x` as net-snmp writes it.
fn parse_engine_id(text: &str) -> Result<Vec<u8>, String> {
    let digits = text.trim_start_matches("0x").trim_start_matches("0X");
    let id = parse_hex(digits)?;
    if !(5..=32).contains(&id.len()) {
        return Err("an engine ID has 5 to 32 bytes (RFC 3411)".to_string());
    }
    Ok(id)
}

impl TrapUserArgs {
    fn trap_user(&self) -> Result<Option<TrapUser>, CliError> {
        let Some(name) = &self.user else {
            return Ok(None);
        };
        let mut usm = Usm::new(name);
        if let Some(password) = &self.auth_password {
            usm = usm.with_auth(self.auth_protocol, password);
            if let Some(password) = &self.priv_password {
                usm = usm.with_privacy(self.priv_protocol, password);
            }
        } else if self.priv_password.is_some() {
            return Err(CliError::Message(
                "a privacy password needs an authentication password (-A) too".to_string(),
            ));
        }
        usm.check().map_err(CliError::Message)?;
        let user = TrapUser::new(usm);
        Ok(Some(match &self.engine_id {
            Some(id) => user.with_engine_id(id.clone()),
            None => user,
        }))
    }
}

/// Listen until `--count` notifications have been shown, or forever.
pub fn listen(args: &ListenArgs, out: &mut impl Write) -> Result<Status, CliError> {
    let mibs = args.sources.load_with(Strictness::Permissive)?;
    let mut users = Vec::new();
    for file in &args.users_files {
        users.extend(read_users(file).map_err(|e| CliError::Message(e.to_string()))?);
    }
    users.extend(args.user.trap_user()?);
    let user_count = users.len();
    let receiver = match &args.local_engine_id {
        Some(id) => NotificationReceiver::with_engine_id(id.clone(), users),
        None => NotificationReceiver::new(users),
    };
    let filter = Filter {
        source: args.from,
        notification: args.notification.clone(),
    };

    let address = SocketAddr::new(args.address, args.port);
    let listener = Listener::bind(address, receiver).map_err(|error| {
        let mut message = error.to_string();
        if let Some(hint) = error.hint() {
            message.push_str("\nferosnmp: hint: ");
            message.push_str(&hint);
        }
        CliError::Message(message)
    })?;
    let local = listener.local_addr()?;
    eprintln!(
        "ferosnmp: listening for traps and INFORMs on UDP {local}; SNMPv3 engine ID {}, {} \
         trap user{}; Ctrl+C stops",
        hex(listener.engine_id()),
        user_count,
        if user_count == 1 { "" } else { "s" }
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Message(format!("cannot start the network runtime: {e}")))?;
    runtime.block_on(async {
        let mut traps = listener.spawn()?;
        let mut shown = 0u64;
        if args.count == Some(0) {
            return Ok(Status::Success);
        }
        while let Some(incoming) = traps.next().await {
            match incoming {
                Incoming::Notification(received) => {
                    let described = describe(&received, Some(&mibs), args.numeric);
                    if !filter.matches(&described) {
                        continue;
                    }
                    match args.format {
                        TrapFormat::Text => write!(out, "{}", described.text())?,
                        TrapFormat::Json => writeln!(out, "{}", described.json())?,
                    }
                    // Show each one as it comes, also when piped.
                    out.flush()?;
                    shown += 1;
                    if args.count.is_some_and(|count| shown >= count) {
                        return Ok(Status::Success);
                    }
                }
                Incoming::Problem(problem) => {
                    if args.quiet {
                        continue;
                    }
                    eprintln!("ferosnmp: from {}: {}", problem.source, problem.error);
                    if let Some(hint) = problem.error.hint() {
                        eprintln!("ferosnmp: hint: {hint}");
                    }
                }
            }
        }
        Ok(Status::Success)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_ids_are_read_as_net_snmp_writes_them() {
        assert_eq!(
            parse_engine_id("0x80001F8804").unwrap(),
            [0x80, 0x00, 0x1f, 0x88, 0x04]
        );
        assert_eq!(parse_engine_id("8000000001").unwrap().len(), 5);
        assert!(parse_engine_id("0x8000").is_err());
        assert!(parse_engine_id("xyz").is_err());
    }
}
