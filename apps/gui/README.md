# ferosnmp-gui

The FeroSNMP window: browse the loaded MIBs as a tree, search them, and see
everything about a node. Runs on Windows 10 and Windows 11.

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

- **File > Settings** sets the MIB folders (searched top to bottom, like
  `-M` in the CLI), whether the net-snmp and libsmi folders on this computer
  are used too, and the strictness. Settings are saved in
  `%APPDATA%\FeroSNMP\settings.toml`. Without any folders only the built-in
  modules (SNMPv2-SMI and friends) are loaded.
- MIBs load in the background; the window stays usable and the status bar
  says when loading is done. **F5** loads them again, for example after you
  changed a MIB file.
- **Ctrl+F** jumps to the search box. Type a name (`ifDescr`), a qualified
  name (`IF-MIB::ifDescr`), an OID (`1.3.6.1.2.1.2.2.1.2`, also an instance
  such as `ifDescr.7`) or a pattern with `*` and `?` (`if*Octets`), then press
  **Enter**. **Enter** again or **F3** goes to the next match, **Shift+F3** to
  the previous one, **Esc** back to the tree.
- Drag the bars between the panes to resize them.

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
| `src/ui/` | The Win32 windows: main window, toolbar, tree, details, icons, Diagnostics tab, source and "why" windows, Settings dialog. |

Everything outside `src/ui/` is plain Rust and is tested on every platform
(`cargo test -p ferosnmp-gui`). The Win32 helpers that are not specific to
SNMP (window classes, DPI, fonts, splitters, in-memory dialogs) are in the
`fero-win32` crate.

Win32 calls are `unsafe` in Rust because the compiler cannot check what
Windows does with the pointers it is given; each `unsafe` block says why its
use is fine. The one rule that is easy to miss: Win32 often calls back into
the window *while* a call is still running (expanding a tree item sends a
notification before `TVM_EXPAND` returns), so window state lives in `Cell`
and `RefCell` and a `RefCell` borrow is never held across such a call. See
the `fero_win32::window` documentation.
