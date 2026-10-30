# ferosnmp-gui

The FeroSNMP window: browse the loaded MIBs as a tree, search them, see
everything about a node, query SNMP agents (GET, GETNEXT, GETBULK, WALK,
tables and SET over SNMPv1, SNMPv2c and SNMPv3), and receive traps and
INFORMs. Runs on Windows 10 and Windows 11.

## Building and running

The GUI is a Windows program, so build it with the Windows version of Rust,
from PowerShell or Git Bash (not from WSL, which builds Linux programs).

One-time setup:

1. Install the **Visual Studio Build Tools** with the "Desktop development
   with C++" workload (Rust uses its linker): <https://visualstudio.microsoft.com/visual-cpp-build-tools/>
2. Install Rust with `rustup-init.exe` from <https://rustup.rs> and accept the
   defaults.

Then, in the repository folder:

```powershell
cargo run --release -p ferosnmp-gui
```

The first build takes a few minutes. The program is then at
`target\release\ferosnmp-gui.exe`; you can start it from Explorer too.

If you also build the CLI from WSL in the same folder, give the Windows build
its own target folder so the two do not keep rebuilding each other's files:

```powershell
cargo run --release -p ferosnmp-gui --target-dir target\windows
```

## Using it

- **File > Settings** has three tabs. **MIBs**: the MIB folders (searched
  top to bottom, like `-M` in the CLI), whether the net-snmp and libsmi
  folders on this computer are used too, and the strictness. **Traps**: see
  [Traps](#traps) below. **Appearance**: light or dark tree and lists (see
  [Dark mode](#dark-mode)). Settings are saved in
  `%APPDATA%\FeroSNMP\settings.toml`. Without any folders only the built-in
  modules (SNMPv2-SMI and friends) are loaded.
- MIBs load in the background; the window stays usable and the status bar
  says when loading is done. **Ctrl+R** loads them again, for example after
  you changed a MIB file.
- **Ctrl+F** jumps to the search box. Type a name (`ifDescr`), a qualified
  name (`IF-MIB::ifDescr`), an OID (`1.3.6.1.2.1.2.2.1.2`, also an instance
  such as `ifDescr.7`) or a pattern with `*` and `?` (`if*Octets`), then press
  **Enter**. **Enter** again or **F3** goes to the next match, **Shift+F3** to
  the previous one, **Esc** back to the tree.
- Drag the bars between the panes to resize them. The window's position and
  size, the panes' sizes and the tab shown along the bottom are remembered
  when you close the window (in the `[layout]` part of `settings.toml`;
  delete it to start over). A window that was on a monitor that is no longer
  there opens on the main one.

### Querying an agent

The second row of the toolbar has an **Agent** box, an **Object** box and
the query buttons.

- **Agent**: pick a saved agent from the list, or type an address such as
  `192.0.2.1`, `router:1161` or `[2001:db8::1]:161`. A typed address uses
  SNMPv2c, community `public`, a timeout of 2 seconds and 2 retries.
  **File > Agents** (or the **Agents...** button) saves agents with their
  own version, port, community, timeout, retries and max-repetitions. They
  are kept in `settings.toml`; the community is stored as plain text there,
  like net-snmp's `snmp.conf`. An SNMPv3 agent has a user, security level,
  authentication and privacy protocol and a context name; its passwords go
  to the Windows Credential Manager, never to `settings.toml`. The Log tab
  shows the agent's engine ID, boots and uptime. See
  [docs/snmpv3.md](../../docs/snmpv3.md).
- **Object**: what to ask for. Clicking a node in the tree fills it in
  (`sysDescr.0` for a scalar, `IF-MIB::ifTable` for a table). You can type a
  name, `MODULE::name`, a name with an instance (`ifDescr.3`) or a numeric
  OID.
- **Get** reads the object (a scalar without `.0` gets it added), **Next**
  the instance after it (and puts that instance in the Object box, so
  pressing Next again steps on), **Bulk** one GETBULK request, **Walk**
  everything below it. **Enter** in either box runs the last kind of query
  again, and **F5** repeats the last query exactly.
- Results stream into the **Results** tab while you scroll; the tab title
  and the status bar show the count. **Cancel** or **Esc** stops a query.
  Starting a new query stops the one running.
- **Raw** (or **View > Raw values**, **Ctrl+Shift+R**) shows numeric OIDs,
  ASN.1 tags and the bytes of each value in hex.
- **Ctrl+C** copies the selected rows, tab-separated, so they paste into a
  spreadsheet as columns; **Ctrl+A** selects all of them. Right-click for
  *Copy Value* (or the column you clicked), *Copy OID*, *Show in MIB tree*
  (also **Enter** or a double click), *Export as CSV* (also **Ctrl+S**) and
  *Clear results*. Ctrl+C in the tree copies the node's numeric OID.
- **Ctrl+F** searches the results (or the table) when you used that list
  last (**F3** for the next match); click the tree to search the MIBs again.
- The **Log** tab lists each query, every request with its request-id,
  retries and time, and how the query ended.

### Tables

**Table** (or **Query > Table**, **Ctrl+T**) walks the table named in the
Object box (a table, its entry or any of its columns, such as `ifTable`,
`ifEntry` or `ifDescr`) and shows it in the **Table** tab as rows and
columns. Each row's instance is split into its INDEX values, read the way
the MIB defines them: integers, IP addresses, strings with their length,
fixed-size strings, OIDs, and the `IMPLIED` last index. A table that
`AUGMENTS` another uses the other table's INDEX. A row whose instance does
not fit the INDEX (an agent bug) is shown as `?.1.2.3`, with a note in the
Log tab.

- The columns are sized to the values once the walk is done.
- **Ctrl+C** copies the selected rows, tab-separated. Right-click for *Copy*
  the column you clicked, *Copy OID* of the cell, *Set* the cell, *Show in
  MIB tree*, *Export table as CSV* (also **Ctrl+S** while the Table tab is
  shown) and *Clear table*.
- The values also arrive in the Results tab, one per line, as for a walk.

This is the same table as `ferosnmp table` prints.

### Writing values

**Set...** (or **Query > Set value**, **F2**) opens the Set value dialog for
the instance in the Object box, the selected result, or the table cell you
right-clicked or double-clicked. The dialog fills in the current value when
it is known, and checks what you type against the MIB as you type:

- the type, the enumeration labels (offered in the Value list), ranges,
  sizes, `BITS` names and display hints such as MAC addresses
  (`00:1b:21:0a:ff:01`) and tenths (`-12.5`);
- whether the object is writable at all.

The line at the bottom says what will be sent, or why not. **Send as**
picks another type than the MIB's (for example hex for a string), and is
needed for an object no loaded MIB defines. To test how an agent handles a
value the MIB does not allow, tick *Send even if the MIB does not allow it*.

Set uses the Agent box like any query: the community needs write access
(net-snmp: `rwcommunity`, or `rwuser` for SNMPv3). When the agent refuses,
a message box says why, with a hint. The value the agent answered with
shows in the Results tab, and in the Table tab when the cell is in the
table shown. F5 and Enter never repeat a Set.

To try a walk of 100,000 rows, start the test agent from WSL or PowerShell:

```powershell
cargo run --release -p snmp-core --example big_agent -- 100000 127.0.0.1:11162
```

then type `127.0.0.1:11162` in the Agent box, `1.3.6.1.4.1.99999` in the
Object box (three columns, so 300,000 values) and press **Walk**.

### Traps

The **Traps** tab receives SNMPv1, SNMPv2c and SNMPv3 traps and INFORMs,
decoded with the loaded MIBs, as `ferosnmp traps listen` does (see
[docs/traps.md](../../docs/traps.md)).

- **Start listening** (the button above the list, **Traps > Start or stop
  listening** or **Ctrl+L**) opens the trap port; the line next to the
  filters says where it listens. Press it again to stop. To listen every
  time the window opens, tick *Start listening when FeroSNMP opens* in
  **Traps > Trap settings**.
- Each notification is one row: the local time it arrived, the sender, the
  version and whether it is a trap or an INFORM (INFORMs are acknowledged),
  the notification's name, its severity, and its objects on one line. The
  newest row stays in view unless you selected a row to read it.
- **Enter** or a double click shows the whole notification in a window of
  its own: community or SNMPv3 user and engine, uptime, the SNMPv1 fields,
  every object with its type, and a hint to load the vendor's MIB for
  anything no loaded MIB defines.
- The four boxes above the list filter it as you type; each box that is
  filled in must match, ignoring case:
  - **From**: part of the sender's address (`192.0.2.`). An SNMPv1 trap is
    also found by the agent address inside it.
  - **Notification**: part of the name (`linkDown`, `Port`), a pattern
    (`link*`, `IF-MIB::*`; with `*` the whole name must match) or an OID
    prefix (`1.3.6.1.4.1.9`), as `ferosnmp traps listen --notification`.
  - **Severity**: part of the severity text, such as `crit`. The severity
    is the value of the first object whose name contains `severity`, as in
    most vendor alarm MIBs (`ciscoEnvMonSeverity`, `alarmSeverity`);
    notifications without one, such as linkDown, have none.
  - **Any text**: anywhere in the notification, names and values included.
  The tab title says how many are shown: `Traps (12 of 340)`.
- The newest 10,000 notifications are kept; older ones are dropped as new
  ones arrive. Reloading the MIBs decodes the ones kept again, so loading
  a vendor's MIB afterwards names the traps already received.
- **Ctrl+C** copies the selected rows; **Ctrl+F** searches the list.
  Right-click for *Show details*, *Show in MIB tree* (the notification's
  definition), *Export as CSV or JSON* and *Clear traps* (also in the
  **Traps** menu). The export holds the rows the filters show; CSV has one
  line per notification with the time in UTC, and JSON is an array of the
  objects `ferosnmp traps listen --format json` writes.
- Datagrams that are not notifications, and SNMPv3 messages that fail a
  check (unknown user, wrong password), go to the **Log** tab with a hint.

**Traps > Trap settings** (the Traps tab of File > Settings):

- **Port** and **Address**: where to listen. Port 162 is the standard.
  Windows' own *SNMP Trap* service may hold it; FeroSNMP then says so and
  offers port 1162 instead. [docs/traps.md](../../docs/traps.md) shows how
  to stop the service and open the Windows Firewall.
- **SNMPv3 trap users**: SNMPv1 and v2c traps are accepted from anyone, but
  an SNMPv3 sender needs a user here, with its security level, protocols
  and passwords. Leave the engine ID empty to accept the user from any
  sender, or give the sender's engine ID to accept it from that sender only
  (net-snmp's `createUser -e`). **Import...** reads the `createUser` lines
  of a `snmptrapd.conf`. The passwords go to the Windows Credential Manager
  (as `FeroSNMP/trap-user/<name>`), never to `settings.toml`.
- Changing the port or the users while listening listens again at once.

### Dark mode

**File > Settings > Appearance** makes the MIB tree, the details and the
lists dark, as in Explorer, or follows the Windows setting (*Settings >
Personalization > Colors > Choose your app mode*), which is the default.
The title bar follows too. Menus, buttons, boxes and tabs stay light: the
classic Windows controls have no dark mode.

### Diagnostics

The **Diagnostics** tab at the bottom lists every problem found while
loading, errors first, with a fix hint where FeroSNMP has one.

- **Click** a row to show the MIB source with the problem highlighted;
  **double-click** or **Enter** also brings that window to the front. The
  source window follows along as you move through the list.
- **Ctrl+W** (or **Edit > Why this symbol?**, or the button in the source
  window) explains how the name in the selected diagnostic, or the selected
  tree node, is resolved: which modules define it, what the `IMPORTS` say,
  and why it fails. This is the same report as `ferosnmp mib why`.
- **Ctrl+E** (**File > Export diagnostics**) saves the list as text or JSON,
  in the same formats as `ferosnmp lint`.
- **Drop** MIB files or folders on the window to load them too, for this
  session only (they are not added to the settings).
- Edit a MIB in another program and switch back: FeroSNMP notices the file
  changed and loads the MIBs again, staying on the same diagnostic.

To try it with the test corpus, run `corpus/fetch.sh` once (from WSL or Git
Bash), then add the repository's `corpus` folder in Settings.

## How the code is organised

| File | What it does |
|------|--------------|
| `build.rs` | Embeds the application manifest (modern controls, DPI awareness, Windows 10/11). |
| `src/main.rs` | Starts the UI on Windows; a stub elsewhere. |
| `src/settings.rs` | The settings file. |
| `src/find.rs` | The search box logic. |
| `src/node_info.rs` | What the details pane shows for a node. |
| `src/loading.rs` | Loading MIBs on a background thread. |
| `src/diagnostics.rs` | The Diagnostics list's rows and columns, and the export formats. |
| `src/rich_text.rs` | MIB source as coloured RTF for the source window. |
| `src/file_stamps.rs` | Noticing loaded files that changed on disk. |
| `src/targets.rs` | Saved agents, and what the Agent box means. |
| `src/query.rs` | The SNMP engine's thread, and handing results to the window in batches. |
| `src/results.rs` | The Results tab's cells, copying and the CSV export. |
| `src/table.rs` | The Table tab: rows, cells, copying, CSV, and finding the table for the Object box. |
| `src/set_value.rs` | What the Set value dialog shows and checks. |
| `src/oplog.rs` | The Log tab. |
| `src/traps.rs` | The Traps tab: the 10,000 newest notifications, filters, cells, copying, CSV and JSON. |
| `src/trap_users.rs` | SNMPv3 trap users in the settings file. |
| `src/secrets.rs` | Which passwords of agents and trap users to keep in the Credential Manager. |
| `src/ui/` | The Win32 windows: main window, toolbar, tree, details, icons, the Results, Table, Traps, Log and Diagnostics tabs, source, "why" and trap windows, Settings, Trap user, Agents and Set value dialogs. |

Everything outside `src/ui/` is plain Rust and is tested on every platform
(`cargo test -p ferosnmp-gui`). The Win32 helpers that are not specific to
SNMP (window classes, DPI, fonts, splitters, in-memory and tabbed dialogs,
dark tree and list views) are in the `fero-win32` crate.

Win32 calls are `unsafe` in Rust because the compiler cannot check what
Windows does with the pointers it is given; each `unsafe` block says why its
use is fine. The one rule that is easy to miss: Win32 often calls back into
the window *while* a call is still running (expanding a tree item sends a
notification before `TVM_EXPAND` returns), so window state lives in `Cell`
and `RefCell` and a `RefCell` borrow is never held across such a call. See
the `fero_win32::window` documentation.

Queries never run on the UI thread. `src/query.rs` starts one thread with a
tokio runtime and the `snmp-core` engine; the window submits an operation
and gets a handle back at once. The engine's thread posts `WM_APP_QUERY`
only when the handle's inbox was empty, and the window takes everything
waiting in one go, at most every 50 ms. Posted messages come before
painting in a Win32 message queue, so taking results any faster would
leave the list no time to paint them during a fast walk.

The trap receiver runs on the same thread: `QueryWorker::listen` starts the
`trap-rx` listener there and hands back a feed with the same kind of inbox,
which posts `WM_APP_TRAPS`; during a trap storm the window takes traps at
most every 100 ms.
