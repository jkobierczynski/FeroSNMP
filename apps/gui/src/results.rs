//! What the Results tab shows: the variable bindings of the last query, as
//! text for a virtual list view, the clipboard and CSV files.
//!
//! Values are kept as they arrived and turned into text only when a row is
//! painted, so a walk of 100,000 rows costs memory for the values only, the
//! raw view switches instantly, and reloading the MIBs updates the names.

use std::fmt::Write;

use mib_core::MibSet;
use snmp_core::{Formatter, ValueText, Varbind};

/// A column of the Results list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    /// `IF-MIB::ifDescr.1`, or the numeric OID in the raw view.
    Name,
    /// `"eth0"`, or the bytes in hex in the raw view.
    Value,
    /// `STRING`, or the ASN.1 tag and type in the raw view.
    Type,
}

impl Column {
    /// Every column, in display order.
    pub const ALL: [Column; 3] = [Column::Name, Column::Value, Column::Type];

    /// The column heading.
    pub fn title(self, raw: bool) -> &'static str {
        match (self, raw) {
            (Column::Name, false) => "Name",
            (Column::Name, true) => "OID",
            (Column::Value, false) => "Value",
            (Column::Value, true) => "Bytes",
            (Column::Type, false) => "Type",
            (Column::Type, true) => "ASN.1 tag",
        }
    }

    /// Width at 96 DPI.
    pub fn width(self) -> i32 {
        match self {
            Column::Name => 300,
            Column::Value => 420,
            Column::Type => 120,
        }
    }
}

/// The results of one query.
#[derive(Debug, Default)]
pub struct ResultList {
    rows: Vec<Varbind>,
    mibs: Option<MibSet>,
    raw: bool,
}

impl ResultList {
    /// Forget every row.
    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// Add rows at the end.
    pub fn extend(&mut self, varbinds: Vec<Varbind>) {
        self.rows.extend(varbinds);
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True if there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The variable binding in a row.
    pub fn get(&self, row: usize) -> Option<&Varbind> {
        self.rows.get(row)
    }

    /// The MIBs used for names and values (after a reload, the new set).
    pub fn set_mibs(&mut self, mibs: Option<MibSet>) {
        self.mibs = mibs;
    }

    /// Show numeric OIDs, tags and bytes instead of names and values.
    pub fn set_raw(&mut self, raw: bool) {
        self.raw = raw;
    }

    /// True in the raw view.
    pub fn raw(&self) -> bool {
        self.raw
    }

    fn formatter(&self) -> Formatter<'_> {
        Formatter::new(self.mibs.as_ref())
    }

    /// The text of one cell, as the list shows it.
    pub fn cell(&self, row: usize, column: Column) -> String {
        match self.rows.get(row) {
            Some(varbind) => cells(&mut self.formatter(), varbind, self.raw, column),
            None => String::new(),
        }
    }

    /// The rows as tab-separated lines, one per row, in the current view:
    /// what Ctrl+C copies, so it pastes into a spreadsheet as columns.
    pub fn tab_separated(&self, rows: impl IntoIterator<Item = usize>) -> String {
        let mut formatter = self.formatter();
        let mut text = String::new();
        for varbind in rows.into_iter().filter_map(|row| self.rows.get(row)) {
            let line: Vec<String> = Column::ALL
                .iter()
                .map(|&column| one_line(&cells(&mut formatter, varbind, self.raw, column)))
                .collect();
            text.push_str(&line.join("\t"));
            text.push_str("\r\n");
        }
        text
    }

    /// The numeric OIDs of some rows, one per line.
    pub fn oids(&self, rows: impl IntoIterator<Item = usize>) -> String {
        let mut text = String::new();
        for varbind in rows.into_iter().filter_map(|row| self.rows.get(row)) {
            let _ = write!(text, "{}\r\n", varbind.oid);
        }
        text
    }

    /// Every row as CSV (RFC 4180), with both views:
    /// `oid,name,type,value,tag,bytes`.
    pub fn csv(&self) -> String {
        let mut formatter = self.formatter();
        let mut text = String::from("oid,name,type,value,tag,bytes\r\n");
        for varbind in &self.rows {
            let ValueText {
                type_name,
                text: value,
            } = formatter.value_text(varbind);
            let fields = [
                varbind.oid.to_string(),
                formatter.oid(&varbind.oid),
                type_name,
                value,
                format!("0x{:02x}", varbind.value.tag()),
                hex(&varbind.value.content_octets()),
            ];
            let fields: Vec<String> = fields.iter().map(|field| csv_field(field)).collect();
            text.push_str(&fields.join(","));
            text.push_str("\r\n");
        }
        text
    }

    /// The first row after `from` (or before it, `backwards`) whose cells or
    /// numeric OID contain `query`, ignoring case; wraps around. Searching
    /// from `None` starts at the top (or the bottom).
    pub fn find(&self, query: &str, from: Option<usize>, backwards: bool) -> Option<usize> {
        let query = query.trim().to_lowercase();
        let count = self.rows.len();
        if query.is_empty() || count == 0 {
            return None;
        }
        let mut formatter = self.formatter();
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
                let varbind = &self.rows[row];
                varbind.oid.to_string().contains(&query)
                    || Column::ALL.iter().any(|&column| {
                        cells(&mut formatter, varbind, self.raw, column)
                            .to_lowercase()
                            .contains(&query)
                    })
            })
    }
}

fn cells(formatter: &mut Formatter<'_>, varbind: &Varbind, raw: bool, column: Column) -> String {
    match (column, raw) {
        (Column::Name, false) => formatter.oid(&varbind.oid),
        (Column::Name, true) => varbind.oid.to_string(),
        (Column::Value, false) => formatter.value_text(varbind).text,
        (Column::Value, true) => hex(&varbind.value.content_octets()),
        (Column::Type, false) => formatter.value_text(varbind).type_name,
        (Column::Type, true) => {
            format!("{:02x} {}", varbind.value.tag(), varbind.value.type_name())
        }
    }
}

/// Bytes as `4c 69 6e`, as `ferosnmp --raw` prints them.
fn hex(data: &[u8]) -> String {
    let mut text = String::with_capacity(data.len() * 3);
    for (i, byte) in data.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Line breaks (in sysDescr, for example) as spaces, for one cell.
pub fn one_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\r', '\n', '\t'], " ")
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// A count with thousands separators: `100,000`.
pub fn count_text(count: usize) -> String {
    let digits = count.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ietf_set;
    use snmp_core::Value;

    fn list() -> ResultList {
        let mut list = ResultList::default();
        list.set_mibs(Some(ietf_set()));
        list.extend(vec![
            Varbind {
                oid: "1.3.6.1.2.1.47.1.1.1.1.7.1".parse().unwrap(),
                value: Value::OctetString(b"lo, \"loopback\"".to_vec()),
            },
            Varbind {
                oid: "1.3.6.1.2.1.47.1.1.1.1.5.1".parse().unwrap(),
                value: Value::Integer(3),
            },
            Varbind {
                oid: "1.3.6.1.2.1.47.1.1.1.1.2.1".parse().unwrap(),
                value: Value::OctetString(b"two\nlines".to_vec()),
            },
        ]);
        list
    }

    #[test]
    fn cells_use_the_mibs() {
        let list = list();
        assert_eq!(list.cell(1, Column::Name), "ENTITY-MIB::entPhysicalClass.1");
        assert_eq!(list.cell(1, Column::Value), "chassis(3)");
        assert_eq!(list.cell(1, Column::Type), "INTEGER");
        assert_eq!(list.cell(9, Column::Name), "");
    }

    #[test]
    fn raw_view() {
        let mut list = list();
        list.set_raw(true);
        assert_eq!(list.cell(1, Column::Name), "1.3.6.1.2.1.47.1.1.1.1.5.1");
        assert_eq!(list.cell(1, Column::Value), "03");
        assert_eq!(list.cell(1, Column::Type), "02 INTEGER");
        assert_eq!(Column::Value.title(true), "Bytes");
    }

    #[test]
    fn copies_and_exports() {
        let list = list();
        assert_eq!(
            list.tab_separated([1, 2]),
            "ENTITY-MIB::entPhysicalClass.1\tchassis(3)\tINTEGER\r\n\
             ENTITY-MIB::entPhysicalDescr.1\t\"two lines\"\tSTRING\r\n"
        );
        assert_eq!(list.oids([0]), "1.3.6.1.2.1.47.1.1.1.1.7.1\r\n");
        let csv = list.csv();
        let lines: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(lines[0], "oid,name,type,value,tag,bytes");
        assert_eq!(
            lines[1],
            r#"1.3.6.1.2.1.47.1.1.1.1.7.1,ENTITY-MIB::entPhysicalName.1,STRING,"""lo, \""loopback\""""",0x04,6c 6f 2c 20 22 6c 6f 6f 70 62 61 63 6b 22"#
        );
        assert!(csv.contains("\"\"\"two\nlines\"\"\""), "{csv}");
    }

    #[test]
    fn find_wraps_around() {
        let list = list();
        assert_eq!(list.find("PHYSICALCLASS", None, false), Some(1));
        assert_eq!(list.find("1.3.6.1.2.1", Some(2), false), Some(0));
        assert_eq!(list.find("1.3.6.1.2.1", Some(0), true), Some(2));
        assert_eq!(list.find("nothing like it", None, false), None);
        assert_eq!(list.find(" ", None, false), None);
    }

    #[test]
    fn counts() {
        assert_eq!(count_text(0), "0");
        assert_eq!(count_text(999), "999");
        assert_eq!(count_text(1000), "1,000");
        assert_eq!(count_text(100_000), "100,000");
        assert_eq!(count_text(1_234_567), "1,234,567");
    }
}
