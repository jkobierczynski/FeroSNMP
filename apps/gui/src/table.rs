//! What the Table tab shows: the last table walked, as rows and columns,
//! with each row's instance split into its INDEX values.
//!
//! The rows are kept as [`snmp_core::TableView`] keeps them (values as they
//! arrived) and turned into text only when a row is painted.

use mib_core::{MibSet, NodeKind, Oid, Table};
use snmp_core::{Formatter, TableView, Value, Varbind};

use crate::results::one_line;

/// The table of the last Table query.
#[derive(Debug, Default)]
pub struct TableList {
    view: Option<TableView>,
    mibs: Option<MibSet>,
}

/// A cell of a value column, to set: its instance and current value.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub oid: Oid,
    pub value: Option<Value>,
}

impl TableList {
    /// Start showing `table`, with no rows yet.
    pub fn start(&mut self, table: Table) {
        self.view = Some(TableView::new(table));
    }

    /// Forget the table.
    pub fn clear(&mut self) {
        self.view = None;
    }

    /// The MIBs used for names and values (after a reload, the new set).
    pub fn set_mibs(&mut self, mibs: Option<MibSet>) {
        self.mibs = mibs;
    }

    /// The table shown, if any.
    pub fn table(&self) -> Option<&Table> {
        self.view.as_ref().map(TableView::table)
    }

    /// Add results; those that are not cells of the table are ignored.
    pub fn extend(&mut self, varbinds: impl IntoIterator<Item = Varbind>) {
        if let Some(view) = &mut self.view {
            view.extend(varbinds);
        }
    }

    /// Put a value in its cell, if it is a cell of a row already shown:
    /// after a SET, the new value. Returns true if a cell changed.
    pub fn update(&mut self, varbind: &Varbind) -> bool {
        let Some(view) = &mut self.view else {
            return false;
        };
        let known = view
            .table()
            .split(&varbind.oid)
            .is_some_and(|(_, suffix)| view.rows().iter().any(|row| row.suffix == suffix));
        known && view.add(varbind.clone())
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.view.as_ref().map_or(0, |view| view.rows().len())
    }

    /// True if there are no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Rows whose instance does not fit the INDEX.
    pub fn index_errors(&self) -> usize {
        self.view
            .as_ref()
            .map_or(0, |view| view.index_errors().count())
    }

    /// The column headings: the INDEX objects, then the value columns.
    pub fn headers(&self) -> Vec<String> {
        self.view
            .as_ref()
            .map(TableView::headers)
            .unwrap_or_default()
    }

    /// How many of the [`headers`](Self::headers) are INDEX objects.
    pub fn index_count(&self) -> usize {
        self.view.as_ref().map_or(0, TableView::index_count)
    }

    fn formatter(&self) -> Formatter<'_> {
        Formatter::new(self.mibs.as_ref())
    }

    /// The text of every cell of a row.
    pub fn row(&self, row: usize) -> Vec<String> {
        match &self.view {
            Some(view) if row < view.rows().len() => view.cells(row, &mut self.formatter()),
            _ => Vec::new(),
        }
    }

    /// The text of one cell, as the list shows it.
    pub fn cell(&self, row: usize, column: usize) -> String {
        self.row(row).into_iter().nth(column).unwrap_or_default()
    }

    /// The instance and value of a cell, for a value column (not an INDEX
    /// column): what Set value works on.
    pub fn cell_target(&self, row: usize, column: usize) -> Option<Cell> {
        let view = self.view.as_ref()?;
        let row = view.rows().get(row)?;
        let position = *view
            .value_columns()
            .get(column.checked_sub(view.index_count())?)?;
        Some(Cell {
            oid: view.cell_oid(position, row),
            value: row.values[position].clone(),
        })
    }

    /// The OID of the column object under a list column, to show in the
    /// MIB tree: an INDEX object or a value column.
    pub fn column_name(&self, column: usize) -> Option<String> {
        self.headers().into_iter().nth(column)
    }

    /// Some rows as tab-separated lines: what Ctrl+C copies.
    pub fn tab_separated(&self, rows: impl IntoIterator<Item = usize>) -> String {
        let mut text = String::new();
        for row in rows {
            let cells: Vec<String> = self.row(row).iter().map(|c| one_line(c)).collect();
            if cells.is_empty() {
                continue;
            }
            text.push_str(&cells.join("\t"));
            text.push_str("\r\n");
        }
        text
    }

    /// One column of some rows, one cell per line.
    pub fn column_text(&self, rows: impl IntoIterator<Item = usize>, column: usize) -> String {
        rows.into_iter()
            .map(|row| format!("{}\r\n", one_line(&self.cell(row, column))))
            .collect()
    }

    /// The whole table as CSV, with a header line.
    pub fn csv(&self) -> String {
        match &self.view {
            Some(view) => view.csv(&mut self.formatter()),
            None => String::new(),
        }
    }

    /// The tab title: `Table` or `ifTable (12 rows)`.
    pub fn title(&self) -> String {
        match self.table() {
            None => "Table".to_string(),
            Some(table) => match self.len() {
                1 => format!("{} (1 row)", table.name),
                n => format!("{} ({} rows)", table.name, crate::results::count_text(n)),
            },
        }
    }

    /// The first row after `from` (or before it, `backwards`) with a cell
    /// that contains `query`, ignoring case; wraps around.
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
                self.row(row)
                    .iter()
                    .any(|cell| cell.to_lowercase().contains(&query))
            })
    }
}

/// The table to walk for the Object box: a table, or any of its rows or
/// columns, such as `ifTable`, `ifEntry` or `ifDescr`.
pub fn find_table(mibs: &MibSet, text: &str) -> Result<Table, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Type the name of a table in the Object box, such as ifTable.".to_string());
    }
    let oid = snmp_core::resolve_object(mibs, text).map_err(|error| format!("{error}."))?;
    let node = mibs.node_at(&oid).ok_or_else(|| {
        format!("{text} is not a table. Give the name of a table, such as ifTable.")
    })?;
    match node.kind() {
        NodeKind::Table | NodeKind::Row | NodeKind::Column => mibs
            .table(node)
            .ok_or_else(|| format!("The MIB does not define the rows of {text}.")),
        kind => Err(format!(
            "{} is a {kind}, not a table. Give the name of a table, such as ifTable, \
             or select one in the MIB tree.",
            node.qualified_name()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_mib;

    fn varbind(oid: &str, value: Value) -> Varbind {
        Varbind {
            oid: oid.parse().unwrap(),
            value,
        }
    }

    /// fstPortTable: INDEX { fstPortSlot, fstPortAddress, fstPortName }.
    fn ports() -> TableList {
        let mibs = test_mib();
        let mut list = TableList::default();
        list.set_mibs(Some(mibs.clone()));
        list.start(find_table(&mibs, "fstPortTable").unwrap());
        let port = "1.3.6.1.4.1.8072.9999.1.4.1";
        list.extend([
            varbind(
                &format!("{port}.5.1.192.0.2.1.4.101.116.104.48"),
                Value::Gauge32(1000),
            ),
            varbind(
                &format!("{port}.5.2.198.51.100.7.4.109.103.109.116"),
                Value::Gauge32(100),
            ),
            varbind(
                &format!("{port}.6.1.192.0.2.1.4.101.116.104.48"),
                Value::Integer(1),
            ),
            // Not in this table.
            varbind("1.3.6.1.2.1.1.1.0", Value::Null),
        ]);
        list
    }

    #[test]
    fn rows_and_cells() {
        let list = ports();
        assert_eq!(list.len(), 2);
        assert_eq!(list.index_count(), 3);
        assert_eq!(
            list.headers(),
            [
                "fstPortSlot",
                "fstPortAddress",
                "fstPortName",
                "fstPortMac",
                "fstPortSpeed",
                "fstPortStatus"
            ]
        );
        assert_eq!(
            list.row(0),
            ["1", "192.0.2.1", "eth0", "", "1000 Mbit/s", "up(1)"]
        );
        assert_eq!(list.cell(1, 2), "mgmt");
        assert_eq!(list.cell(1, 5), "");
        assert_eq!(list.cell(7, 0), "");
        assert_eq!(list.title(), "fstPortTable (2 rows)");
        assert_eq!(
            list.tab_separated([1]),
            "2\t198.51.100.7\tmgmt\t\t100 Mbit/s\t\r\n"
        );
        assert_eq!(list.column_text([0, 1], 4), "1000 Mbit/s\r\n100 Mbit/s\r\n");
        assert!(list.csv().starts_with(
            "fstPortSlot,fstPortAddress,fstPortName,fstPortMac,fstPortSpeed,fstPortStatus\r\n"
        ));
        assert_eq!(list.find("MGMT", None, false), Some(1));
        assert_eq!(list.find("up(1)", Some(0), false), Some(0));
        assert_eq!(list.find("nothing", None, false), None);
    }

    #[test]
    fn cells_to_set() {
        let mut list = ports();
        // An INDEX column has no instance of its own.
        assert_eq!(list.cell_target(0, 1), None);
        let status = list.cell_target(0, 5).unwrap();
        assert_eq!(
            status.oid.to_string(),
            "1.3.6.1.4.1.8072.9999.1.4.1.6.1.192.0.2.1.4.101.116.104.48"
        );
        assert_eq!(status.value, Some(Value::Integer(1)));
        assert_eq!(list.cell_target(1, 5).unwrap().value, None);

        // A SET answer updates the cell of a row that is shown ...
        assert!(list.update(&Varbind {
            oid: status.oid.clone(),
            value: Value::Integer(2),
        }));
        assert_eq!(list.cell(0, 5), "down(2)");
        // ... but adds no rows.
        assert!(!list.update(&varbind(
            "1.3.6.1.4.1.8072.9999.1.4.1.6.3.192.0.2.1.0",
            Value::Integer(1)
        )));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn tables_are_found_from_any_part() {
        let mibs = test_mib();
        for text in ["fstPortTable", "fstPortEntry", "fstPortSpeed"] {
            assert_eq!(find_table(&mibs, text).unwrap().name, "fstPortTable");
        }
        let error = find_table(&mibs, "fstMode").unwrap_err();
        assert!(error.contains("not a table"), "{error}");
        assert!(find_table(&mibs, "").is_err());
        assert!(find_table(&mibs, "noSuchThing").is_err());
    }

    #[test]
    fn nothing_shown() {
        let list = TableList::default();
        assert_eq!(list.title(), "Table");
        assert!(list.is_empty());
        assert!(list.headers().is_empty());
        assert_eq!(list.csv(), "");
    }
}
