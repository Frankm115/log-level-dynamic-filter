# log-level-dynamic-filter

Evaluates per-module log level overrides at runtime from a compiled filter directive tree. Standard library only, zero dependencies.

## Usage

```rust
use log_level_dynamic_filter::{FilterTree, Level};

// Parse a directive string once, at startup.
let tree = FilterTree::parse("warn, my_app::http=debug, my_app::db=off");

// Query at runtime — no re-parsing.
assert_eq!(tree.resolve("my_app::http::server"), Some(Level::Debug));
assert_eq!(tree.resolve("my_app::db"), Some(Level::Off));
assert_eq!(tree.resolve("my_app"), Some(Level::Warn));
assert_eq!(tree.resolve("unrelated"), Some(Level::Warn));

// Convenience predicate.
assert!(tree.enabled("my_app::http::server", Level::Debug));
assert!(!tree.enabled("my_app::db", Level::Error)); // off blocks everything
```

For programmatic construction with error detection, use `FilterTree::builder()`:

```rust
use log_level_dynamic_filter::{FilterTree, Level};

let mut b = FilterTree::builder();
b.add("", Level::Info).unwrap();           // root default
b.add("app::http", Level::Debug).unwrap(); // per-module override
let tree = b.finish();

assert_eq!(tree.resolve("app::http::server"), Some(Level::Debug));
```

## Why this exists

Logging frameworks let you set a global level and override it per module. Doing that lookup by re-parsing a directive string on every log call is wasteful. This library parses the directive once into a tree of `FilterNode`s, then answers `resolve(target_path)` in time proportional to the number of `::`-separated segments in the target.

The trade-off: the tree is immutable after construction. If you need to change a level at runtime, build a new tree and swap it in. This keeps lookups allocation-free and branch-predictable.

## Directive syntax

Comma-separated entries. Whitespace around entries and the `=` separator is ignored.

- `level` — the root level, the fallback when no module-specific directive matches.
- `path::to::mod=level` — a per-module override.

Levels (lowercase, case-sensitive): `off`, `error`, `warn`, `info`, `debug`, `trace`.

## Resolution rule

The most specific (deepest) matching directive wins. A directive at `a::b` overrides one at `a` for target `a::b::c`, but `a` still applies to `a::d`. Levels are never combined or merged across ancestors — one directive applies per target.

## Edge cases

- An empty tree (no root directive) resolves every target to `None`. `enabled()` returns `false` for everything in this state. This is deliberate: an unconfigured filter should not emit.
- Unknown levels and malformed entries are silently dropped by `parse()`. A typo won't disable logging, but it also won't be reported. Use `FilterTree::builder()` and `add()` if you need to detect invalid input — `add()` returns `Err(())` for paths with empty segments like `"a::"` or `"::b"`.
- Level names are case-sensitive. `INFO` is not recognized; only `info` is.
- `"=info"` (empty path with an equals sign) is dropped, not treated as a root directive. Use bare `info` for the root.

## Exported names

- `Level` — enum with variants `Off`, `Error`, `Warn`, `Info`, `Debug`, `Trace`. Methods: `from_str(&str) -> Option<Level>`, `enabled(self, filter: Level) -> bool`, `Display`.
- `FilterTree` — the compiled tree. Methods: `new()`, `builder()`, `parse(&str)`, `resolve(&str) -> Option<Level>`, `enabled(&str, Level) -> bool`, `root() -> &FilterNode`.
- `FilterTreeBuilder` — incremental builder. Methods: `add(&str, Level) -> Result<(), ()>`, `finish() -> FilterTree`.
- `FilterNode` — a node in the tree. Methods: `new()`, `set_level(Level)`, `level() -> Option<Level>`, `child(&str) -> Option<&FilterNode>`, `child_segments() -> Vec<&str>`, `resolve(&str) -> Option<Level>`.
