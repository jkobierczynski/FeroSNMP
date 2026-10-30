//! What the Traps tab shows: the notifications received while listening,
//! decoded with the loaded MIBs, as text for a virtual list view, the
//! clipboard, and CSV and JSON files.
//!
//! At most [`LIMIT`] notifications are kept; when more arrive, the oldest
//! are dropped. The filters choose which of them the list shows. Each
//! notification keeps a number (`seq`) that only grows, so the rows the
//! filters let through can be remembered by number, and dropping the
//! oldest notifications only removes rows from the top.

use std::collections::VecDeque;
use std::fmt::Write;

use mib_core::MibSet;
use snmp_core::NotificationKind;
use trap_rx::{Described, Filter, Received, describe};

use crate::results::{csv_field, one_line};

/// How many notifications are kept.
pub const LIMIT: usize = 10_000;

/// A column of the Traps list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapColumn {
    /// When it arrived, in local time: `2026-10-29 14:03:22`.
    Time,
    /// The sender: `192.0.2.1:50123`.
    Source,
    /// `v2c trap`, `v3 inform`, ...
    Type,
    /// `IF-MIB::linkDown`, or the numeric OID.
    Notification,
    /// The value of the notification's severity object, if it has one.
    Severity,
    /// The objects on one line: `ifIndex.2 = 2, ifAdminStatus.2 = down(2)`.
    Objects,
}

impl TrapColumn {
    /// Every column, in display order.
    pub const ALL: [TrapColumn; 6] = [
        TrapColumn::Time,
        TrapColumn::Source,
        TrapColumn::Type,
        TrapColumn::Notification,
        TrapColumn::Severity,
        TrapColumn::Objects,
    ];

    /// The column heading.
    pub fn title(self) -> &'static str {
        match self {
            TrapColumn::Time => "Time",
            TrapColumn::Source => "From",
            TrapColumn::Type => "Type",
            TrapColumn::Notification => "Notification",
            TrapColumn::Severity => "Severity",
            TrapColumn::Objects => "Objects",
        }
    }

    /// Width at 96 DPI.
    pub fn width(self) -> i32 {
        match self {
            TrapColumn::Time => 140,
            TrapColumn::Source => 140,
            TrapColumn::Type => 80,
            TrapColumn::Notification => 240,
            TrapColumn::Severity => 90,
            TrapColumn::Objects => 520,
        }
    }
}

/// The filters above the Traps list. An empty field lets everything
/// through; each field that is filled in must match. Case is ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrapFilter {
    /// Part of the sender's address, such as `192.0.2.` (for SNMPv1 traps
    /// also the agent address in the trap).
    pub source: String,
    /// The notification's name as `ferosnmp traps listen --notification`
    /// takes it: `link*`, `IF-MIB::*`, `down`, or an OID prefix such as
    /// `1.3.6.1.4.1.9`.
    pub notification: String,
    /// Part of the severity text, such as `crit`.
    pub severity: String,
    /// Text anywhere in the notification: time, names, values, community,
    /// user.
    pub text: String,
}

impl TrapFilter {
    fn matches(&self, entry: &TrapEntry) -> bool {
        let contains = |haystack: &str, needle: &str| {
            let needle = needle.trim().to_lowercase();
            needle.is_empty() || haystack.to_lowercase().contains(&needle)
        };
        let described = &entry.described;
        let source = !self.source.trim().is_empty() && {
            let agent = described.v1.as_ref().map(|v1| v1.agent_address.as_str());
            !contains(&described.source.to_string(), &self.source)
                && !agent.is_some_and(|agent| contains(agent, &self.source))
        };
        if source {
            return false;
        }
        let notification = self.notification.trim();
        let by_name = Filter {
            notification: Some(notification.to_string()),
            ..Filter::default()
        };
        if !notification.is_empty() && !by_name.matches(described) {
            return false;
        }
        contains(&entry.severity, &self.severity)
            && (self.text.trim().is_empty()
                || entry.haystack.contains(&self.text.trim().to_lowercase()))
    }
}

/// One received notification.
#[derive(Debug)]
struct TrapEntry {
    seq: u64,
    /// When it arrived, in local time, as the list shows it.
    local_time: String,
    received: Received,
    described: Described,
    severity: String,
    objects: String,
    /// Everything in the notification as lower-case text, for the text
    /// filter.
    haystack: String,
}

impl TrapEntry {
    fn new(seq: u64, local_time: String, received: Received, mibs: Option<&MibSet>) -> Self {
        let described = describe(&received, mibs, false);
        let mut entry = TrapEntry {
            seq,
            local_time,
            received,
            described,
            severity: String::new(),
            objects: String::new(),
            haystack: String::new(),
        };
        entry.refresh_text();
        entry
    }

    /// Describe the notification again with other MIBs.
    fn redescribe(&mut self, mibs: Option<&MibSet>) {
        self.described = describe(&self.received, mibs, false);
        self.refresh_text();
    }

    fn refresh_text(&mut self) {
        self.severity = severity(&self.described);
        self.objects = self
            .described
            .objects
            .iter()
            .map(|object| format!("{} = {}", bare(&object.name), object.value))
            .collect::<Vec<_>>()
            .join(", ");
        self.haystack = format!("{} {}", self.local_time, self.described.text()).to_lowercase();
    }

    fn cell(&self, column: TrapColumn) -> String {
        let described = &self.described;
        match column {
            TrapColumn::Time => self.local_time.clone(),
            TrapColumn::Source => described.source.to_string(),
            TrapColumn::Type => type_text(described),
            TrapColumn::Notification => described.name.clone(),
            TrapColumn::Severity => self.severity.clone(),
            TrapColumn::Objects => one_line(&self.objects),
        }
    }
}

/// `v2c trap`, `v3 inform`.
fn type_text(described: &Described) -> String {
    format!("v{} {}", described.version, described.kind)
}

/// A name without its module: `ifIndex.2` for `IF-MIB::ifIndex.2`.
fn bare(name: &str) -> &str {
    name.rsplit_once("::").map_or(name, |(_, bare)| bare)
}

/// The severity of a notification, as far as one can tell: the value of the
/// first object whose name says it is a severity (`ciscoEnvMonSeverity`,
/// `alarmSeverity`, ...). Most vendor alarm MIBs have one; standard
/// notifications such as linkDown do not, and get an empty text.
fn severity(described: &Described) -> String {
    described
        .objects
        .iter()
        .find(|object| bare(&object.name).to_lowercase().contains("severity"))
        .map(|object| object.value.clone())
        .unwrap_or_default()
}

/// What changed in the list of shown rows after notifications arrived.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Change {
    /// Rows were added at the end.
    pub added: usize,
    /// Rows were dropped from the top, because the buffer was full.
    pub dropped: usize,
}

/// The notifications received, and the ones the filters show.
#[derive(Debug, Default)]
pub struct TrapList {
    entries: VecDeque<TrapEntry>,
    /// The seq of the next notification.
    next_seq: u64,
    /// The seqs of the shown rows, in order.
    shown: VecDeque<u64>,
    filter: TrapFilter,
    mibs: Option<MibSet>,
}

impl TrapList {
    /// Add a notification that arrived at `local_time`, dropping the oldest
    /// one when the list is full.
    pub fn push(&mut self, received: Received, local_time: String) -> Change {
        let mut change = Change::default();
        if self.entries.len() == LIMIT
            && let Some(oldest) = self.entries.pop_front()
            && self.shown.front() == Some(&oldest.seq)
        {
            self.shown.pop_front();
            change.dropped += 1;
        }
        let entry = TrapEntry::new(self.next_seq, local_time, received, self.mibs.as_ref());
        self.next_seq += 1;
        if self.filter.matches(&entry) {
            self.shown.push_back(entry.seq);
            change.added += 1;
        }
        self.entries.push_back(entry);
        change
    }

    /// Forget every notification.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.shown.clear();
    }

    /// Number of rows shown.
    pub fn len(&self) -> usize {
        self.shown.len()
    }

    /// True if no rows are shown.
    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    /// Number of notifications kept, shown or not.
    pub fn total(&self) -> usize {
        self.entries.len()
    }

    /// The filters in use.
    pub fn filter(&self) -> &TrapFilter {
        &self.filter
    }

    /// Show only the notifications `filter` lets through.
    pub fn set_filter(&mut self, filter: TrapFilter) {
        self.filter = filter;
        self.refilter();
    }

    /// Decode every notification again with newly loaded MIBs.
    pub fn set_mibs(&mut self, mibs: Option<MibSet>) {
        self.mibs = mibs;
        for entry in &mut self.entries {
            entry.redescribe(self.mibs.as_ref());
        }
        self.refilter();
    }

    fn refilter(&mut self) {
        self.shown = self
            .entries
            .iter()
            .filter(|entry| self.filter.matches(entry))
            .map(|entry| entry.seq)
            .collect();
    }

    fn entry(&self, row: usize) -> Option<&TrapEntry> {
        let seq = *self.shown.get(row)?;
        let first = self.entries.front()?.seq;
        self.entries
            .get(usize::try_from(seq.checked_sub(first)?).ok()?)
    }

    /// The text of one cell.
    pub fn cell(&self, row: usize, column: TrapColumn) -> String {
        self.entry(row)
            .map(|entry| entry.cell(column))
            .unwrap_or_default()
    }

    /// The decoded notification in a row.
    pub fn described(&self, row: usize) -> Option<&Described> {
        self.entry(row).map(|entry| &entry.described)
    }

    /// The whole notification in a row, as `ferosnmp traps listen` prints
    /// it, with the local time first.
    pub fn details(&self, row: usize) -> Option<String> {
        let entry = self.entry(row)?;
        Some(format!(
            "Received {} (local time)\n{}",
            entry.local_time,
            entry.described.text()
        ))
    }

    /// The tab title: `Traps (42)`, or `Traps (42 of 1,000)` when filtered.
    pub fn title(&self) -> String {
        use crate::results::count_text;
        match (self.total(), self.len()) {
            (0, _) => "Traps".to_string(),
            (total, shown) if total == shown => format!("Traps ({})", count_text(total)),
            (total, shown) => format!("Traps ({} of {})", count_text(shown), count_text(total)),
        }
    }

    /// Some rows as tab-separated lines, for Ctrl+C.
    pub fn tab_separated(&self, rows: impl IntoIterator<Item = usize>) -> String {
        let mut text = String::new();
        for entry in rows.into_iter().filter_map(|row| self.entry(row)) {
            let cells: Vec<String> = TrapColumn::ALL
                .iter()
                .map(|&column| one_line(&entry.cell(column)))
                .collect();
            text.push_str(&cells.join("\t"));
            text.push_str("\r\n");
        }
        text
    }

    /// The first shown row after `from` (or before it, `backwards`) whose
    /// text contains `query`, ignoring case; wraps around.
    pub fn find(&self, query: &str, from: Option<usize>, backwards: bool) -> Option<usize> {
        let query = query.trim().to_lowercase();
        let count = self.len();
        if query.is_empty() || count == 0 {
            return None;
        }
        let start = match (from, backwards) {
            (Some(row), false) => row + 1,
            (Some(row), true) => row + count - 1,
            (None, false) => 0,
            (None, true) => count - 1,
        };
        (0..count)
            .map(|step| {
                if backwards {
                    (start + count - step) % count
                } else {
                    (start + step) % count
                }
            })
            .find(|&row| {
                self.entry(row)
                    .is_some_and(|entry| entry.haystack.contains(&query))
            })
    }

    /// The shown rows as CSV (RFC 4180), one row per notification, with the
    /// time in UTC.
    pub fn csv(&self) -> String {
        let mut text = String::from(
            "time,source,version,type,notification,oid,severity,community,user,objects\r\n",
        );
        for row in 0..self.len() {
            let Some(entry) = self.entry(row) else {
                continue;
            };
            let described = &entry.described;
            let objects = described
                .objects
                .iter()
                .map(|object| format!("{} = {}", object.name, object.value))
                .collect::<Vec<_>>()
                .join("; ");
            let user = described
                .user
                .as_ref()
                .map(|(user, level, _)| format!("{user} {level}"))
                .unwrap_or_default();
            let fields = [
                described.time.clone(),
                described.source.to_string(),
                described.version.to_string(),
                described.kind.to_string(),
                described.name.clone(),
                described
                    .oid
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                entry.severity.clone(),
                described.community.clone().unwrap_or_default(),
                user,
                objects,
            ];
            let fields: Vec<String> = fields.iter().map(|field| csv_field(field)).collect();
            let _ = write!(text, "{}\r\n", fields.join(","));
        }
        text
    }

    /// The shown rows as a JSON array, each notification as
    /// `ferosnmp traps listen --format json` writes it.
    pub fn json(&self) -> String {
        let rows: Vec<String> = (0..self.len())
            .filter_map(|row| self.entry(row))
            .map(|entry| format!("  {}", entry.described.json()))
            .collect();
        if rows.is_empty() {
            "[]\n".to_string()
        } else {
            format!("[\n{}\n]\n", rows.join(",\n"))
        }
    }
}

/// `Inform` or `Trap`, capitalised, for window titles.
pub fn kind_title(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::Trap => "Trap",
        NotificationKind::Inform => "INFORM",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{recorded, trap_mibs};
    use snmp_core::NotificationReceiver;
    use std::time::{Duration, UNIX_EPOCH};

    fn received(name: &str, source: &str) -> Received {
        let mut receiver = NotificationReceiver::new(vec![]);
        let notification = receiver
            .handle(&recorded(name))
            .result
            .unwrap()
            .expect("a notification");
        Received {
            at: UNIX_EPOCH + Duration::from_secs(1_793_311_285),
            source: source.parse().unwrap(),
            notification,
            acknowledged: false,
        }
    }

    fn list() -> TrapList {
        let mut list = TrapList::default();
        list.set_mibs(Some(trap_mibs()));
        list.push(
            received("v2c-port-change", "192.0.2.1:5000"),
            "2026-10-29 23:01:25".into(),
        );
        list.push(
            received("v1-link-down", "192.0.2.2:5001"),
            "2026-10-29 23:01:26".into(),
        );
        list.push(
            received("v2c-vendor", "198.51.100.7:162"),
            "2026-10-29 23:01:27".into(),
        );
        list
    }

    #[test]
    fn cells_use_the_mibs() {
        let list = list();
        assert_eq!(list.len(), 3);
        assert_eq!(list.title(), "Traps (3)");
        assert_eq!(list.cell(0, TrapColumn::Time), "2026-10-29 23:01:25");
        assert_eq!(list.cell(0, TrapColumn::Source), "192.0.2.1:5000");
        assert_eq!(list.cell(0, TrapColumn::Type), "v2c trap");
        assert_eq!(
            list.cell(0, TrapColumn::Notification),
            "FEROSNMP-TRAP-TEST-MIB::fttPortChange"
        );
        assert_eq!(
            list.cell(0, TrapColumn::Objects),
            "fttPortName = \"eth0\", fttPortState = down(2)"
        );
        assert_eq!(list.cell(1, TrapColumn::Type), "v1 trap");
        assert_eq!(list.cell(1, TrapColumn::Notification), "IF-MIB::linkDown");
        assert_eq!(list.cell(7, TrapColumn::Time), "");
        let details = list.details(0).unwrap();
        assert!(
            details.starts_with("Received 2026-10-29 23:01:25"),
            "{details}"
        );
        assert!(
            details.contains("fttPortState = INTEGER: down(2)"),
            "{details}"
        );
    }

    #[test]
    fn filters_choose_the_rows() {
        let mut list = list();
        let filter = |source: &str, notification: &str, severity: &str, text: &str| TrapFilter {
            source: source.into(),
            notification: notification.into(),
            severity: severity.into(),
            text: text.into(),
        };
        list.set_filter(filter("192.0.2.", "", "", ""));
        assert_eq!(list.len(), 2);
        assert_eq!(list.title(), "Traps (2 of 3)");
        // An SNMPv1 trap is also found by the agent address inside it.
        list.set_filter(filter("192.0.2.10", "", "", ""));
        assert_eq!(list.len(), 1);
        assert_eq!(list.cell(0, TrapColumn::Type), "v1 trap");
        list.set_filter(filter("", "link*", "", ""));
        assert_eq!(list.len(), 1);
        list.set_filter(filter("", "1.3.6.1.4.1.9", "", ""));
        assert_eq!(list.len(), 1);
        assert_eq!(list.cell(0, TrapColumn::Source), "198.51.100.7:162");
        list.set_filter(filter("", "", "", "ETH0"));
        assert_eq!(list.len(), 1);
        list.set_filter(filter("", "", "major", ""));
        assert_eq!(list.len(), 0);
        list.set_filter(TrapFilter::default());
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn severity_comes_from_a_severity_object() {
        let described = list().described(0).unwrap().clone();
        assert_eq!(severity(&described), "");
        let mut alarm = described;
        alarm.objects[1].name = "VENDOR-MIB::alarmSeverity.0".into();
        alarm.objects[1].value = "critical(1)".into();
        assert_eq!(severity(&alarm), "critical(1)");
    }

    #[test]
    fn the_oldest_are_dropped_at_the_limit() {
        let mut list = TrapList::default();
        for i in 0..LIMIT {
            list.push(received("v2c-inform", "192.0.2.1:5000"), format!("{i}"));
        }
        assert_eq!(list.len(), LIMIT);
        let change = list.push(received("v2c-inform", "192.0.2.1:5000"), "new".into());
        assert_eq!(
            change,
            Change {
                added: 1,
                dropped: 1
            }
        );
        assert_eq!(list.len(), LIMIT);
        assert_eq!(list.total(), LIMIT);
        assert_eq!(list.cell(0, TrapColumn::Time), "1");
        assert_eq!(list.cell(LIMIT - 1, TrapColumn::Time), "new");
        // A filtered-out notification dropped from the top changes no row.
        list.set_filter(TrapFilter {
            text: "new".into(),
            ..TrapFilter::default()
        });
        assert_eq!(list.len(), 1);
        let change = list.push(received("v2c-inform", "192.0.2.1:5000"), "x".into());
        assert_eq!(change, Change::default());
        assert_eq!(list.cell(0, TrapColumn::Time), "new");
    }

    #[test]
    fn copies_finds_and_exports() {
        let list = list();
        let copied = list.tab_separated([1]);
        assert!(
            copied.starts_with("2026-10-29 23:01:26\t192.0.2.2:5001\tv1 trap\tIF-MIB::linkDown")
        );
        assert_eq!(list.find("link-3-updown", None, false), Some(2));
        assert_eq!(list.find("23:01", Some(2), false), Some(0));
        assert_eq!(list.find("nothing like it", None, false), None);

        let csv = list.csv();
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(
            lines[0],
            "time,source,version,type,notification,oid,severity,community,user,objects"
        );
        assert!(
            lines[1].starts_with(
                "2026-10-29T22:01:25.000Z,192.0.2.1:5000,2c,trap,\
                 FEROSNMP-TRAP-TEST-MIB::fttPortChange,1.3.6.1.4.1.8072.9999.2.0.1,,public,,"
            ),
            "{}",
            lines[1]
        );
        let json: serde_json::Value = serde_json::from_str(&list.json()).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 3);
        assert_eq!(json[1]["notification"], "IF-MIB::linkDown");
        assert_eq!(TrapList::default().json(), "[]\n");
    }
}
