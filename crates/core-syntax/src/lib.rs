use std::path::Path;
use tree_sitter::{Language, Node, Parser, Tree};

pub struct Highlighter {
    parser: Parser,
    tree: Option<Tree>,
    language: String,
}

#[derive(Debug, Clone)]
pub struct HighlightedLine {
    pub spans: Vec<HighlightSpan>,
}

#[derive(Debug, Clone)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub token_type: String,
}

impl Highlighter {
    pub fn new(language: &str) -> Option<Self> {
        let lang = language_for(language)?;
        let mut parser = Parser::new();
        parser.set_language(&lang).ok()?;
        Some(Self {
            parser,
            tree: None,
            language: language.to_string(),
        })
    }

    pub fn parse(&mut self, source: &str) {
        self.tree = self.parser.parse(source, self.tree.as_ref());
    }

    pub fn highlight_line(&self, source: &str, line_idx: usize) -> HighlightedLine {
        let tree = match &self.tree {
            Some(t) => t,
            None => return HighlightedLine { spans: Vec::new() },
        };

        // Find the byte range of the requested line
        let mut line_start = 0usize;
        let mut current_line = 0usize;
        for (i, ch) in source.char_indices() {
            if current_line == line_idx {
                line_start = i;
                break;
            }
            if ch == '\n' {
                current_line += 1;
            }
        }
        if current_line < line_idx {
            return HighlightedLine { spans: Vec::new() };
        }

        let line_end = source[line_start..]
            .find('\n')
            .map(|pos| line_start + pos)
            .unwrap_or(source.len());

        let line_text = &source[line_start..line_end];
        if line_text.is_empty() {
            return HighlightedLine { spans: Vec::new() };
        }

        let mut spans = Vec::new();
        let root = tree.root_node();
        self.collect_spans(&root, line_start, line_end, &mut spans);

        // Convert byte offsets to be relative to line start
        let mut line_spans: Vec<HighlightSpan> = spans
            .into_iter()
            .filter_map(|span| {
                let start = span.start.max(line_start);
                let end = span.end.min(line_end);
                if start < end {
                    // Convert byte offsets to character offsets within line
                    let rel_start = count_chars(&source[line_start..start]);
                    let rel_end = count_chars(&source[line_start..end]);
                    Some(HighlightSpan {
                        start: rel_start,
                        end: rel_end,
                        token_type: span.token_type,
                    })
                } else {
                    None
                }
            })
            .collect();

        line_spans.sort_by_key(|s| s.start);
        // Remove duplicates / overlaps — keep the most specific (last in tree order)
        line_spans.dedup_by(|b, a| a.start == b.start && a.end == b.end);

        HighlightedLine { spans: line_spans }
    }

    pub fn supported_languages() -> Vec<&'static str> {
        vec![
            "rust",
            "javascript",
            "typescript",
            "python",
            "go",
            "json",
            "toml",
            "yaml",
            "markdown",
            "bash",
            "css",
            "html",
        ]
    }

    fn collect_spans(
        &self,
        node: &Node,
        line_start: usize,
        line_end: usize,
        spans: &mut Vec<HighlightSpan>,
    ) {
        let node_start = node.start_byte();
        let node_end = node.end_byte();

        // Skip nodes that don't overlap with our line
        if node_end <= line_start || node_start >= line_end {
            return;
        }

        // Map node kind to token type
        if let Some(token_type) = map_node_to_token(node, &self.language) {
            // Only add leaf-ish nodes or nodes we specifically want
            if node.child_count() == 0 || is_significant_parent(node, &self.language) {
                spans.push(HighlightSpan {
                    start: node_start,
                    end: node_end,
                    token_type,
                });
            }
        }

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_spans(&child, line_start, line_end, spans);
        }
    }
}

fn count_chars(s: &str) -> usize {
    s.chars().count()
}

fn language_for(name: &str) -> Option<Language> {
    match name {
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        "javascript" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "json" => Some(tree_sitter_json::LANGUAGE.into()),
        "toml" => Some(tree_sitter_toml_ng::LANGUAGE.into()),
        "yaml" => Some(tree_sitter_yaml::LANGUAGE.into()),
        "markdown" => Some(tree_sitter_md::LANGUAGE.into()),
        "bash" => Some(tree_sitter_bash::LANGUAGE.into()),
        "css" => Some(tree_sitter_css::LANGUAGE.into()),
        "html" => Some(tree_sitter_html::LANGUAGE.into()),
        _ => None,
    }
}

pub fn detect_language(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust".to_string(),
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript".to_string(),
        Some("ts") | Some("tsx") => "typescript".to_string(),
        Some("py") | Some("pyi") => "python".to_string(),
        Some("go") => "go".to_string(),
        Some("json") => "json".to_string(),
        Some("toml") => "toml".to_string(),
        Some("yml") | Some("yaml") => "yaml".to_string(),
        Some("md") | Some("markdown") => "markdown".to_string(),
        Some("sh") | Some("bash") | Some("zsh") => "bash".to_string(),
        Some("css") => "css".to_string(),
        Some("html") | Some("htm") => "html".to_string(),
        _ => "text".to_string(),
    }
}

fn is_significant_parent(_node: &Node, _lang: &str) -> bool {
    false
}

fn map_node_to_token(node: &Node, lang: &str) -> Option<String> {
    let kind = node.kind();

    // Universal mappings that apply across languages
    let token = match kind {
        // Comments
        "line_comment" | "block_comment" | "comment" => "comment",

        // Strings
        "string_literal" | "string" | "string_content" | "raw_string_literal"
        | "interpreted_string_literal" | "template_string" | "string_fragment" => "string",
        "char_literal" => "string",

        // Numbers
        "integer_literal" | "float_literal" | "number" | "int_literal"
        | "imaginary_literal" | "integer" | "float" => "number",

        // Booleans
        "true" | "false" | "boolean" => "constant.builtin",
        "none" | "nil" => "constant.builtin",

        // Identifiers and names — depends on context
        "type_identifier" | "type_name" | "type_spec" => "type",
        "field_identifier" | "property_identifier" => "property",
        "shorthand_field_identifier" => "property",

        // Function-related
        "function_item" => return None, // parent, skip
        "call_expression" => return None,

        // Operators
        "!" | "!=" | "%" | "&" | "&&" | "*" | "+" | "-" | "/" | "<" | "<<" | "<="
        | "=" | "==" | ">" | ">=" | ">>" | "^" | "|" | "||" | "~" | "+=" | "-="
        | "*=" | "/=" | "=>" | "->" | ".." | "..=" | "::" => "operator",

        // Punctuation
        "(" | ")" | "[" | "]" | "{" | "}" => "punctuation.bracket",
        "," | ";" | ":" | "." => "punctuation.delimiter",
        "#" | "##" => "punctuation.special",

        // Attributes
        "attribute_item" | "attribute" | "decorator" => "attribute",

        _ => {
            // Language-specific mappings
            return map_language_specific(node, kind, lang);
        }
    };

    Some(token.to_string())
}

fn map_language_specific(node: &Node, kind: &str, lang: &str) -> Option<String> {
    match lang {
        "rust" => map_rust_node(node, kind),
        "javascript" | "typescript" => map_js_node(node, kind),
        "python" => map_python_node(node, kind),
        "go" => map_go_node(node, kind),
        "json" => map_json_node(kind),
        "toml" => map_toml_node(kind),
        "yaml" => map_yaml_node(kind),
        "bash" => map_bash_node(kind),
        "css" => map_css_node(kind),
        "html" => map_html_node(kind),
        _ => None,
    }
}

fn map_rust_node(node: &Node, kind: &str) -> Option<String> {
    let token = match kind {
        // Keywords
        "fn" | "let" | "mut" | "const" | "static" | "pub" | "use" | "mod" | "crate"
        | "extern" | "self" | "super" | "struct" | "enum" | "trait" | "impl" | "type"
        | "where" | "as" | "ref" | "move" | "async" | "await" | "dyn" | "unsafe"
        | "abstract" | "become" | "box" | "do" | "final" | "macro" | "override"
        | "priv" | "typeof" | "unsized" | "virtual" | "yield" => "keyword",
        "if" | "else" | "match" | "for" | "while" | "loop" | "break" | "continue"
        | "return" | "in" => "keyword.control",

        // Primitive types
        "primitive_type" | "mutable_specifier" => "type.builtin",

        // Identifiers — check parent context
        "identifier" => {
            if let Some(parent) = node.parent() {
                match parent.kind() {
                    "function_item" | "function_signature_item" => {
                        // Is this the function name?
                        if parent
                            .child_by_field_name("name")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("function".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    "call_expression" => {
                        if parent
                            .child_by_field_name("function")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("function.call".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    "parameter" | "closure_parameters" => {
                        return Some("variable.parameter".to_string())
                    }
                    "use_declaration" | "use_as_clause" | "use_list" | "scoped_identifier" => {
                        return None
                    }
                    "struct_item" | "enum_item" | "trait_item" | "type_item" => {
                        return Some("type.definition".to_string())
                    }
                    "field_declaration" => return Some("property".to_string()),
                    "field_expression" => {
                        if parent
                            .child_by_field_name("field")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("property".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    _ => return Some("variable".to_string()),
                }
            }
            return Some("variable".to_string());
        }

        // Macros
        "macro_invocation" => return None,
        "macro_rules!" => "keyword",
        // Macro name (the ! is separate)
        "!" if node
            .parent()
            .is_some_and(|p| p.kind() == "macro_invocation") =>
        {
            "function.call"
        }

        // Lifetime
        "lifetime" => "keyword",

        // Self (capital)
        "Self" => "type.builtin",

        _ => return None,
    };
    Some(token.to_string())
}

fn map_js_node(node: &Node, kind: &str) -> Option<String> {
    let token = match kind {
        // Keywords
        "function" | "const" | "let" | "var" | "class" | "new" | "this" | "typeof"
        | "instanceof" | "void" | "delete" | "in" | "of" | "async" | "await" | "yield"
        | "static" | "get" | "set" | "extends" | "super" | "with" | "debugger" => "keyword",
        "if" | "else" | "for" | "while" | "do" | "switch" | "case" | "default" | "break"
        | "continue" | "return" | "throw" | "try" | "catch" | "finally" => "keyword.control",
        "import" | "export" | "from" | "as" => "keyword.import",

        // Built-in constants
        "null" | "undefined" => "constant.builtin",

        // Identifiers
        "identifier" | "property_identifier" => {
            if let Some(parent) = node.parent() {
                match parent.kind() {
                    "function_declaration" | "method_definition" | "function" => {
                        if parent
                            .child_by_field_name("name")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("function".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    "call_expression" => {
                        if parent
                            .child_by_field_name("function")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("function.call".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    "formal_parameters" => return Some("variable.parameter".to_string()),
                    "pair" => return Some("property".to_string()),
                    _ => return Some("variable".to_string()),
                }
            }
            return Some("variable".to_string());
        }

        // Regex
        "regex" | "regex_pattern" => "string.special",

        // Template
        "template_substitution" => return None,
        "`" => "string",
        "${" | "}" if node
            .parent()
            .is_some_and(|p| p.kind() == "template_substitution") =>
        {
            "punctuation.special"
        }

        _ => return None,
    };
    Some(token.to_string())
}

fn map_python_node(node: &Node, kind: &str) -> Option<String> {
    let token = match kind {
        // Keywords
        "def" | "class" | "lambda" | "global" | "nonlocal" | "del" | "assert" | "with"
        | "as" | "is" | "in" | "not" | "and" | "or" | "async" | "await" | "yield" => "keyword",
        "if" | "elif" | "else" | "for" | "while" | "break" | "continue" | "return" | "try"
        | "except" | "finally" | "raise" | "pass" => "keyword.control",
        "import" | "from" => "keyword.import",

        // Built-in constants
        "None" => "constant.builtin",
        "True" | "False" => "constant.builtin",

        // Identifiers
        "identifier" => {
            if let Some(parent) = node.parent() {
                match parent.kind() {
                    "function_definition" => {
                        if parent
                            .child_by_field_name("name")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("function".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    "call" => {
                        if parent
                            .child_by_field_name("function")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("function.call".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    "parameters" | "default_parameter" | "typed_parameter" => {
                        return Some("variable.parameter".to_string())
                    }
                    "class_definition" => {
                        if parent
                            .child_by_field_name("name")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("type.definition".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    _ => return Some("variable".to_string()),
                }
            }
            return Some("variable".to_string());
        }

        // Decorators
        "decorator" => "attribute",

        _ => return None,
    };
    Some(token.to_string())
}

fn map_go_node(node: &Node, kind: &str) -> Option<String> {
    let token = match kind {
        // Keywords
        "func" | "var" | "const" | "type" | "struct" | "interface" | "map" | "chan"
        | "package" | "import" | "defer" | "go" | "range" | "select" | "case"
        | "default" | "fallthrough" => "keyword",
        "if" | "else" | "for" | "switch" | "break" | "continue" | "return" | "goto" => {
            "keyword.control"
        }

        // Identifiers
        "identifier" | "field_identifier" => {
            if let Some(parent) = node.parent() {
                match parent.kind() {
                    "function_declaration" | "method_declaration" => {
                        if parent
                            .child_by_field_name("name")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("function".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    "call_expression" => {
                        if parent
                            .child_by_field_name("function")
                            .is_some_and(|n| n.id() == node.id())
                        {
                            return Some("function.call".to_string());
                        }
                        return Some("variable".to_string());
                    }
                    "type_spec" => return Some("type.definition".to_string()),
                    "field_declaration" => return Some("property".to_string()),
                    _ => {
                        if kind == "field_identifier" {
                            return Some("property".to_string());
                        }
                        return Some("variable".to_string());
                    }
                }
            }
            return Some("variable".to_string());
        }

        // Built-in types
        "type_identifier" => "type",

        // nil
        "nil" => "constant.builtin",
        "iota" => "constant.builtin",

        _ => return None,
    };
    Some(token.to_string())
}

fn map_json_node(kind: &str) -> Option<String> {
    let token = match kind {
        "string" | "string_content" => "string",
        "number" => "number",
        "true" | "false" => "constant.builtin",
        "null" => "constant.builtin",
        "pair" => return None,
        _ => return None,
    };
    Some(token.to_string())
}

fn map_toml_node(kind: &str) -> Option<String> {
    let token = match kind {
        "string" | "bare_key" => "string",
        "integer" | "float" => "number",
        "boolean" => "constant.builtin",
        "table" | "table_array_element" => return None,
        "key" => "property",
        _ => return None,
    };
    Some(token.to_string())
}

fn map_yaml_node(kind: &str) -> Option<String> {
    let token = match kind {
        "string_scalar" | "double_quote_scalar" | "single_quote_scalar" => "string",
        "integer_scalar" | "float_scalar" => "number",
        "boolean_scalar" | "null_scalar" => "constant.builtin",
        "anchor" | "alias" => "keyword",
        "tag" => "type",
        "key" | "block_mapping_pair" => return None,
        _ => return None,
    };
    Some(token.to_string())
}

fn map_bash_node(kind: &str) -> Option<String> {
    let token = match kind {
        "if" | "then" | "else" | "elif" | "fi" | "for" | "while" | "do" | "done" | "case"
        | "esac" | "in" | "function" | "select" | "until" => "keyword",
        "command_name" => "function.call",
        "variable_name" => "variable",
        "string" | "raw_string" | "heredoc_body" | "heredoc_start" => "string",
        "number" => "number",
        "file_redirect" | "heredoc_redirect" => "operator",
        "special_variable_name" => "variable.builtin",
        _ => return None,
    };
    Some(token.to_string())
}

fn map_css_node(kind: &str) -> Option<String> {
    let token = match kind {
        "tag_name" => "type",
        "class_name" | "id_name" => "variable",
        "property_name" => "property",
        "color_value" | "integer_value" | "float_value" => "number",
        "string_value" => "string",
        "plain_value" | "keyword_query" => "constant",
        "pseudo_class_selector" | "pseudo_element_selector" => "keyword",
        "important" => "keyword",
        "function_name" => "function",
        _ => return None,
    };
    Some(token.to_string())
}

fn map_html_node(kind: &str) -> Option<String> {
    let token = match kind {
        "tag_name" => "keyword",
        "attribute_name" => "variable",
        "attribute_value" | "quoted_attribute_value" => "string",
        "doctype" => "keyword",
        "text" => return None,
        "<" | ">" | "/>" | "</" => "punctuation.bracket",
        "=" => "operator",
        _ => return None,
    };
    Some(token.to_string())
}
