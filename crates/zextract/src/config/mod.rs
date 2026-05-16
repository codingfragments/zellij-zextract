//! Configuration: KDL-subset parser + typed schema + file loading.
//!
//! Three layers, each in its own file:
//!
//!   - `parse.rs` — tokenizer + recursive-descent parser producing
//!     a generic AST (`Node` + `Value`). No knowledge of the zextract
//!     schema. Tests cover KDL surface we accept.
//!   - `schema.rs` (later commit) — typed `Config` struct, defaults,
//!     and `Config::from_ast(&[Node])` conversion. Domain validation
//!     lives here.
//!   - `load.rs` (later commit) — file I/O, path resolution, error
//!     formatting for the banner UI.
//!
//! The parser is a hand-rolled subset of KDL — chosen over the full
//! `kdl` crate because:
//!
//!   - Our schema is bounded; no need for the full KDL v2 spec
//!   - ~150 LOC parser instead of ~150 KB binary cost
//!   - Domain-aware error messages (line/col + "expected }" wording)
//!     for the parse-error banner in the picker
//!   - Independence from `kdl` crate spec/version churn
//!
//! Supported KDL features:
//!   - Nodes: `name arg1 arg2 { children }`
//!   - String values: `"foo"`, `"with \"escapes\""`
//!   - Integer values: `42`, `-7`
//!   - Boolean values: `true`, `false`
//!   - Identifiers as values: `enabled off` (bare word)
//!   - Block children: `name { node1; node2 }`
//!   - Node terminators: newline OR `;`
//!   - Line comments: `// to end of line`
//!
//! Deliberately unsupported (would error at parse time):
//!   - KDL properties (`key=value`)
//!   - Multi-line strings (`r#"..."#` raw strings)
//!   - Type annotations (`(u64)42`)
//!   - Slashdash comments (`/-`)
//!   - Block comments (`/* ... */`)
//!   - Floating-point numbers
//!
//! If the user hits any of those, the parser emits "unexpected
//! character at line:col" — the banner UI surfaces it directly.

pub mod parse;
