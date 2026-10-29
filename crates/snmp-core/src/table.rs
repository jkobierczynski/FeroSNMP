//! Table view: the results of a table walk as rows and columns, with the
//! instance suffix of each row split into its INDEX values.

use std::fmt::Write;

use mib_core::{BaseType, IndexColumn, IndexError, IndexValue, Table};

use crate::display_hint;
use crate::format::Formatter;
use crate::value::{Value, Varbind};

/// The rows of a table, collected from the variable bindings of a walk.
///
/// Rows are kept in OID order of their instance suffix, also when the
/// results arrive column by column (as a walk returns them). The columns
/// shown are the INDEX components first, then every column that holds
/// values; an index column that the agent also returns as a value (such as
/// `ifIndex`) is shown once, as an index.
///
/// ```
/// use mib_core::MibSet;
/// use snmp_core::{Formatter, TableView, Value, Varbind};
///
/// fn show(mibs: &MibSet) {
///     // SNMPv2-MIB::sysORTable, from any table, row or column node.
///     let node = mibs.find("sysORDescr").unwrap();
///     let mut view = TableView::new(mibs.table(node).unwrap());
///     let oid = "1.3.6.1.2.1.1.9.1.3.1".parse().unwrap();
///     view.add(Varbind { oid, value: Value::OctetString(b"SNMP".to_vec()) });
///     let mut formatter = Formatter::new(Some(mibs));
///     assert_eq!(view.headers(), ["sysORIndex", "sysORID", "sysORDescr", "sysORUpTime"]);
///     assert_eq!(view.cells(0, &mut formatter), ["1", "", "SNMP", ""]);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct TableView {
    table: Table,
    /// Positions in `table.columns` shown as value columns.
    shown: Vec<usize>,
    rows: Vec<TableRow>,
    others: usize,
}

/// One row of a [`TableView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRow {
    /// The instance suffix shared by every cell of the row.
    pub suffix: Vec<u32>,
    /// The INDEX values, or why the suffix does not fit the INDEX.
    pub index: Result<Vec<IndexValue>, IndexError>,
    /// One value per column of the [`Table`], `None` where none arrived.
    pub values: Vec<Option<Value>>,
}

impl TableView {
    /// An empty view of `table`.
    pub fn new(table: Table) -> TableView {
        let shown = table
            .columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.is_readable())
            .filter(|(_, column)| !table.index.iter().any(|index| index.name == column.name))
            .map(|(position, _)| position)
            .collect();
        TableView {
            table,
            shown,
            rows: Vec::new(),
            others: 0,
        }
    }

    /// The table described.
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// Add one result. Returns false (and counts it) when the OID is not a
    /// cell of this table.
    pub fn add(&mut self, varbind: Varbind) -> bool {
        let Some((column, suffix)) = self.table.split(&varbind.oid) else {
            self.others += 1;
            return false;
        };
        // Walks return rows in order within each column, so a new row
        // usually goes at the end; look there first.
        let at = match self.rows.last() {
            Some(last) if last.suffix.as_slice() < suffix => Err(self.rows.len()),
            _ => self
                .rows
                .binary_search_by(|row| row.suffix.as_slice().cmp(suffix)),
        };
        let at = match at {
            Ok(at) => at,
            Err(at) => {
                self.rows.insert(
                    at,
                    TableRow {
                        suffix: suffix.to_vec(),
                        index: self.table.decode_index(suffix),
                        values: vec![None; self.table.columns.len()],
                    },
                );
                at
            }
        };
        self.rows[at].values[column] = Some(varbind.value);
        true
    }

    /// Add every result of a batch; see [`add`](Self::add).
    pub fn extend(&mut self, varbinds: impl IntoIterator<Item = Varbind>) {
        for varbind in varbinds {
            self.add(varbind);
        }
    }

    /// The rows, in OID order.
    pub fn rows(&self) -> &[TableRow] {
        &self.rows
    }

    /// How many results were not cells of this table.
    pub fn others(&self) -> usize {
        self.others
    }

    /// The rows whose suffix does not fit the INDEX.
    pub fn index_errors(&self) -> impl Iterator<Item = (&TableRow, &IndexError)> {
        self.rows
            .iter()
            .filter_map(|row| row.index.as_ref().err().map(|error| (row, error)))
    }

    /// Column headers: the index names, then the value columns.
    pub fn headers(&self) -> Vec<String> {
        self.table
            .index
            .iter()
            .map(|index| index.name.clone())
            .chain(
                self.shown
                    .iter()
                    .map(|&c| self.table.columns[c].name.clone()),
            )
            .collect()
    }

    /// How many of the [`headers`](Self::headers) are index columns.
    pub fn index_count(&self) -> usize {
        self.table.index.len()
    }

    /// The value columns, as positions in [`Table::columns`], in the order
    /// [`headers`](Self::headers) lists them after the index columns.
    pub fn value_columns(&self) -> &[usize] {
        &self.shown
    }

    /// The text of every cell of row `row`, in the order of
    /// [`headers`](Self::headers). When the suffix does not fit the INDEX,
    /// the first index cell holds the raw suffix and the others are empty.
    ///
    /// # Panics
    ///
    /// Panics when `row` is out of range.
    pub fn cells(&self, row: usize, formatter: &mut Formatter<'_>) -> Vec<String> {
        let row = &self.rows[row];
        let mut cells = Vec::with_capacity(self.table.index.len() + self.shown.len());
        match &row.index {
            Ok(values) => {
                for (column, value) in self.table.index.iter().zip(values) {
                    cells.push(index_text(column, value, formatter));
                }
            }
            Err(_) => {
                cells.push(format!("?{}", suffix_text(&row.suffix)));
                cells.resize(self.table.index.len(), String::new());
            }
        }
        for &column in &self.shown {
            cells.push(match &row.values[column] {
                Some(value) => {
                    let varbind = Varbind {
                        oid: self.cell_oid(column, row),
                        value: value.clone(),
                    };
                    let text = formatter.value_text(&varbind);
                    // Cells are text already; quotes would only be noise.
                    match text.type_name.as_str() {
                        "STRING" => unquote(&text.text),
                        _ => text.text,
                    }
                }
                None => String::new(),
            });
        }
        cells
    }

    /// The OID of the cell in column `column` (a position in
    /// [`Table::columns`]) of `row`: the instance to GET or SET.
    pub fn cell_oid(&self, column: usize, row: &TableRow) -> mib_core::Oid {
        let mut arcs = self.table.columns[column].oid.arcs().to_vec();
        arcs.extend_from_slice(&row.suffix);
        mib_core::Oid::new(arcs)
    }

    /// The whole table as aligned text, one line per row, with a header
    /// line, and a note per row whose suffix does not fit the INDEX.
    pub fn text(&self, formatter: &mut Formatter<'_>) -> String {
        let headers = self.headers();
        let rows: Vec<Vec<String>> = (0..self.rows.len())
            .map(|row| self.cells(row, formatter))
            .collect();
        let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
        for cells in &rows {
            for (width, cell) in widths.iter_mut().zip(cells) {
                *width = (*width).max(cell.chars().count());
            }
        }
        let mut out = String::new();
        let mut line = |cells: &[String]| {
            let mut text = String::new();
            for (i, (cell, width)) in cells.iter().zip(&widths).enumerate() {
                if i > 0 {
                    text.push_str("  ");
                }
                let _ = write!(text, "{cell:<width$}");
            }
            out.push_str(text.trim_end());
            out.push('\n');
        };
        line(&headers);
        for cells in &rows {
            line(cells);
        }
        for (row, error) in self.index_errors() {
            let _ = writeln!(out, "note: row ?{}: {error}", suffix_text(&row.suffix));
        }
        out
    }

    /// The whole table as CSV (RFC 4180), with a header line.
    pub fn csv(&self, formatter: &mut Formatter<'_>) -> String {
        let field = |text: &str| {
            if text.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", text.replace('"', "\"\""))
            } else {
                text.to_string()
            }
        };
        let mut out = String::new();
        let mut line = |cells: &[String]| {
            let fields: Vec<String> = cells.iter().map(|c| field(c)).collect();
            out.push_str(&fields.join(","));
            out.push_str("\r\n");
        };
        line(&self.headers());
        for row in 0..self.rows.len() {
            line(&self.cells(row, formatter));
        }
        out
    }
}

/// `"say \"hi\""` as `say "hi"`.
fn unquote(text: &str) -> String {
    let Some(inner) = text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) else {
        return text.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    out
}

fn suffix_text(suffix: &[u32]) -> String {
    suffix.iter().map(|arc| format!(".{arc}")).collect()
}

/// An index value as text, with the index object's enumeration labels and
/// display hint.
pub fn index_text(column: &IndexColumn, value: &IndexValue, formatter: &Formatter<'_>) -> String {
    let syntax = column.syntax.as_ref();
    let hint = syntax.and_then(|s| s.display_hint.as_deref());
    match value {
        IndexValue::Integer(n) => {
            let n = i64::from(*n);
            if let Some(label) = syntax.and_then(|s| s.enums.iter().find(|e| e.value == n)) {
                return format!("{}({n})", label.label);
            }
            hint.and_then(|hint| display_hint::format_integer(hint, n))
                .unwrap_or_else(|| n.to_string())
        }
        IndexValue::Octets(bytes) if bytes.is_empty() => "\"\"".to_string(),
        IndexValue::Octets(bytes) => {
            if syntax.map(|s| s.base_type) != Some(BaseType::Bits)
                && let Some(text) = hint.and_then(|hint| display_hint::format_octets(hint, bytes))
            {
                return text;
            }
            value.to_string()
        }
        IndexValue::ObjectIdentifier(oid) => formatter.oid(oid),
        IndexValue::IpAddress(_) | IndexValue::Arcs(_) => value.to_string(),
    }
}
