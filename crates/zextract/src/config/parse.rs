//! Recursive-descent parser for our KDL subset. See `config/mod.rs`
//! for the supported surface. Produces a generic AST (`Vec<Node>`);
//! schema-level conversion to a typed `Config` lives in the
//! upcoming `config/schema.rs`.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    String(String),
    Integer(i64),
    Boolean(bool),
    /// Bare identifier used as a value, e.g. `default_profile quick`
    /// (no quotes). Treated as a string by callers that just need a
    /// name. Distinguished here in case future schema rules want to
    /// require quoted strings in specific contexts.
    Ident(String),
}

impl Value {
    /// Convenience: extract a string-like representation. Strings and
    /// idents return their inner text; numbers and booleans format.
    pub fn as_str_lossy(&self) -> String {
        match self {
            Value::String(s) | Value::Ident(s) => s.clone(),
            Value::Integer(n) => n.to_string(),
            Value::Boolean(b) => b.to_string(),
        }
    }

    pub fn as_string(&self) -> Option<&str> {
        match self {
            Value::String(s) | Value::Ident(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        if let Value::Integer(n) = self {
            Some(*n)
        } else {
            None
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Boolean(b) = self {
            Some(*b)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub args: Vec<Value>,
    pub children: Vec<Node>,
    /// 1-based line of the node's name token. Surfaced by the parse-
    /// error banner and used for diagnostic logging.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse(input: &str) -> Result<Vec<Node>, ParseError> {
    Parser::new(input).parse_document()
}

// ----------------------------------------------------------------------

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            src: input.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn parse_document(&mut self) -> Result<Vec<Node>, ParseError> {
        let nodes = self.parse_nodes(/* in_block */ false)?;
        self.skip_trivia();
        if self.pos < self.src.len() {
            return Err(self.error("unexpected trailing input"));
        }
        Ok(nodes)
    }

    fn parse_nodes(&mut self, in_block: bool) -> Result<Vec<Node>, ParseError> {
        let mut out = Vec::new();
        loop {
            self.skip_trivia();
            if self.pos >= self.src.len() {
                return Ok(out);
            }
            if self.peek() == Some(b'}') {
                if in_block {
                    self.advance();
                    return Ok(out);
                }
                return Err(self.error("unexpected '}' at document level"));
            }
            let node = self.parse_node()?;
            out.push(node);
        }
    }

    fn parse_node(&mut self) -> Result<Node, ParseError> {
        let line = self.line;
        let name = self.parse_node_name()?;
        let mut args = Vec::new();
        let mut children = Vec::new();
        loop {
            self.skip_line_trivia();
            match self.peek() {
                Some(b'{') => {
                    self.advance(); // consume '{'
                    children = self.parse_nodes(/* in_block */ true)?;
                    break;
                }
                Some(b';') => {
                    self.advance();
                    break;
                }
                // `}` ends the current node WITHOUT consuming — the
                // enclosing parse_nodes loop reads it as its own
                // block-terminator.
                Some(b'}') => break,
                Some(b'\n') | None => {
                    if self.peek().is_some() {
                        self.advance(); // consume newline
                    }
                    break;
                }
                Some(_) => {
                    let v = self.parse_value()?;
                    args.push(v);
                }
            }
        }
        Ok(Node {
            name,
            args,
            children,
            line,
        })
    }

    fn parse_node_name(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Some(b'"') => self.parse_quoted_string(),
            Some(c) if is_ident_start(c) => self.parse_bare_ident(),
            Some(_) => Err(self.error("expected node name (identifier or quoted string)")),
            None => Err(self.error("unexpected end of input; expected node name")),
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            Some(b'"') => Ok(Value::String(self.parse_quoted_string()?)),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            Some(c) if is_ident_start(c) => {
                let id = self.parse_bare_ident()?;
                Ok(match id.as_str() {
                    "true" => Value::Boolean(true),
                    "false" => Value::Boolean(false),
                    _ => Value::Ident(id),
                })
            }
            Some(_) => Err(self.error("expected value")),
            None => Err(self.error("unexpected end of input; expected value")),
        }
    }

    fn parse_quoted_string(&mut self) -> Result<String, ParseError> {
        debug_assert_eq!(self.peek(), Some(b'"'));
        self.advance(); // opening quote
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(b'"') => {
                    self.advance();
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.advance();
                    match self.peek() {
                        Some(b'"') => {
                            out.push('"');
                            self.advance();
                        }
                        Some(b'\\') => {
                            out.push('\\');
                            self.advance();
                        }
                        Some(b'n') => {
                            out.push('\n');
                            self.advance();
                        }
                        Some(b't') => {
                            out.push('\t');
                            self.advance();
                        }
                        Some(b'r') => {
                            out.push('\r');
                            self.advance();
                        }
                        Some(_) => return Err(self.error("unknown escape sequence in string")),
                        None => return Err(self.error("unterminated string escape")),
                    }
                }
                Some(b'\n') | None => {
                    return Err(self.error("unterminated string"));
                }
                Some(c) => {
                    out.push(c as char);
                    self.advance();
                }
            }
        }
    }

    fn parse_bare_ident(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_ident_continue(c) {
                self.advance();
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(self.error("expected identifier"));
        }
        Ok(std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.error("invalid UTF-8 in identifier"))?
            .to_string())
    }

    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.advance();
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.error("invalid UTF-8 in number"))?;
        text.parse::<i64>()
            .map(Value::Integer)
            .map_err(|_| self.error("invalid integer"))
    }

    // ---- whitespace + comments ----

    /// Skip whitespace, line comments, AND newlines. Used between nodes.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') => {
                    self.advance();
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                _ => return,
            }
        }
    }

    /// Skip only inline whitespace + line comments. Stops at newlines —
    /// used inside a node where the newline terminates the node.
    fn skip_line_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') => {
                    self.advance();
                }
                Some(b'/') if self.peek_at(1) == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                _ => return,
            }
        }
    }

    // ---- cursor primitives ----

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn error(&self, msg: &str) -> ParseError {
        ParseError {
            line: self.line,
            col: self.col,
            message: msg.to_string(),
        }
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_continue(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(input: &str) -> Vec<Node> {
        parse(input).expect("parse should succeed")
    }

    #[test]
    fn empty_input() {
        assert!(p("").is_empty());
        assert!(p("   \n  ").is_empty());
        assert!(p("// just a comment\n").is_empty());
    }

    #[test]
    fn single_node_no_args() {
        let nodes = p("hello\n");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "hello");
        assert!(nodes[0].args.is_empty());
        assert!(nodes[0].children.is_empty());
    }

    #[test]
    fn node_with_string_arg() {
        let nodes = p(r#"preview "off""#);
        assert_eq!(nodes[0].name, "preview");
        assert_eq!(nodes[0].args, vec![Value::String("off".to_string())]);
    }

    #[test]
    fn node_with_integer_arg() {
        let nodes = p("recent_lines 150");
        assert_eq!(nodes[0].args, vec![Value::Integer(150)]);
    }

    #[test]
    fn node_with_negative_integer() {
        let nodes = p("offset -42");
        assert_eq!(nodes[0].args, vec![Value::Integer(-42)]);
    }

    #[test]
    fn node_with_boolean_args() {
        let nodes = p("flags true false");
        assert_eq!(
            nodes[0].args,
            vec![Value::Boolean(true), Value::Boolean(false)]
        );
    }

    #[test]
    fn node_with_multiple_string_args() {
        let nodes = p(r#"actions "open" "copy" "insert""#);
        assert_eq!(nodes[0].args.len(), 3);
        assert_eq!(nodes[0].args[0].as_string(), Some("open"));
        assert_eq!(nodes[0].args[2].as_string(), Some("insert"));
    }

    #[test]
    fn node_with_block() {
        let nodes = p("ui {\n  preview \"off\"\n  mask_secrets false\n}");
        assert_eq!(nodes[0].name, "ui");
        assert_eq!(nodes[0].children.len(), 2);
        assert_eq!(nodes[0].children[0].name, "preview");
        assert_eq!(nodes[0].children[1].name, "mask_secrets");
    }

    #[test]
    fn nested_blocks() {
        let nodes = p(
            r#"
            grab {
                profiles {
                    quick { lines 150 }
                    deep { lines 1500 }
                }
            }
            "#,
        );
        assert_eq!(nodes[0].name, "grab");
        assert_eq!(nodes[0].children[0].name, "profiles");
        assert_eq!(nodes[0].children[0].children.len(), 2);
        assert_eq!(nodes[0].children[0].children[0].name, "quick");
        assert_eq!(
            nodes[0].children[0].children[0].children[0].args,
            vec![Value::Integer(150)]
        );
    }

    #[test]
    fn semicolon_separator() {
        let nodes = p("limits { copy 100; insert 5; open 10 }");
        let children = &nodes[0].children;
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].name, "copy");
        assert_eq!(children[2].name, "open");
    }

    #[test]
    fn line_comments_skipped() {
        let nodes = p(
            r#"
            // first comment
            ui {
                preview "off"  // trailing comment
                // mid-block
                mask_secrets false
            }
            // final
            "#,
        );
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].children.len(), 2);
    }

    #[test]
    fn string_escapes() {
        let nodes = p(r#"template "with \"quote\" and \\ slash and \n newline""#);
        let s = nodes[0].args[0].as_string().unwrap();
        assert_eq!(s, "with \"quote\" and \\ slash and \n newline");
    }

    #[test]
    fn bare_identifier_value_is_ident_not_string() {
        // KDL allows `default_profile quick` without quotes.
        let nodes = p("default_profile quick");
        assert!(matches!(&nodes[0].args[0], Value::Ident(s) if s == "quick"));
        // But the caller can read it as a string lossy.
        assert_eq!(nodes[0].args[0].as_str_lossy(), "quick");
    }

    #[test]
    fn line_numbers_tracked() {
        let nodes = p("first\nsecond\nthird\n");
        assert_eq!(nodes[0].line, 1);
        assert_eq!(nodes[1].line, 2);
        assert_eq!(nodes[2].line, 3);
    }

    // ---- error paths ----

    #[test]
    fn unterminated_string_errors_with_location() {
        let err = parse(r#"foo "unterminated"#).unwrap_err();
        assert!(err.message.contains("unterminated string"), "{err:?}");
    }

    #[test]
    fn unexpected_close_brace_at_doc_level() {
        let err = parse("}").unwrap_err();
        assert!(err.message.contains("unexpected '}'"), "{err:?}");
    }

    #[test]
    fn unknown_escape_errors() {
        let err = parse(r#"foo "bad \z escape""#).unwrap_err();
        assert!(err.message.contains("unknown escape"), "{err:?}");
    }

    #[test]
    fn missing_node_name_errors() {
        // A bare `123` at document position is not a node name.
        let err = parse("123 foo").unwrap_err();
        assert!(err.message.contains("expected node name"), "{err:?}");
    }

    // ---- realistic schema-shaped fixture parses without error ----

    #[test]
    fn realistic_zextract_kdl_parses() {
        let input = r#"
            ui {
                preview "off"
                preview_open_width "90%"
                preview_closed_width "70%"
                mask_secrets false
                editor_command_prefix "nvim"
            }

            grab {
                default_profile "quick"
                profiles {
                    quick    { source "scrollback"  lines 150  }
                    deep     { source "scrollback"  lines 1500 }
                    viewport { source "viewport"              }
                    full     { source "scrollback"            }
                }
            }

            patterns {
                url  { enabled true }
                file { enabled true }
                jira {
                    regex "[A-Z]+-[0-9]+"
                    type "url"
                    template "https://jira.example.com/browse/{match}"
                }
            }

            types {
                url { actions "open" "copy" "insert"; default "open" }
            }

            actions {
                url {
                    open command "open {url}"
                }
            }

            limits { copy 100; insert 5; open 10 }
        "#;
        let nodes = parse(input).expect("realistic config parses");
        // Top-level: ui, grab, patterns, types, actions, limits = 6
        assert_eq!(nodes.len(), 6);
        let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["ui", "grab", "patterns", "types", "actions", "limits"]);
    }
}
