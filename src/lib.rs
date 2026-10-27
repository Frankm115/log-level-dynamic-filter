//! # log-level-dynamic-filter
//!
//! Evaluates per-module log level overrides at runtime from a compiled
//! filter directive tree.
//!
//! Zero third-party dependencies: standard library only.
//!
//! ## Interpretation of "compiled"
//!
//! A [`FilterTree`] is "compiled" in the sense that the raw directive
//! string is parsed once into a structured tree. The tree is then queried
//! many times at runtime without re-parsing. This trades a small upfront
//! cost for O(depth) lookups where *depth* is the number of path segments
//! in the target module path.
//!
//! ## Interpretation of "runtime overrides"
//!
//! Only one override applies to a given target: the most specific (deepest)
//! matching node. We do not combine or merge levels from multiple ancestors.
//! This mirrors the convention used by common logging frameworks where the
//! closest enclosing directive wins.

use std::collections::BTreeMap;

// ---------------------------------------------------------------------
// Level
// ---------------------------------------------------------------------

/// Log severity levels, ordered from most to least verbose.
///
/// `Off` disables all logging for a module that resolves to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Level {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl Level {
    /// Parse a level from its lowercase string form.
    ///
    /// Returns `None` for any unrecognized input rather than panicking,
    /// so that a malformed directive can be skipped by the caller.
    pub fn from_str(s: &str) -> Option<Level> {
        match s {
            "off" => Some(Level::Off),
            "error" => Some(Level::Error),
            "warn" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" => Some(Level::Debug),
            "trace" => Some(Level::Trace),
            _ => None,
        }
    }

    /// `true` when a message at `self` severity should be emitted given
    /// a filter that resolved to `filter`.
    ///
    /// `Level::Off` is excluded for any filter value.
    pub fn enabled(self, filter: Level) -> bool {
        filter != Level::Off && self <= filter
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Level::Off => "off",
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
            Level::Trace => "trace",
        };
        f.write_str(s)
    }
}

// ---------------------------------------------------------------------
// FilterNode
// ---------------------------------------------------------------------

/// A single node in the compiled filter tree.
///
/// A node optionally carries a level directive and a set of named child
/// nodes. Children are keyed on a single path segment (e.g. `"http"` in
/// `my_app::http::server`).
#[derive(Debug, Clone, Default)]
pub struct FilterNode {
    level: Option<Level>,
    children: BTreeMap<String, FilterNode>,
}

impl FilterNode {
    /// Create an empty node with no directive and no children.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach or replace the level directive on this node.
    pub fn set_level(&mut self, level: Level) {
        self.level = Some(level);
    }

    /// The level directive on this node, if any.
    pub fn level(&self) -> Option<Level> {
        self.level
    }

    /// Borrow a child node by segment name, if present.
    pub fn child(&self, segment: &str) -> Option<&FilterNode> {
        self.children.get(segment)
    }

    /// Return the segment names of this node's direct children, sorted.
    pub fn child_segments(&self) -> Vec<&str> {
        self.children.keys().map(|s| s.as_str()).collect()
    }

    /// Descend into `path` (split on `::`) and return the most specific
    /// matching ancestor's level, or `None` if no ancestor carries one.
    ///
    /// Most-specific-wins: a directive at `a::b` overrides one at `a` for
    /// the target `a::b::c`, but the `a` directive still applies to `a::d`.
    pub fn resolve(&self, path: &str) -> Option<Level> {
        let mut current: &FilterNode = self;
        let mut result = current.level;
        if path.is_empty() {
            return result;
        }
        for segment in path.split("::") {
            match current.children.get(segment) {
                Some(child) => {
                    current = child;
                    if let Some(l) = current.level {
                        result = Some(l);
                    }
                }
                None => break,
            }
        }
        result
    }
}

// ---------------------------------------------------------------------
// FilterTree
// ---------------------------------------------------------------------

/// A compiled filter directive tree.
///
/// Build it via [`FilterTree::builder`] and [`FilterTreeBuilder::add`],
/// or parse it from a directive string with [`FilterTree::parse`].
///
/// # Directive syntax
///
/// A directive is a comma-separated list of entries. Each entry is either
///
/// * `level`               — the root level, applied when nothing more
///                          specific matches; or
/// * `path::to::mod=level` — a per-module override.
///
/// Whitespace around entries and the `=` separator is ignored. Empty
/// entries (from a trailing comma) are skipped.
///
/// Unknown levels and malformed entries are silently dropped. This keeps
/// a single typo from disabling all logging, at the cost of hiding the
/// error. If you need to validate, build the tree manually with
/// [`FilterTreeBuilder`] and inspect [`FilterTreeBuilder::add`] results.
#[derive(Debug, Clone, Default)]
pub struct FilterTree {
    root: FilterNode,
}

impl FilterTree {
    /// Build a tree programmatically.
    pub fn builder() -> FilterTreeBuilder {
        FilterTreeBuilder {
            tree: FilterTree::new(),
        }
    }

    /// Create an empty tree with no root directive.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a directive string into a tree.
    ///
    /// See the type-level documentation for the syntax. Malformed entries
    /// and unknown levels are dropped.
    pub fn parse(input: &str) -> Self {
        let mut builder = Self::builder();
        for raw in input.split(',') {
            let entry = raw.trim();
            if entry.is_empty() {
                continue;
            }
            if let Some(eq_idx) = entry.find('=') {
                let path = entry[..eq_idx].trim();
                let level_str = entry[eq_idx + 1..].trim();
                if path.is_empty() {
                    continue;
                }
                if let Some(level) = Level::from_str(level_str) {
                    let _ = builder.add(path, level);
                }
            } else if let Some(level) = Level::from_str(entry) {
                let _ = builder.add("", level);
            }
        }
        builder.finish()
    }

    /// Resolve the effective level for `target_path`.
    ///
    /// Returns the most specific matching directive, or `None` if no node
    /// in the tree (including the root) carries a level for this path.
    pub fn resolve(&self, target_path: &str) -> Option<Level> {
        self.root.resolve(target_path)
    }

    /// Convenience: `true` when a message at `message` level should be
    /// emitted for `target_path`.
    ///
    /// If no directive matches, the message is not emitted. This is a
    /// deliberate default: an empty tree filters everything out.
    pub fn enabled(&self, target_path: &str, message: Level) -> bool {
        match self.resolve(target_path) {
            Some(filter) => message.enabled(filter),
            None => false,
        }
    }

    /// Borrow the root node, from which all directive lookups descend.
    pub fn root(&self) -> &FilterNode {
        &self.root
    }
}

// ---------------------------------------------------------------------
// FilterTreeBuilder
// ---------------------------------------------------------------------

/// Incremental builder for a [`FilterTree`].
///
/// Using a builder (rather than parsing a string) lets the caller detect
/// malformed paths and unknown levels, which the parser silently drops.
#[derive(Debug, Clone)]
pub struct FilterTreeBuilder {
    tree: FilterTree,
}

impl FilterTreeBuilder {
    /// Add or replace a directive.
    ///
    /// * `""` sets the root level.
    /// * Any other `path` is split on `::` and each segment is descended
    ///   or created. A bare `"::"` or a path containing empty segments
    ///   (e.g. `"a::::b"`) is rejected.
    ///
    /// Returns `Ok(())` on success or `Err(())` when the path is invalid.
    pub fn add(&mut self, path: &str, level: Level) -> Result<(), ()> {
        if path.is_empty() {
            self.tree.root.set_level(level);
            return Ok(());
        }
        let segments: Vec<&str> = path.split("::").collect();
        if segments.iter().any(|s| s.is_empty()) {
            // A path like "a::" or "::b" or "a::::b" contains an empty
            // segment, which has no meaningful module name and would
            // create a nameless child node. Reject it.
            return Err(());
        }
        let mut node = &mut self.tree.root;
        for seg in segments {
            node = node.children.entry(seg.to_owned()).or_default();
        }
        node.set_level(level);
        Ok(())
    }

    /// Consume the builder and return the compiled tree.
    pub fn finish(self) -> FilterTree {
        self.tree
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Level ----

    #[test]
    fn level_from_str_roundtrip() {
        for &l in &[
            Level::Off,
            Level::Error,
            Level::Warn,
            Level::Info,
            Level::Debug,
            Level::Trace,
        ] {
            let s = l.to_string();
            assert_eq!(Level::from_str(&s), Some(l));
        }
    }

    #[test]
    fn level_from_str_unknown() {
        assert_eq!(Level::from_str("verbose"), None);
        assert_eq!(Level::from_str(""), None);
        assert_eq!(Level::from_str("INFO"), None); // case-sensitive
    }

    #[test]
    fn level_enabled_off_excludes_all() {
        assert!(!Level::Error.enabled(Level::Off));
        assert!(!Level::Trace.enabled(Level::Off));
    }

    #[test]
    fn level_enabled_boundary() {
        assert!(Level::Info.enabled(Level::Info));
        assert!(Level::Error.enabled(Level::Info));
        assert!(!Level::Debug.enabled(Level::Info));
    }

    #[test]
    fn level_ordering() {
        assert!(Level::Error < Level::Warn);
        assert!(Level::Warn < Level::Info);
        assert!(Level::Info < Level::Debug);
        assert!(Level::Debug < Level::Trace);
        assert!(Level::Off < Level::Error);
    }

    // ---- Builder basic ----

    #[test]
    fn builder_root_only() {
        let mut b = FilterTree::builder();
        b.add("", Level::Info).unwrap();
        let t = b.finish();
        assert_eq!(t.resolve("anything::here"), Some(Level::Info));
    }

    #[test]
    fn builder_nested_override() {
        let mut b = FilterTree::builder();
        b.add("", Level::Warn).unwrap();
        b.add("my_app::http", Level::Debug).unwrap();
        let t = b.finish();

        assert_eq!(t.resolve("my_app"), Some(Level::Warn));
        assert_eq!(t.resolve("my_app::http"), Some(Level::Debug));
        assert_eq!(t.resolve("my_app::http::server"), Some(Level::Debug));
        assert_eq!(t.resolve("my_app::db"), Some(Level::Warn));
        assert_eq!(t.resolve("other"), Some(Level::Warn));
    }

    #[test]
    fn builder_deeper_override_wins() {
        let mut b = FilterTree::builder();
        b.add("", Level::Warn).unwrap();
        b.add("a::b", Level::Info).unwrap();
        b.add("a::b::c", Level::Trace).unwrap();
        let t = b.finish();

        assert_eq!(t.resolve("a"), Some(Level::Warn));
        assert_eq!(t.resolve("a::b"), Some(Level::Info));
        assert_eq!(t.resolve("a::b::c"), Some(Level::Trace));
        assert_eq!(t.resolve("a::b::c::d"), Some(Level::Trace));
        assert_eq!(t.resolve("a::b::d"), Some(Level::Info));
        assert_eq!(t.resolve("a::x"), Some(Level::Warn));
    }

    // ---- Builder edge cases ----

    #[test]
    fn builder_rejects_empty_segments() {
        let mut b = FilterTree::builder();
        assert!(b.add("", Level::Info).is_ok()); // root is allowed
        assert!(b.add("a::", Level::Info).is_err());
        assert!(b.add("::a", Level::Info).is_err());
        assert!(b.add("a::::b", Level::Info).is_err());
        assert!(b.add("::", Level::Info).is_err());
    }

    #[test]
    fn builder_replace_level() {
        let mut b = FilterTree::builder();
        b.add("app", Level::Info).unwrap();
        b.add("app", Level::Error).unwrap();
        let t = b.finish();
        assert_eq!(t.resolve("app"), Some(Level::Error));
    }

    #[test]
    fn builder_empty_tree_resolves_none() {
        let t = FilterTree::new();
        assert_eq!(t.resolve("anything"), None);
        assert!(!t.enabled("anything", Level::Error));
    }

    // ---- Parser ----

    #[test]
    fn parse_root_and_modules() {
        let t = FilterTree::parse("warn, my_app::http=debug, my_app::db=off");
        assert_eq!(t.resolve(""), Some(Level::Warn));
        assert_eq!(t.resolve("my_app"), Some(Level::Warn));
        assert_eq!(t.resolve("my_app::http"), Some(Level::Debug));
        assert_eq!(t.resolve("my_app::http::server"), Some(Level::Debug));
        assert_eq!(t.resolve("my_app::db"), Some(Level::Off));
        assert_eq!(t.resolve("my_app::db::inner"), Some(Level::Off));
        assert_eq!(t.resolve("other"), Some(Level::Warn));
    }

    #[test]
    fn parse_whitespace_tolerance() {
        let t = FilterTree::parse("  info  ,  app::mod  =  debug  ");
        assert_eq!(t.resolve(""), Some(Level::Info));
        assert_eq!(t.resolve("app::mod"), Some(Level::Debug));
    }

    #[test]
    fn parse_empty_input() {
        let t = FilterTree::parse("");
        assert_eq!(t.resolve(""), None);
        assert_eq!(t.resolve("app"), None);
    }

    #[test]
    fn parse_drops_unknown_levels() {
        let t = FilterTree::parse("info, app=verbose, other=fatal");
        assert_eq!(t.resolve(""), Some(Level::Info));
        assert_eq!(t.resolve("app"), Some(Level::Info)); // falls back to root
        assert_eq!(t.resolve("other"), Some(Level::Info));
    }

    #[test]
    fn parse_drops_empty_entries() {
        let t = FilterTree::parse("info,, ,app=debug,");
        assert_eq!(t.resolve(""), Some(Level::Info));
        assert_eq!(t.resolve("app"), Some(Level::Debug));
    }

    #[test]
    fn parse_drops_empty_path_with_eq() {
        // "=info" has an empty path; we skip it rather than treat it as root.
        let t = FilterTree::parse("=info, app=debug");
        assert_eq!(t.resolve(""), None);
        assert_eq!(t.resolve("app"), Some(Level::Debug));
    }

    #[test]
    fn parse_case_sensitive_levels() {
        // Uppercase level names are not recognized.
        let t = FilterTree::parse("INFO, app=DEBUG");
        assert_eq!(t.resolve(""), None);
        assert_eq!(t.resolve("app"), None);
    }

    #[test]
    fn parse_single_module_no_root() {
        let t = FilterTree::parse("app::http=trace");
        assert_eq!(t.resolve(""), None);
        assert_eq!(t.resolve("app"), None);
        assert_eq!(t.resolve("app::http"), Some(Level::Trace));
        assert_eq!(t.resolve("app::http::inner"), Some(Level::Trace));
    }

    // ---- enabled() ----

    #[test]
    fn enabled_uses_effective_level() {
        let t = FilterTree::parse("warn, app=debug");
        assert!(t.enabled("app", Level::Debug));
        assert!(t.enabled("app", Level::Trace) == false); // trace > debug
        assert!(t.enabled("app", Level::Error));
        assert!(!t.enabled("other", Level::Debug)); // falls to warn
        assert!(t.enabled("other", Level::Warn));
    }

    #[test]
    fn enabled_off_blocks_all() {
        let t = FilterTree::parse("app=off");
        assert!(!t.enabled("app", Level::Error));
        assert!(!t.enabled("app", Level::Trace));
    }

    #[test]
    fn enabled_no_match_blocks_all() {
        let t = FilterTree::parse("app=info");
        assert!(!t.enabled("other", Level::Error));
    }

    // ---- resolve empty path ----

    #[test]
    fn resolve_empty_path_returns_root() {
        let t = FilterTree::parse("info");
        assert_eq!(t.resolve(""), Some(Level::Info));
    }

    #[test]
    fn resolve_empty_path_no_root() {
        let t = FilterTree::parse("app=info");
        assert_eq!(t.resolve(""), None);
    }

    // ---- FilterNode inspection ----

    #[test]
    fn node_child_segments_sorted() {
        let mut b = FilterTree::builder();
        b.add("", Level::Info).unwrap();
        b.add("app::zebra", Level::Debug).unwrap();
        b.add("app::alpha", Level::Trace).unwrap();
        let t = b.finish();

        let root = t.root();
        assert_eq!(root.level(), Some(Level::Info));
        assert_eq!(root.child_segments(), vec!["app"]);

        let app = root.child("app").unwrap();
        assert_eq!(app.child_segments(), vec!["alpha", "zebra"]);
        assert_eq!(app.child("alpha").unwrap().level(), Some(Level::Trace));
    }

    // ---- Determinism: BTreeMap ordering ----

    #[test]
    fn resolve_is_deterministic_regardless_of_insertion_order() {
        // Two trees built with the same directives but different insertion
        // orders must produce identical results. BTreeMap guarantees this.
        let mut b1 = FilterTree::builder();
        b1.add("", Level::Warn).unwrap();
        b1.add("a::b", Level::Info).unwrap();
        b1.add("a::c", Level::Debug).unwrap();
        let t1 = b1.finish();

        let mut b2 = FilterTree::builder();
        b2.add("a::c", Level::Debug).unwrap();
        b2.add("a::b", Level::Info).unwrap();
        b2.add("", Level::Warn).unwrap();
        let t2 = b2.finish();

        for path in &["", "a", "a::b", "a::c", "a::b::d", "a::c::d", "x"] {
            assert_eq!(t1.resolve(path), t2.resolve(path));
        }
    }
}
