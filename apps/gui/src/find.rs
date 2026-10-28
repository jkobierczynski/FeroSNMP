//! The search box: find nodes by name, wildcard pattern or OID.

use mib_core::{MibSet, NodeId, Pattern};

/// The result of one search, and which match is shown.
#[derive(Debug, Clone, Default)]
pub struct Matches {
    query: String,
    nodes: Vec<NodeId>,
    current: usize,
}

impl Matches {
    /// Search `set` for `query`.
    ///
    /// The query can be a pattern with `*` and `?`, plain text (matching every
    /// name that contains it), a name (`ifDescr`, `IF-MIB::ifDescr`) or an OID
    /// (`1.3.6.1.2.1.2.2.1.2`, also an instance such as `ifDescr.7`).
    ///
    /// Matches are in OID order, except that the node the query names exactly
    /// comes first: searching `pwIndex` shows `pwIndex` before `pwIndexNext`,
    /// even though `pwIndexNext` comes first in the tree.
    pub fn find(set: &MibSet, query: &str) -> Matches {
        let query = query.trim();
        let mut nodes: Vec<NodeId> = if query.is_empty() {
            Vec::new()
        } else {
            set.search(&Pattern::new(query))
                .into_iter()
                .map(|node| node.id())
                .collect()
        };
        if !query.contains(['*', '?'])
            && let Some(exact) = set.resolve(query)
        {
            nodes.retain(|&id| id != exact.id());
            nodes.insert(0, exact.id());
        }
        Matches {
            query: query.to_string(),
            nodes,
            current: 0,
        }
    }

    /// The query, trimmed, as searched.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Number of matches.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True if nothing matched.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The match being shown, if there are any.
    pub fn current(&self) -> Option<NodeId> {
        self.nodes.get(self.current).copied()
    }

    /// Position of the current match, counting from 1, for "match 2 of 9".
    pub fn position(&self) -> usize {
        if self.nodes.is_empty() {
            0
        } else {
            self.current + 1
        }
    }

    /// Go to the next match, wrapping around after the last one.
    pub fn next(&mut self) -> Option<NodeId> {
        if !self.nodes.is_empty() {
            self.current = (self.current + 1) % self.nodes.len();
        }
        self.current()
    }

    /// Go to the previous match, wrapping around before the first one.
    pub fn previous(&mut self) -> Option<NodeId> {
        if !self.nodes.is_empty() {
            self.current = (self.current + self.nodes.len() - 1) % self.nodes.len();
        }
        self.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ietf_set;

    fn names(set: &MibSet, matches: &Matches) -> Vec<String> {
        matches
            .nodes
            .iter()
            .map(|&id| set.node(id).name().to_string())
            .collect()
    }

    #[test]
    fn exact_name_comes_first() {
        let set = ietf_set();
        let matches = Matches::find(&set, "pwIndex");
        let found = names(&set, &matches);
        assert_eq!(found[0], "pwIndex");
        assert_eq!(found[1], "pwIndexNext", "the others follow in OID order");
    }

    #[test]
    fn wildcards_match_whole_names() {
        let set = ietf_set();
        let matches = Matches::find(&set, "entPhysical*Class");
        assert_eq!(names(&set, &matches), ["entPhysicalClass"]);
        let matches = Matches::find(&set, "pw*Index");
        let found = names(&set, &matches);
        assert!(
            found.contains(&"pwAttachedPwIndex".to_string()),
            "{found:?}"
        );
        assert!(!found.contains(&"pwIndexNext".to_string()), "{found:?}");
    }

    #[test]
    fn numeric_oids_and_instances_are_found() {
        let set = ietf_set();
        let matches = Matches::find(&set, "1.3.6.1.2.1.47.1.1.1.1.5");
        assert_eq!(names(&set, &matches), ["entPhysicalClass"]);
        let matches = Matches::find(&set, "entPhysicalClass.3");
        assert_eq!(names(&set, &matches), ["entPhysicalClass"]);
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let set = ietf_set();
        let mut matches = Matches::find(&set, "bfdSessIndex*");
        assert!(matches.len() >= 2);
        assert_eq!(matches.position(), 1);
        matches.previous();
        assert_eq!(matches.position(), matches.len());
        matches.next();
        assert_eq!(matches.position(), 1);
    }

    #[test]
    fn nothing_found() {
        let set = ietf_set();
        let mut matches = Matches::find(&set, "noSuchNameAnywhere");
        assert!(matches.is_empty());
        assert_eq!(matches.position(), 0);
        assert_eq!(matches.next(), None);
        assert!(Matches::find(&set, "   ").is_empty());
    }
}
