use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use syn::{Arm, Block, ImplItem, Item, Lit, ext::IdentExt, fold::Fold, visit::Visit};

use crate::{Error, Fingerprints, Hash, combine, insert, matcher, named_query};

/// Parse once and fingerprint matching functions or match arms.
///
/// Queries are `fn:<glob>` or `arm:<fn-glob>:<integer-or-string>`, optionally
/// prefixed with `name=`. A named query aggregates all its matches into one key.
/// Missing queries fail unless `allow_missing` is true. Helpers are matched by
/// unqualified identifier anywhere in this file, not by compiler name resolution.
pub fn fingerprint_rust(
    source: &str,
    queries: &[impl AsRef<str>],
    allow_missing: bool,
) -> Result<Fingerprints, Error> {
    fingerprint_rust_groups(source, &[queries], allow_missing).map(|mut groups| groups.remove(0))
}

pub(crate) fn fingerprint_rust_groups(
    source: &str,
    groups: &[&[impl AsRef<str>]],
    allow_missing: bool,
) -> Result<Vec<Fingerprints>, Error> {
    let file = StripDocs.fold_file(syn::parse_file(source)?);
    let mut index = Index::default();
    index.items(&file.items, &[]);
    index.assign_keys();
    let mut helpers: BTreeMap<&str, Vec<&Definition>> = BTreeMap::new();
    for definition in index
        .functions
        .iter()
        .map(|function| &function.definition)
        .chain(&index.other)
    {
        helpers
            .entry(&definition.name)
            .or_default()
            .push(definition);
    }
    groups
        .iter()
        .map(|queries| {
            let mut results = Fingerprints::new();
            for raw in *queries {
                let raw = raw.as_ref();
                let (alias, query) = named_query(raw, true)?;
                let branch = query.starts_with("branch:");
                let (pattern, literal) = if let Some(pattern) = query.strip_prefix("fn:") {
                    (pattern, None)
                } else if let Some(arm) = query
                    .strip_prefix("arm:")
                    .or_else(|| query.strip_prefix("branch:"))
                {
                    let (pattern, value) = arm.split_once(':').ok_or_else(|| invalid(raw))?;
                    (
                        pattern,
                        Some(Literal::parse(value).ok_or_else(|| invalid(raw))?),
                    )
                } else {
                    return Err(invalid(raw));
                };
                if pattern.is_empty() {
                    return Err(invalid(raw));
                }
                let glob = matcher(pattern, raw)?;
                let mut matches = Fingerprints::new();
                for function in &index.functions {
                    if !glob.is_match(&function.definition.name) {
                        continue;
                    }
                    let (key, tokens, references) = if let Some(literal) = &literal {
                        let mut arms = Arms::default();
                        arms.visit_block(function.body);
                        let selected: Vec<TokenStream> = if branch {
                            let mut branches = Branches {
                                literal,
                                selected: vec![],
                            };
                            branches.visit_block(function.body);
                            branches.selected
                        } else {
                            arms.arms
                                .into_iter()
                                .filter(|arm| literal.matches(&arm.pat))
                                .map(ToTokens::to_token_stream)
                                .collect()
                        };
                        if selected.is_empty() {
                            continue;
                        }
                        let tokens = selected.iter().map(|tokens| tokens.to_string()).collect();
                        let mut references = References::default();
                        for tokens in selected {
                            // Parse selections as expressions/arms to retain the ordinary
                            // one-hop same-file reference rule.
                            if let Ok(arm) = syn::parse2::<Arm>(tokens.clone()) {
                                references.visit_arm(&arm);
                            } else if let Ok(expr) = syn::parse2::<syn::Expr>(tokens) {
                                references.visit_expr(&expr);
                            }
                        }
                        (
                            format!("{}:{}", function.key, literal.label()),
                            tokens,
                            references.names,
                        )
                    } else {
                        (
                            function.key.clone(),
                            vec![function.definition.tokens.clone()],
                            function.definition.references.clone(),
                        )
                    };
                    let mut hash = Hash::new(if branch {
                        "rust:branch"
                    } else if literal.is_some() {
                        "rust:arm"
                    } else {
                        "rust:fn"
                    });
                    for tokens in tokens {
                        hash.field(&tokens);
                    }
                    let mut helpers: Vec<_> = references
                        .iter()
                        .filter_map(|name| helpers.get(name.as_str()))
                        .flatten()
                        .copied()
                        .collect();
                    helpers.sort_by_key(|definition| &definition.path);
                    for helper in helpers {
                        // Do not add the selected function a second time for recursion.
                        if literal.is_none() && helper.path == function.definition.path {
                            continue;
                        }
                        hash.field(&helper.path);
                        hash.field(&helper.tokens);
                    }
                    insert(&mut matches, key, Some(hash.finish()))?;
                }
                if matches.is_empty() && branch {
                    insert(
                        &mut results,
                        alias.unwrap_or(query).into(),
                        Some(Hash::new("rust:branch").finish()),
                    )?;
                } else if matches.is_empty() {
                    if !allow_missing {
                        return Err(Error::Missing(raw.into()));
                    }
                    insert(&mut results, alias.unwrap_or(query).into(), None)?;
                } else if let Some(alias) = alias {
                    let value = if matches.len() == 1 {
                        matches.into_values().next().expect("one matched item")
                    } else {
                        Some(combine(&matches))
                    };
                    insert(&mut results, alias.into(), value)?;
                } else {
                    for (key, value) in matches {
                        insert(&mut results, key, value)?;
                    }
                }
            }
            Ok(results)
        })
        .collect()
}

fn invalid(query: &str) -> Error {
    Error::Query {
        query: query.into(),
        reason: "expected fn:<glob>, arm:<fn-glob>:<literal> or branch:<fn-glob>:<literal>".into(),
    }
}

struct StripDocs;

impl Fold for StripDocs {
    fn fold_attributes(&mut self, attrs: Vec<syn::Attribute>) -> Vec<syn::Attribute> {
        attrs
            .into_iter()
            .filter(|attr| !attr.path().is_ident("doc"))
            .map(|attr| self.fold_attribute(attr))
            .collect()
    }

    fn fold_macro(&mut self, mut node: syn::Macro) -> syn::Macro {
        // syn deliberately leaves macro input opaque, including doc comments.
        node.tokens = strip_doc_tokens(node.tokens);
        node
    }
}

fn strip_doc_tokens(tokens: TokenStream) -> TokenStream {
    let tokens: Vec<_> = tokens.into_iter().collect();
    let mut result = TokenStream::new();
    let mut index = 0;
    while index < tokens.len() {
        if matches!(&tokens[index], TokenTree::Punct(punct) if punct.as_char() == '#') {
            let mut next = index + 1;
            if matches!(tokens.get(next), Some(TokenTree::Punct(punct)) if punct.as_char() == '!') {
                next += 1;
            }
            if let Some(TokenTree::Group(group)) = tokens.get(next) {
                if group.delimiter() == Delimiter::Bracket
                    && syn::parse2::<syn::Meta>(group.stream())
                        .is_ok_and(|meta| meta.path().is_ident("doc"))
                {
                    index = next + 1;
                    continue;
                }
            }
        }
        let token = match &tokens[index] {
            TokenTree::Group(group) => {
                let mut normalized =
                    Group::new(group.delimiter(), strip_doc_tokens(group.stream()));
                normalized.set_span(group.span());
                TokenTree::Group(normalized)
            }
            token => token.clone(),
        };
        result.extend([token]);
        index += 1;
    }
    result
}

struct Definition {
    name: String,
    path: String,
    tokens: String,
    references: BTreeSet<String>,
}

struct Function<'a> {
    definition: Definition,
    key: String,
    body: &'a Block,
}

#[derive(Default)]
struct Index<'a> {
    functions: Vec<Function<'a>>,
    other: Vec<Definition>,
    occurrences: BTreeMap<String, usize>,
}

impl<'a> Index<'a> {
    fn definition(&mut self, name: String, scope: &[String], tokens: &impl ToTokens) -> Definition {
        let mut path = scope.to_vec();
        path.push(name.clone());
        let path = path.join("::");
        // Repeated impls/cfg alternatives must never overwrite one another.
        let occurrence = self.occurrences.entry(path.clone()).or_default();
        *occurrence += 1;
        Definition {
            name,
            path: if *occurrence == 1 {
                path
            } else {
                format!("{path}#{occurrence}")
            },
            tokens: tokens.to_token_stream().to_string(),
            references: BTreeSet::new(),
        }
    }

    fn items(&mut self, items: &'a [Item], scope: &[String]) {
        for item in items {
            match item {
                Item::Fn(item) => {
                    let mut definition =
                        self.definition(item.sig.ident.unraw().to_string(), scope, item);
                    let mut references = References::default();
                    references.visit_item_fn(item);
                    definition.references = references.names;
                    LocalDefinitions {
                        index: self,
                        scope: vec![definition.path.clone()],
                    }
                    .visit_block(&item.block);
                    self.functions.push(Function {
                        definition,
                        key: String::new(),
                        body: &item.block,
                    });
                }
                Item::Const(item) => {
                    let definition = self.definition(item.ident.unraw().to_string(), scope, item);
                    LocalDefinitions {
                        index: self,
                        scope: vec![definition.path.clone()],
                    }
                    .visit_expr(&item.expr);
                    self.other.push(definition);
                }
                Item::Static(item) => {
                    let definition = self.definition(item.ident.unraw().to_string(), scope, item);
                    LocalDefinitions {
                        index: self,
                        scope: vec![definition.path.clone()],
                    }
                    .visit_expr(&item.expr);
                    self.other.push(definition);
                }
                Item::Mod(item) => {
                    if let Some((_, items)) = &item.content {
                        let mut nested = scope.to_vec();
                        nested.push(item.ident.unraw().to_string());
                        self.items(items, &nested);
                    }
                }
                Item::Impl(item) => {
                    let mut nested = scope.to_vec();
                    let ty = item.self_ty.to_token_stream().to_string();
                    nested.push(if let Some((negation, path, _)) = &item.trait_ {
                        format!(
                            "<{ty} as {}{}>",
                            negation.to_token_stream(),
                            path.to_token_stream()
                        )
                    } else {
                        ty
                    });
                    for member in &item.items {
                        match member {
                            ImplItem::Fn(method) => {
                                let mut definition = self.definition(
                                    method.sig.ident.unraw().to_string(),
                                    &nested,
                                    method,
                                );
                                let mut references = References::default();
                                references.visit_impl_item_fn(method);
                                definition.references = references.names;
                                LocalDefinitions {
                                    index: self,
                                    scope: vec![definition.path.clone()],
                                }
                                .visit_block(&method.block);
                                self.functions.push(Function {
                                    definition,
                                    key: String::new(),
                                    body: &method.block,
                                });
                            }
                            ImplItem::Const(constant) => {
                                let definition = self.definition(
                                    constant.ident.unraw().to_string(),
                                    &nested,
                                    constant,
                                );
                                LocalDefinitions {
                                    index: self,
                                    scope: vec![definition.path.clone()],
                                }
                                .visit_expr(&constant.expr);
                                self.other.push(definition);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn assign_keys(&mut self) {
        let mut counts = BTreeMap::new();
        for function in &self.functions {
            *counts
                .entry(function.definition.name.clone())
                .or_insert(0_usize) += 1;
        }
        for function in &mut self.functions {
            function.key = if counts[&function.definition.name] == 1 {
                function.definition.name.clone()
            } else if function.definition.path == function.definition.name {
                format!("crate::{}", function.definition.path)
            } else {
                function.definition.path.clone()
            };
        }
    }
}

// Block-local definitions are helpers too, but are not fn-query candidates.
struct LocalDefinitions<'index, 'ast> {
    index: &'index mut Index<'ast>,
    scope: Vec<String>,
}

impl<'ast> Visit<'ast> for LocalDefinitions<'_, 'ast> {
    fn visit_item(&mut self, item: &'ast Item) {
        let name = match item {
            Item::Fn(item) => Some(item.sig.ident.unraw().to_string()),
            Item::Const(item) => Some(item.ident.unraw().to_string()),
            Item::Static(item) => Some(item.ident.unraw().to_string()),
            _ => None,
        };
        let nested = match item {
            Item::Mod(item) => Some(item.ident.unraw().to_string()),
            Item::Impl(item) => Some(item.self_ty.to_token_stream().to_string()),
            _ => name.clone(),
        };
        let nested = if let Some(name) = name {
            let definition = self.index.definition(name, &self.scope, item);
            let path = definition.path.clone();
            self.index.other.push(definition);
            Some(path)
        } else {
            nested.map(|name| format!("{}::{name}", self.scope.join("::")))
        };
        if let Some(nested) = nested {
            let previous = std::mem::replace(&mut self.scope, vec![nested]);
            syn::visit::visit_item(self, item);
            self.scope = previous;
        } else {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast ImplItem) {
        let name = match item {
            ImplItem::Fn(item) => Some(item.sig.ident.unraw().to_string()),
            ImplItem::Const(item) => Some(item.ident.unraw().to_string()),
            _ => None,
        };
        if let Some(name) = name {
            let definition = self.index.definition(name, &self.scope, item);
            let previous = std::mem::replace(&mut self.scope, vec![definition.path.clone()]);
            self.index.other.push(definition);
            syn::visit::visit_impl_item(self, item);
            self.scope = previous;
        } else {
            syn::visit::visit_impl_item(self, item);
        }
    }
}

#[derive(Default)]
struct References {
    names: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for References {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path.leading_colon.is_none() && path.segments.len() == 1 {
            self.names
                .insert(path.segments[0].ident.unraw().to_string());
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if node.qself.is_some() {
            visit_qualified_path(self, node.qself.as_ref(), &node.path);
        } else {
            syn::visit::visit_expr_path(self, node);
        }
    }

    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        if node.qself.is_some() {
            visit_qualified_path(self, node.qself.as_ref(), &node.path);
        } else {
            syn::visit::visit_type_path(self, node);
        }
    }

    fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.macro_tokens(node.tokens.clone());
    }
}

impl References {
    /// Inspect syntax only, without expanding macros. Names are filtered through
    /// the existing same-file fn/const/static index, not resolved as dependencies.
    fn macro_tokens(&mut self, tokens: TokenStream) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        let punctuation = |i: usize, ch: char| matches!(tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == ch);
        for (i, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Group(group) => self.macro_tokens(group.stream()),
                TokenTree::Ident(name) => {
                    let qualified_before =
                        i >= 2 && punctuation(i - 1, ':') && punctuation(i - 2, ':');
                    let qualified_after = punctuation(i + 1, ':')
                        && punctuation(i + 2, ':')
                        && !punctuation(i + 3, '<');
                    let member_or_variable =
                        i > 0 && (punctuation(i - 1, '.') || punctuation(i - 1, '$'));
                    if !qualified_before
                        && !qualified_after
                        && !member_or_variable
                        && !punctuation(i + 1, '!')
                    {
                        self.names.insert(name.unraw().to_string());
                    }
                }
                _ => {}
            }
        }
    }
}

fn visit_qualified_path<'ast>(
    visitor: &mut References,
    qself: Option<&'ast syn::QSelf>,
    path: &'ast syn::Path,
) {
    if let Some(qself) = qself {
        visitor.visit_qself(qself);
    }
    for segment in &path.segments {
        visitor.visit_path_arguments(&segment.arguments);
    }
}

#[derive(Default)]
struct Arms<'a> {
    arms: Vec<&'a Arm>,
}

impl<'ast> Visit<'ast> for Arms<'ast> {
    fn visit_arm(&mut self, arm: &'ast Arm) {
        self.arms.push(arm);
        syn::visit::visit_arm(self, arm);
    }
}

#[derive(PartialEq)]
enum Literal {
    Integer(String),
    String(String),
}

impl Literal {
    fn parse(value: &str) -> Option<Self> {
        match syn::parse_str::<Lit>(value) {
            Ok(Lit::Int(value)) => Some(Self::integer(&value)),
            Ok(Lit::Str(value)) => Some(Self::String(value.value())),
            _ if value.chars().next().is_some_and(|first| {
                !first.is_ascii_digit() && !['"', '\'', '-'].contains(&first)
            }) =>
            {
                Some(Self::String(value.into()))
            }
            _ => None,
        }
    }

    fn integer(value: &syn::LitInt) -> Self {
        let digits = value.base10_digits();
        Self::Integer(if digits == "-0" { "0" } else { digits }.into())
    }

    fn label(&self) -> String {
        match self {
            Self::Integer(value) | Self::String(value) => value.clone(),
        }
    }

    fn matches(&self, pattern: &syn::Pat) -> bool {
        struct Find<'a> {
            target: &'a Literal,
            found: bool,
        }
        impl<'ast> Visit<'ast> for Find<'_> {
            fn visit_lit(&mut self, literal: &'ast Lit) {
                let value = match literal {
                    Lit::Int(value) => Some(Literal::integer(value)),
                    Lit::Str(value) => Some(Literal::String(value.value())),
                    _ => None,
                };
                self.found |= value.as_ref() == Some(self.target);
            }

            fn visit_attribute(&mut self, _: &'ast syn::Attribute) {}

            fn visit_macro(&mut self, _: &'ast syn::Macro) {}
        }
        let mut find = Find {
            target: self,
            found: false,
        };
        find.visit_pat(pattern);
        find.found
    }
}

/// Branch selection does not include sibling `else` bodies. Macro conditions
/// (e.g. matches!) stay opaque to syn but their literal tokens are inspected.
struct Branches<'a> {
    literal: &'a Literal,
    selected: Vec<TokenStream>,
}
impl<'ast> Visit<'ast> for Branches<'_> {
    fn visit_arm(&mut self, arm: &'ast Arm) {
        let condition = arm
            .guard
            .as_ref()
            .map(|(_, condition)| condition.to_token_stream());
        if self.literal.contains(arm.pat.to_token_stream())
            || condition.is_some_and(|c| self.literal.contains(c))
        {
            self.selected.push(arm.to_token_stream());
        }
        syn::visit::visit_arm(self, arm);
    }
    fn visit_expr_if(&mut self, expr: &'ast syn::ExprIf) {
        // A branch nested in the condition is smaller than the containing if.
        let before = self.selected.len();
        self.visit_expr(&expr.cond);
        if self.selected.len() == before && self.literal.contains(expr.cond.to_token_stream()) {
            let mut selected = expr.clone();
            selected.else_branch = None;
            self.selected.push(selected.to_token_stream());
        }
        self.visit_block(&expr.then_branch);
        if let Some((_, other)) = &expr.else_branch {
            self.visit_expr(other);
        }
    }
}
impl Literal {
    fn contains(&self, tokens: TokenStream) -> bool {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for (i, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Group(group) if self.contains(group.stream()) => return true,
                TokenTree::Literal(_) => {
                    let negative =
                        i > 0 && matches!(&tokens[i-1], TokenTree::Punct(p) if p.as_char() == '-');
                    let text = format!("{}{token}", if negative { "-" } else { "" });
                    if Self::parse(&text).as_ref() == Some(self) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
}
