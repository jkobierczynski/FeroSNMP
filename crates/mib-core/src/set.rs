//! A loaded, immutable set of MIB modules and the OID tree they define.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mib_rs::mib as m;

use crate::convert;
use crate::details::{IndexPart, NodeDetails, NodeKind, Syntax};
use crate::diagnostic::Diagnostic;
use crate::hints::{self, HintContext};
use crate::library::Strictness;
use crate::oid::Oid;
use crate::pattern::Pattern;
use crate::source::SourceView;
use crate::trace::{self, SymbolTrace, TraceError};

/// Every module loaded from a [`MibLibrary`](crate::MibLibrary), merged into
/// one OID tree.
///
/// A `MibSet` never changes after loading. Cloning it is cheap (it is a
/// reference-counted pointer), and it can be shared between threads, so the
/// UI, the SNMP engine and the trap decoder can all read the same set without
/// locks. To pick up changed files, load a new set and swap it in.
#[derive(Clone)]
pub struct MibSet {
    inner: Arc<Inner>,
}

struct Inner {
    mib: mib_rs::Mib,
    diagnostics: Vec<Diagnostic>,
    strictness: Strictness,
}

impl MibSet {
    /// Wrap a loaded mib-rs set. `folders` are the folders that were
    /// searched, for the hints about modules that were not found.
    pub(crate) fn new(mib: mib_rs::Mib, strictness: Strictness, folders: &[PathBuf]) -> Self {
        let mut diagnostics = convert::diagnostics(&mib.diagnostic_report());
        diagnostics.extend(hints::extra_diagnostics(&mib, strictness));
        hints::add_hints(&HintContext { mib: &mib, folders }, &mut diagnostics);
        MibSet {
            inner: Arc::new(Inner {
                mib,
                diagnostics,
                strictness,
            }),
        }
    }

    fn mib(&self) -> &mib_rs::Mib {
        &self.inner.mib
    }

    /// The strictness the set was loaded with.
    pub fn strictness(&self) -> Strictness {
        self.inner.strictness
    }

    /// Problems found while loading, ordered by kind, then module and position.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.inner.diagnostics
    }

    /// Explain how a name resolves: `ifIndex` looks at every module that
    /// defines it, `IF-MIB::ifIndex` follows the rules for a reference
    /// written in IF-MIB (its own definitions, its IMPORTS, then the
    /// fallbacks the strictness allows). Names that start with an uppercase
    /// letter are looked up as types.
    ///
    /// ```
    /// use mib_core::{MibLibrary, TraceOutcome};
    ///
    /// let set = MibLibrary::new().with_system_folders(false).load()?;
    /// let trace = set.trace("SNMPv2-TC::DisplayString")?;
    /// assert!(matches!(trace.outcome, TraceOutcome::Resolved { .. }));
    /// println!("{trace}");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn trace(&self, query: &str) -> Result<SymbolTrace, TraceError> {
        trace::trace(self.mib(), self.strictness(), query)
    }

    /// The text of a loaded file with its tokens classified, for showing it
    /// with the place of a diagnostic highlighted. This is the text exactly
    /// as it was loaded, even if the file changed on disk since. Returns
    /// `None` for files that are not part of the set; use
    /// [`SourceView::read`] for those.
    pub fn source_view(&self, file: &Path) -> Option<SourceView> {
        self.mib().modules().find_map(|module| {
            let source = module.source()?;
            let mib_rs::SourceOrigin::File { path } = source.origin() else {
                return None;
            };
            (path == file).then(|| SourceView::from_bytes(path, source.bytes().to_vec()))
        })
    }

    /// Every loaded module, including the built-in foundation modules.
    pub fn modules(&self) -> Vec<ModuleInfo> {
        self.mib()
            .modules()
            .map(|module| ModuleInfo {
                name: module.name().to_string(),
                file: module
                    .source()
                    .map(|source| convert::source_file(source.origin(), source.label())),
                embedded: matches!(
                    module.source_origin(),
                    Some(mib_rs::SourceOrigin::Embedded { .. })
                ),
                smi_version: SmiVersion::from_mib(module.language()),
            })
            .collect()
    }

    /// Information about one loaded module, by name.
    pub fn module(&self, name: &str) -> Option<ModuleInfo> {
        self.modules()
            .into_iter()
            .find(|module| module.name == name)
    }

    /// Number of nodes in the OID tree.
    pub fn node_count(&self) -> usize {
        self.mib().node_count()
    }

    /// The root of the OID tree. It has an empty OID and no name; its
    /// children are `ccitt(0)`, `iso(1)` and `joint-iso-ccitt(2)`.
    pub fn root(&self) -> Node<'_> {
        Node::new(self.mib().root_node())
    }

    /// The node with this id. Ids come from [`Node::id`] on the same set.
    ///
    /// # Panics
    ///
    /// Panics if the id came from a different `MibSet`.
    pub fn node(&self, id: NodeId) -> Node<'_> {
        Node::new(self.mib().node_by_id(id.0))
    }

    /// Look up a node by name (`ifDescr`) or qualified name
    /// (`IF-MIB::ifDescr`).
    ///
    /// When several modules define the same name, objects are preferred over
    /// other definitions; use [`find_all`](Self::find_all) to see every match,
    /// or a qualified name to pick one.
    pub fn find(&self, name: &str) -> Option<Node<'_>> {
        let node = match name.split_once("::") {
            Some((module, name)) => self.mib().module(module)?.node(name)?,
            None => self.mib().node(name)?,
        };
        Some(Node::new(node))
    }

    /// Every node with this (unqualified) name, from all modules.
    pub fn find_all(&self, name: &str) -> Vec<Node<'_>> {
        self.mib()
            .nodes_by_name(name)
            .iter()
            .map(|&id| Node::new(self.mib().node_by_id(id)))
            .collect()
    }

    /// The node at exactly this OID, if one is defined.
    pub fn node_at(&self, oid: &Oid) -> Option<Node<'_>> {
        self.mib()
            .exact_node_by_oid(&convert::to_mib_oid(oid))
            .map(Node::new)
    }

    /// Split an instance OID, such as one in an SNMP response, into the
    /// deepest defined node and the arcs after it.
    ///
    /// For `1.3.6.1.2.1.2.2.1.2.7` this returns `ifDescr` with suffix `[7]`.
    /// If no defined node matches at all, the node is the [root](Self::root)
    /// and the suffix is the whole OID.
    pub fn lookup(&self, oid: &Oid) -> InstanceLookup<'_> {
        let lookup = self.mib().lookup_instance(&convert::to_mib_oid(oid));
        InstanceLookup {
            node: Node::new(lookup.node()),
            suffix: lookup.suffix().to_vec(),
        }
    }

    /// Look up whatever the user typed: a name (`sysDescr`), a qualified
    /// name (`SNMPv2-MIB::sysDescr`), a numeric OID (`1.3.6.1.2.1.1.1`, with
    /// or without a leading dot) or an instance (`ifDescr.7`).
    ///
    /// Instances resolve to the deepest defined node (`ifDescr`); use
    /// [`lookup`](Self::lookup) when the suffix matters. Returns `None` when
    /// nothing matches.
    pub fn resolve(&self, query: &str) -> Option<Node<'_>> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        let node = self.mib().resolve_node(query)?;
        // A numeric OID outside every loaded module falls back to the root;
        // that is "not found" for the caller.
        if node.oid().is_empty() {
            return None;
        }
        Some(Node::new(node))
    }

    /// Every named node whose name matches `pattern`, in OID order. A
    /// pattern written as `MODULE::name` is matched against qualified names.
    pub fn search(&self, pattern: &Pattern) -> Vec<Node<'_>> {
        let qualified = pattern.is_qualified();
        let mut matches = Vec::new();
        // Depth first, children pushed in reverse so they come off the stack
        // in arc order; that visits the nodes in OID order.
        let mut stack = vec![self.root()];
        while let Some(node) = stack.pop() {
            if !node.name().is_empty() {
                let matched = if qualified {
                    pattern.matches(&node.qualified_name())
                } else {
                    pattern.matches(node.name())
                };
                if matched {
                    matches.push(node);
                }
            }
            let first_child = stack.len();
            stack.extend(node.children());
            stack[first_child..].reverse();
        }
        matches
    }

    /// Format an OID for display as `MODULE::name.suffix`, such as
    /// `IF-MIB::ifDescr.7`. Falls back to the numeric form when no node
    /// matches.
    pub fn format_oid(&self, oid: &Oid) -> String {
        self.mib().format_oid(&convert::to_mib_oid(oid))
    }
}

impl fmt::Debug for MibSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MibSet")
            .field("modules", &self.mib().modules().count())
            .field("nodes", &self.node_count())
            .field("diagnostics", &self.diagnostics().len())
            .field("strictness", &self.strictness())
            .finish()
    }
}

/// Identifies a node within one [`MibSet`]; get the node back with
/// [`MibSet::node`].
///
/// Useful where a reference cannot be stored, such as the item data of a
/// tree view control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(mib_rs::raw::NodeId);

/// One node in the OID tree of a [`MibSet`].
///
/// A `Node` borrows the set it came from, so it is cheap to copy and pass
/// around but cannot outlive the set. Use [`Node::details`] for an owned
/// summary, or [`Node::id`] to remember a node and find it again later.
#[derive(Clone, Copy)]
pub struct Node<'a> {
    inner: m::Node<'a>,
}

impl<'a> Node<'a> {
    fn new(inner: m::Node<'a>) -> Self {
        Node { inner }
    }

    /// This node's id, to find it again with [`MibSet::node`].
    pub fn id(self) -> NodeId {
        NodeId(self.inner.id())
    }

    /// The name, such as `"ifDescr"`. Empty for unnamed nodes and the root.
    pub fn name(self) -> &'a str {
        self.inner.name()
    }

    /// `MODULE::name`, or just the name if the module is unknown.
    pub fn qualified_name(self) -> String {
        match self.module_name() {
            Some(module) => format!("{module}::{}", self.name()),
            None => self.name().to_string(),
        }
    }

    /// The numeric OID.
    pub fn oid(self) -> Oid {
        convert::oid(self.inner.oid())
    }

    /// The last arc of the OID (`2` for `ifDescr`, `1.3.6.1.2.1.2.2.1.2`).
    pub fn arc(self) -> u32 {
        self.inner.arc()
    }

    /// What kind of definition this is.
    pub fn kind(self) -> NodeKind {
        convert::node_kind(self.inner.kind())
    }

    /// Name of the module that defines this node.
    pub fn module_name(self) -> Option<&'a str> {
        self.inner.module().map(|module| module.name())
    }

    /// The node one level up, or `None` for the root.
    pub fn parent(self) -> Option<Node<'a>> {
        self.inner.parent().map(Node::new)
    }

    /// Direct children, in arc order.
    pub fn children(self) -> impl Iterator<Item = Node<'a>> + 'a {
        self.inner.children().map(Node::new)
    }

    /// True if the node has at least one child (to show an expand button).
    pub fn has_children(self) -> bool {
        self.inner.children().next().is_some()
    }

    /// Everything known about this node, as owned data.
    pub fn details(self) -> NodeDetails {
        let node = self.inner;
        let object = node.object();
        let row = object.and_then(|object| object.row());
        let definition = Definition::of(node);
        NodeDetails {
            name: node.name().to_string(),
            oid: self.oid(),
            module: self.module_name().map(str::to_string),
            kind: self.kind(),
            status: definition.status.map(convert::status),
            access: object.map(|object| convert::access(object.access())),
            syntax: object.and_then(syntax),
            units: object.and_then(|object| non_empty(object.units())),
            index: row
                .map(|row| {
                    row.effective_indexes()
                        .map(|index| IndexPart {
                            name: index.name().to_string(),
                            implied: index.implied(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            augments: object
                .filter(|object| object.is_row())
                .and_then(|row| row.augments())
                .map(|augmented| augmented.name().to_string()),
            description: non_empty(definition.description),
            reference: non_empty(definition.reference),
        }
    }
}

/// STATUS, DESCRIPTION and REFERENCE of whatever is defined at a node.
///
/// mib-rs keeps these on the definition (object, notification, group, ...),
/// not on the tree node, except for plain OBJECT-IDENTITY style nodes.
struct Definition<'a> {
    status: Option<mib_rs::Status>,
    description: &'a str,
    reference: &'a str,
}

impl<'a> Definition<'a> {
    fn of(node: m::Node<'a>) -> Self {
        macro_rules! from_entity {
            ($entity:expr) => {
                Definition {
                    status: Some($entity.status()),
                    description: $entity.description(),
                    reference: $entity.reference(),
                }
            };
        }
        if let Some(object) = node.object() {
            from_entity!(object)
        } else if let Some(notification) = node.notification() {
            from_entity!(notification)
        } else if let Some(group) = node.group() {
            from_entity!(group)
        } else if let Some(compliance) = node.compliance() {
            from_entity!(compliance)
        } else if let Some(capability) = node.capability() {
            from_entity!(capability)
        } else {
            Definition {
                status: node.status(),
                description: node.description(),
                reference: node.reference(),
            }
        }
    }
}

fn syntax(object: m::Object<'_>) -> Option<Syntax> {
    let ty = object.ty()?;
    let mut type_chain = Vec::new();
    let mut next = Some(ty);
    while let Some(ty) = next {
        if !ty.name().is_empty() {
            type_chain.push(ty.name().to_string());
        }
        next = ty.parent();
    }
    let base_type = convert::base_type(ty.effective_base());
    if !type_chain.iter().any(|name| name == base_type.as_str()) {
        type_chain.push(base_type.as_str().to_string());
    }
    Some(Syntax {
        type_chain,
        base_type,
        display_hint: non_empty(object.effective_display_hint()),
        enums: convert::named_numbers(object.effective_enums()),
        bits: convert::named_numbers(object.effective_bits()),
        ranges: convert::ranges(object.effective_ranges()),
        sizes: convert::ranges(object.effective_sizes()),
    })
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_string())
}

impl PartialEq for Node<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Node<'_> {}

impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Node({} {})", self.qualified_name(), self.oid())
    }
}

/// Result of [`MibSet::lookup`]: the deepest defined node and the instance
/// arcs after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceLookup<'a> {
    /// The deepest defined node whose OID is a prefix of the looked-up OID.
    pub node: Node<'a>,
    /// The remaining arcs, such as `[0]` for a scalar or the encoded INDEX
    /// values for a table cell.
    pub suffix: Vec<u32>,
}

/// SMI language version of a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SmiVersion {
    /// RFC 1155 / 1212 (SMIv1).
    V1,
    /// RFC 2578 (SMIv2).
    V2,
    /// RFC 3159 (SPPI, used by policy MIBs).
    Sppi,
    /// Not enough evidence to tell, often because of errors in the module.
    Unknown,
}

impl SmiVersion {
    fn from_mib(language: mib_rs::Language) -> Self {
        match language {
            mib_rs::Language::SMIv1 => SmiVersion::V1,
            mib_rs::Language::SMIv2 => SmiVersion::V2,
            mib_rs::Language::SPPI => SmiVersion::Sppi,
            mib_rs::Language::Unknown => SmiVersion::Unknown,
        }
    }
}

impl fmt::Display for SmiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SmiVersion::V1 => "SMIv1",
            SmiVersion::V2 => "SMIv2",
            SmiVersion::Sppi => "SPPI",
            SmiVersion::Unknown => "unknown",
        })
    }
}

/// A module in a loaded [`MibSet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInfo {
    /// Module name, from the `DEFINITIONS` line.
    pub name: String,
    /// The file it was loaded from, or `embedded:<NAME>` for a built-in module.
    pub file: Option<PathBuf>,
    /// True if the module was not found in any folder and the built-in copy
    /// was used (only foundation modules such as SNMPv2-SMI are built in).
    pub embedded: bool,
    /// SMI language version.
    pub smi_version: SmiVersion,
}
