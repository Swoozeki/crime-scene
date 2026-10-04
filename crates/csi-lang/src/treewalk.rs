//! A single generic tree walk that extracts functions, cyclomatic complexity and nesting
//! for every parsed language. Language differences live in the small tables below.

use crate::{Function, Lang};
use std::cell::RefCell;
use std::collections::HashMap;
use tree_sitter::{Language, Node, Parser};

pub struct WalkResult {
    pub complexity: u32,
    pub functions: Vec<Function>,
    pub methods: u32,
    pub ctor_params: u32,
    pub top_nesting: u32,
}

thread_local! {
    static PARSERS: RefCell<HashMap<Lang, Parser>> = RefCell::new(HashMap::new());
}

fn language(lang: Lang) -> Option<Language> {
    Some(match lang {
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::Php => tree_sitter_php::LANGUAGE_PHP.into(),
        Lang::Template => tree_sitter_angular::language(),
        Lang::Scss => tree_sitter_scss::language(),
        Lang::Css => tree_sitter_css::LANGUAGE.into(),
        _ => return None,
    })
}

pub fn walk(lang: Lang, text: &str) -> Option<WalkResult> {
    let tree = PARSERS.with(|p| {
        let mut map = p.borrow_mut();
        if let std::collections::hash_map::Entry::Vacant(e) = map.entry(lang) {
            let mut parser = Parser::new();
            parser.set_language(&language(lang)?).ok()?;
            e.insert(parser);
        }
        map.get_mut(&lang)?.parse(text, None)
    })?;
    let mut w = Walker {
        lang,
        src: text.as_bytes(),
        fns: vec![],
        top_decisions: 0,
        top_nesting: 0,
        methods: 0,
        ctor_params: 0,
    };
    w.visit(tree.root_node(), None, 0, 0);

    let mut seen: HashMap<String, u32> = HashMap::new();
    let functions: Vec<Function> = w
        .fns
        .into_iter()
        .map(|f| {
            let n = seen.entry(f.name.clone()).or_insert(0);
            *n += 1;
            let name = if *n > 1 { format!("{}#{}", f.name, n) } else { f.name };
            Function { name, start: f.start, end: f.end, cc: f.cc, nesting: f.nesting, loc: f.end - f.start + 1 }
        })
        .collect();
    let complexity = functions.iter().map(|f| f.cc).sum::<u32>() + w.top_decisions;
    Some(WalkResult {
        complexity: complexity.max(1),
        functions,
        methods: w.methods,
        ctor_params: w.ctor_params,
        top_nesting: w.top_nesting,
    })
}

struct FnAcc {
    name: String,
    start: u32,
    end: u32,
    cc: u32,
    nesting: u32,
}

enum FnAction {
    New(String),
    Fold,
}

struct Walker<'a> {
    lang: Lang,
    src: &'a [u8],
    fns: Vec<FnAcc>,
    top_decisions: u32,
    top_nesting: u32,
    methods: u32,
    ctor_params: u32,
}

const MAX_DEPTH: u32 = 1500;

impl Walker<'_> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn short(&self, n: Node, max: usize) -> String {
        let t = self.text(n).lines().next().unwrap_or("").trim();
        let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.chars().count() > max { format!("{}…", t.chars().take(max).collect::<String>()) } else { t }
    }

    fn visit(&mut self, node: Node, cur: Option<usize>, depth: u32, guard: u32) {
        if guard > MAX_DEPTH {
            return;
        }
        let kind = node.kind();
        let mut cur = cur;
        let mut depth = depth;
        let mut new_fn = false;

        if self.is_function(node, kind) {
            match self.classify(node, kind, cur.is_some()) {
                FnAction::New(name) => {
                    self.fns.push(FnAcc {
                        name,
                        start: node.start_position().row as u32 + 1,
                        end: node.end_position().row as u32 + 1,
                        cc: 1,
                        nesting: 0,
                    });
                    cur = Some(self.fns.len() - 1);
                    depth = 0;
                    new_fn = true;
                }
                FnAction::Fold => {
                    if self.is_decision(node, kind) {
                        self.decide(cur);
                    }
                }
            }
            if matches!(kind, "method_definition" | "method_declaration") {
                self.methods += 1;
                if kind == "method_definition"
                    && node.child_by_field_name("name").map(|n| self.text(n)) == Some("constructor")
                    && let Some(p) = node.child_by_field_name("parameters")
                {
                    self.ctor_params += p.named_child_count() as u32;
                }
            }
        } else if self.is_decision(node, kind) {
            self.decide(cur);
        }

        if !new_fn && self.is_nesting(node, kind) {
            depth += 1;
            match cur {
                Some(i) => self.fns[i].nesting = self.fns[i].nesting.max(depth),
                None => self.top_nesting = self.top_nesting.max(depth),
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child, cur, depth, guard + 1);
        }
    }

    fn decide(&mut self, cur: Option<usize>) {
        match cur {
            Some(i) => self.fns[i].cc += 1,
            None => self.top_decisions += 1,
        }
    }

    fn is_function(&self, node: Node, kind: &str) -> bool {
        match self.lang {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => matches!(
                kind,
                "function_declaration"
                    | "generator_function_declaration"
                    | "function_expression"
                    | "function"
                    | "generator_function"
                    | "arrow_function"
                    | "method_definition"
            ),
            Lang::Php => {
                matches!(kind, "function_definition" | "method_declaration" | "anonymous_function" | "arrow_function")
            }
            Lang::Template => match kind {
                "if_statement" | "for_statement" | "switch_statement" | "defer_statement" => true,
                "element" => {
                    node.child(0).filter(|c| c.kind() == "start_tag").is_some_and(|t| self.text(t).contains("*ng"))
                }
                _ => false,
            },
            Lang::Scss | Lang::Css => kind == "rule_set",
            _ => false,
        }
    }

    fn is_decision(&self, node: Node, kind: &str) -> bool {
        let (kinds, ops): (&[&str], &[&str]) = match self.lang {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => (
                &[
                    "if_statement",
                    "for_statement",
                    "for_in_statement",
                    "while_statement",
                    "do_statement",
                    "switch_case",
                    "catch_clause",
                    "ternary_expression",
                ],
                &["&&", "||", "??"],
            ),
            Lang::Php => (
                &[
                    "if_statement",
                    "else_if_clause",
                    "for_statement",
                    "foreach_statement",
                    "while_statement",
                    "do_statement",
                    "case_statement",
                    "catch_clause",
                    "conditional_expression",
                    "match_conditional_expression",
                ],
                &["&&", "||", "and", "or", "??"],
            ),
            Lang::Template => (
                &[
                    "if_statement",
                    "else_if_statement",
                    "for_statement",
                    "case_statement",
                    "structural_directive",
                    "ternary_expression",
                    "conditional_expression",
                    "nullish_coalescing_expression",
                ],
                &["&&", "||"],
            ),
            Lang::Scss | Lang::Css => (
                &["rule_set", "if_statement", "else_if_clause", "each_statement", "for_statement", "while_statement"],
                &[],
            ),
            _ => (&[], &[]),
        };
        if kinds.contains(&kind) {
            return true;
        }
        if !ops.is_empty() && kind == "binary_expression" {
            let mut c = node.walk();
            return node.children(&mut c).any(|ch| !ch.is_named() && ops.contains(&ch.kind()));
        }
        false
    }

    fn is_nesting(&self, node: Node, kind: &str) -> bool {
        let kinds: &[&str] = match self.lang {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => &[
                "if_statement",
                "for_statement",
                "for_in_statement",
                "while_statement",
                "do_statement",
                "switch_statement",
                "catch_clause",
            ],
            Lang::Php => &[
                "if_statement",
                "for_statement",
                "foreach_statement",
                "while_statement",
                "do_statement",
                "switch_statement",
                "match_expression",
                "catch_clause",
            ],
            Lang::Template => &["if_statement", "for_statement", "switch_statement", "defer_statement"],
            Lang::Scss | Lang::Css => {
                &["rule_set", "media_statement", "if_statement", "each_statement", "for_statement"]
            }
            _ => &[],
        };
        if !kinds.contains(&kind) {
            return false;
        }
        // `else if` chains do not deepen nesting
        !(kind == "if_statement" && node.parent().is_some_and(|p| p.kind() == "else_clause"))
    }

    fn classify(&self, node: Node, kind: &str, enclosed: bool) -> FnAction {
        match self.lang {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => match self.ts_name(node, kind) {
                Some(n) => FnAction::New(n),
                None if enclosed => FnAction::Fold,
                None => FnAction::New(self.call_context(node).unwrap_or_else(|| "<anonymous>".into())),
            },
            Lang::Php => match kind {
                "function_definition" => FnAction::New(self.field(node, "name").unwrap_or("<function>").to_string()),
                "method_declaration" => {
                    let m = self.field(node, "name").unwrap_or("<method>");
                    FnAction::New(match self.enclosing_type(node) {
                        Some(c) => format!("{c}::{m}"),
                        None => m.to_string(),
                    })
                }
                _ if enclosed => FnAction::Fold,
                _ => FnAction::New(self.call_context(node).unwrap_or_else(|| "<closure>".into())),
            },
            Lang::Template => {
                if enclosed {
                    FnAction::Fold
                } else if kind == "element" {
                    FnAction::New(node.child(0).map(|t| self.short(t, 60)).unwrap_or_default())
                } else {
                    FnAction::New(self.short(node, 60).trim_end_matches('{').trim().to_string())
                }
            }
            Lang::Scss | Lang::Css => {
                if enclosed {
                    FnAction::Fold
                } else {
                    let sel = node
                        .named_child(0)
                        .filter(|c| c.kind() == "selectors")
                        .map(|c| self.short(c, 60))
                        .unwrap_or_else(|| "<rule>".into());
                    FnAction::New(sel)
                }
            }
            _ => FnAction::Fold,
        }
    }

    fn field<'b>(&'b self, node: Node, name: &str) -> Option<&'b str> {
        node.child_by_field_name(name).map(|n| self.text(n))
    }

    fn ts_name(&self, node: Node, kind: &str) -> Option<String> {
        match kind {
            "function_declaration" | "generator_function_declaration" => {
                Some(self.field(node, "name").unwrap_or("default").to_string())
            }
            "method_definition" => {
                let m = self.field(node, "name")?;
                Some(match self.enclosing_type(node) {
                    Some(c) => format!("{c}.{m}"),
                    None => m.to_string(),
                })
            }
            _ => {
                if let Some(n) = self.field(node, "name") {
                    return Some(n.to_string());
                }
                let parent = node.parent()?;
                let name = match parent.kind() {
                    "variable_declarator" => self.field(parent, "name")?.to_string(),
                    "public_field_definition" | "field_definition" => {
                        let f = self.field(parent, "name").or_else(|| self.field(parent, "property"))?;
                        match self.enclosing_type(parent) {
                            Some(c) => format!("{c}.{f}"),
                            None => f.to_string(),
                        }
                    }
                    "pair" => self.field(parent, "key")?.trim_matches(['"', '\'']).to_string(),
                    "assignment_expression" => self.short(parent.child_by_field_name("left")?, 60),
                    "export_statement" => "default".to_string(),
                    _ => return None,
                };
                Some(name)
            }
        }
    }

    fn enclosing_type(&self, node: Node) -> Option<String> {
        let mut p = node.parent();
        while let Some(n) = p {
            if matches!(
                n.kind(),
                "class_declaration"
                    | "class"
                    | "abstract_class_declaration"
                    | "trait_declaration"
                    | "interface_declaration"
                    | "enum_declaration"
            ) {
                return self.field(n, "name").map(str::to_string);
            }
            p = n.parent();
        }
        None
    }

    /// `describe('cart')` for an anonymous top-level callback passed to a call.
    fn call_context(&self, node: Node) -> Option<String> {
        let args = node.parent()?;
        if !matches!(args.kind(), "arguments" | "argument") {
            return None;
        }
        let call = if args.kind() == "argument" { args.parent()?.parent()? } else { args.parent()? };
        let callee = call.child_by_field_name("function").map(|f| self.short(f, 40))?;
        let first = call
            .child_by_field_name("arguments")
            .and_then(|a| a.named_child(0))
            .filter(|a| matches!(a.kind(), "string" | "template_string" | "encapsed_string" | "argument"))
            .map(|a| self.short(a, 50));
        Some(match first {
            Some(f) => format!("{callee}({f})"),
            None => format!("{callee}(…)"),
        })
    }
}
