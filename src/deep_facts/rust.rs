//! Rust: value forms, paths, and the macro and attribute policy.

use super::*;

impl<'a> Walker<'a> {
    // -- Rust ---------------------------------------------------------------
    //
    // Rust is read here rather than through the Python/JS arms above: its
    // declarations are items, its calls are `call_expression` with a
    // `field_expression` or `scoped_identifier` callee, and a `use` is a path,
    // not a binding list. Every function below is reached only when
    // `Kinds::rust` is set, so nothing here can change another language.

    /// The Rust-only nodes, in the order they are met. `false` means "not mine,
    /// let the shared walk read it" (an `expression_statement`, a block, a
    /// plain value).

    /// `const NAME: Type = value;`: the target is the name, the value is what
    /// follows the `=`.
    pub(crate) fn rust_const(&mut self, id: u32) {
        let children = self.children(id);
        let name = children
            .iter()
            .copied()
            .find(|&child| matches!(self.kind(child), "identifier" | "default" | "_" | "placeholder"))
            .map(|child| self.text_string(child))
            .unwrap_or_default();
        // `const _: () = assert!(...)`: the name is the placeholder, and the
        // lexer writes it as the event's target. Skipping it dropped the whole
        // statement, including every call inside the assertion.
        if name.is_empty() {
            return;
        }
        let Some(equal) = children.iter().position(|&child| self.kind(child) == "=") else {
            return;
        };
        let Some(&value) = children.get(equal + 1) else { return };
        // `const T: [KindInfo; N] = { let x = ..; .. }`: the initialiser is a
        // block, and a block's form is the block, not the names inside it. The
        // one-pass reader sees one flat value over the whole block, so the names
        // a block carries are read here too — the const's own event is the only
        // one on that line, and those names have no other place to be recorded.
        let value = if matches!(self.kind(value), "block_expression" | "block" | "compound_statement") {
            let mut parts = Vec::new();
            for child in self.children(value) {
                if self.is_layout(child) || matches!(self.kind(child), "{" | "}") {
                    continue;
                }
                if self.kind(child) == "let_declaration" {
                    let kids: Vec<u32> = self
                        .children(child)
                        .into_iter()
                        .filter(|&piece| !self.is_layout(piece) && self.text_string(piece) != "let")
                        .collect();
                    if let Some(&pattern) = kids.first() {
                        let pattern = if matches!(self.kind(pattern), "mutable_specifier" | "mut_pattern" | "ref_pattern") {
                            self.children(pattern).into_iter().find(|&inner| !self.is_layout(inner)).unwrap_or(pattern)
                        } else {
                            pattern
                        };
                        for name in self.assignment_targets(pattern) {
                            let mut atom = String::from("[\"n\",");
                            push_string(&mut atom, &name);
                            atom.push(']');
                            parts.push(atom);
                        }
                    }
                    if let Some(&init) = kids.last() {
                        if init != kids.first().copied().unwrap_or(0) {
                            let form = self.form(init);
                            if form != "0" {
                                parts.push(form);
                            }
                        }
                    }
                    continue;
                }
                // `table[i] = KindInfo { .. }` inside the block: a statement
                // again, so its target and its initialiser are what carry names.
                if self.kind(child) == "expression_statement" {
                    if let Some(inner) = self.expression_child(child) {
                        if self.kind(inner) == "assignment_expression" {
                            let kids = self.children(inner);
                            if let Some(&target) = kids.first() {
                                for name in self.assignment_targets(target) {
                                    let mut atom = String::from("[\"n\",");
                                    push_string(&mut atom, &name);
                                    atom.push(']');
                                    parts.push(atom);
                                }
                            }
                            if let Some(&value_node) = kids.last() {
                                let form = self.form(value_node);
                                if form != "0" {
                                    parts.push(form);
                                }
                            }
                            continue;
                        }
                    }
                }
                let form = self.form(child);
                if form != "0" {
                    parts.push(form);
                }
            }
            if parts.is_empty() {
                "0".to_string()
            } else {
                format!("[\"m\",[{}]]", parts.join(","))
            }
        } else {
            self.form(value)
        };
        let line = self.line(id);
        // The target list is a plain array of names, as the lexer writes it.
        let mut targets = "[\"".to_string();
        targets.push_str(&name);
        targets.push_str("\"]");
        self.event(&format!("[\"a\",{line},{targets},{value},0]"));
    }


    /// `{ free: [u8; N], bump: u8 }`: one event for the field list, the way the
    /// lexer reads the body.
    pub(crate) fn rust_field_list(&mut self, id: u32) {
        let mut parts = Vec::new();
        for child in self.children(id) {
            if self.is_layout(child) || !matches!(self.kind(child), "field_declaration" | "field_declaration_list") {
                continue;
            }
            // The field's own children are the name and the type: read them
            // one by one, the way the lexer reads the field as an expression.
            let mut field = Vec::new();
            for part in self.children(child) {
                if self.is_layout(part) {
                    continue;
                }
                let value = self.form(part);
                if value != "0" {
                    field.push(value);
                }
            }
            if !field.is_empty() {
                parts.push(format!("[\"m\",[{}]]", field.join(",")));
            }
        }
        if parts.is_empty() {
            return;
        }
        // The lexer stamps the statement on the first field's line.
        let line = self
            .children(id)
            .into_iter()
            .find(|&child| self.kind(child) == "field_declaration")
            .map(|child| self.line(child))
            .unwrap_or_else(|| self.line(id));
        self.event(&format!("[\"x\",{line},[\"t\",[{}]]]", parts.join(",")));
    }

    pub(crate) fn enter_rust(&mut self, id: u32) -> bool {
        match self.kind(id) {
            "impl_item" => {
                self.rust_impl(id);
                true
            }
            "use_declaration" => {
                self.rust_import(id);
                true
            }
            "let_declaration" => {
                self.rust_let(id);
                true
            }
            // `const X: usize = 4;` and `static Y: &str = "s";`: the lexer
            // reads a module-level binding and files an assignment event; the
            // item was never read here, so every module constant was lost.
            "const_item" | "static_item" => {
                self.rust_const(id);
                true
            }

            // A struct's fields are declarations, but the one-pass lexer reads
            // the body as an expression statement and files one event for it.
            // The names are real either way, and without an event the file
            // looks like the CST lost them.
            "field_declaration_list" => {
                self.rust_field_list(id);
                true
            }
            "return_expression" => {
                let line = self.line(id);
                let value = match self.expression_child(id) {
                    Some(child) => self.form(child),
                    None => "0".to_string(),
                };
                self.event(&format!("[\"r\",{line},{value}]"));
                true
            }
            "if_expression" | "while_expression" => {
                self.rust_condition(id);
                self.event("[\"b\"]");
                true
            }
            "loop_expression" => {
                self.event("[\"b\"]");
                true
            }
            "for_expression" => {
                self.rust_for(id);
                true
            }
            "match_expression" => {
                // `match c.peek() {`: the scrutinee is a value the lexer reads
                // as an expression statement before the block opens.
                self.rust_condition(id);
                self.event("[\"b\"]");
                true
            }
            "match_arm" => {
                // `pat => expr,`: the arm's value is an expression of its own,
                // and where the calls written after `=>` live. The value is a
                // statement of the match, so it is recorded as one: without
                // this event an arm's value is invisible to the record.
                let line = self.line(id);
                if let Some(value) = self.rust_after_arrow(id) {
                    // `"--out" => options.out = Some(..)`: an arm that assigns is
                    // an assignment, and the lexer files it with its target. An
                    // arm recorded as a bare expression has no binder, so the
                    // guard sees the target vanish.
                    match self.kind(value) {
                        "assignment_expression" => self.assignment(value, false),
                        "compound_assignment_expr" => self.assignment(value, true),
                        _ => {
                            let form = self.form(value);
                            if form != "0" {
                                self.event(&format!("[\"x\",{line},{form}]"));
                            }
                        }
                    }
                }
                self.event("[\"e\"]");
                false
            }
            _ => false,
        }
    }

    /// The control-flow markers a Rust expression closes with. `if`/`while`/
    /// `loop`/`for`/`match` are expressions here, so they are not the shared
    /// `_statement` kinds the other arms close.
    pub(crate) fn leave_rust(&mut self, id: u32) {
        if matches!(self.kind(id), "if_expression" | "while_expression" | "loop_expression" | "for_expression" | "match_expression") {
            self.event("[\"j\"]");
        }
    }

    /// `impl Reader { ... }`, `impl Draw for Widget { ... }`: a container named
    /// after the type it implements, so its methods take that type as owner and
    /// their calls sit on them. A trait impl records the trait as a base.
    pub(crate) fn rust_impl(&mut self, id: u32) {
        let line = self.line(id);
        let mut inherent = String::new();
        let mut trait_name = String::new();
        let mut seen_for = false;
        for child in self.children(id) {
            match self.kind(child) {
                "impl" | "type_parameters" | "where_clause" | "declaration_list" => {}
                "for" => seen_for = true,
                _ => {
                    let name = self.rust_type_name(child);
                    if name.is_empty() {
                        continue;
                    }
                    if seen_for && inherent.is_empty() {
                        inherent = name;
                    } else if !seen_for && trait_name.is_empty() {
                        trait_name = name;
                    }
                }
            }
        }
        let (name, base) = if inherent.is_empty() { (trait_name, String::new()) } else { (inherent, trait_name) };
        if name.is_empty() {
            return;
        }
        let index = self.add_symbol(name.clone(), String::new(), KIND_CLASS, line, Vec::new());
        if !base.is_empty() {
            self.facts.symbols[index].bases.push(base);
        }
        let symbol = self.frame().symbol;
        self.stack.push(Frame { node: id, symbol, class: name, receiver: String::new(), container: true, defines: Some(index) });
    }

    /// The spelled name of a type node: `Reader`, `Reader<'a>`, `fmt::Display`.
    pub(crate) fn rust_type_name(&self, id: u32) -> String {
        match self.kind(id) {
            "type_identifier" | "identifier" => self.text_string(id),
            "generic_type" | "reference_type" | "scoped_type_identifier" | "tuple_type" => {
                self.last_name(id).unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    /// `let mut value = f(x);` / `let x: T;`. A binding is an assignment event
    /// when it carries a value and a name event when it does not, as the lexer
    /// writes it. The pattern is the child after `let`; the value is the child
    /// after `=`.
    pub(crate) fn rust_let(&mut self, id: u32) {
        let children = self.children(id);
        let Some(&pattern) = children.iter().find(|&&child| !matches!(self.kind(child), "let" | ";" | ":")) else {
            return;
        };
        let line = self.line(id);
        let targets = self.rust_pattern_names(pattern);
        match children.iter().position(|&child| self.kind(child) == "=") {
            Some(equal) if children.len() > equal + 1 => {
                let value = children[equal + 1];
                self.assignment_parts(targets, value, line, false);
            }
            _ => {
                if targets.len() == 1 {
                    let mut text = format!("[\"x\",{line},[\"n\",");
                    push_string(&mut text, &targets[0]);
                    text.push_str("]]");
                    self.event(&text);
                }
            }
        }
    }

    /// The value side of a `match` arm: the child after `=>`.
    pub(crate) fn rust_after_arrow(&self, id: u32) -> Option<u32> {
        let children = self.children(id);
        let at = children.iter().position(|&child| self.kind(child) == "=>")?;
        children.get(at + 1).copied()
    }

    /// The value side of a `let_condition` (`if let Ok(x) = f()`): the child
    /// after `=`. The pattern is a binding, not a value.
    pub(crate) fn rust_let_value(&self, id: u32) -> Option<u32> {
        let children = self.children(id);
        let at = children.iter().position(|&child| self.kind(child) == "=")?;
        children.get(at + 1).copied()
    }

    /// The names a Rust pattern binds. A tuple-struct pattern's first child is
    /// the variant (`Ok(name)` binds `name`, not `Ok`) and a struct pattern's
    /// field names are its own, so neither is read as a binder.
    pub(crate) fn rust_pattern_names(&self, id: u32) -> Vec<String> {
        match self.kind(id) {
            "identifier" | "shorthand_field_identifier" | "self" | "_" => vec![self.text_string(id)],
            "mut_pattern" | "ref_pattern" | "reference_pattern" => self.first_identifier(id).into_iter().collect(),
            "tuple_struct_pattern" => {
                self.children(id).into_iter().skip(1).flat_map(|child| self.rust_pattern_names(child)).collect()
            }
            "struct_pattern" => self
                .children(id)
                .into_iter()
                .filter(|&child| matches!(self.kind(child), "field_pattern" | "shorthand_field_identifier" | "remaining_field_pattern"))
                .flat_map(|child| self.rust_pattern_names(child))
                .collect(),
            // `Struct { field: binder }` binds `binder`; `Struct { field }`
            // binds the field's own name.
            "field_pattern" => self
                .children(id)
                .into_iter()
                .rev()
                .find_map(|child| self.first_identifier(child))
                .into_iter()
                .collect(),
            "tuple_pattern" | "or_pattern" | "slice_pattern" | "tuple_expression" => {
                self.children(id).into_iter().flat_map(|child| self.rust_pattern_names(child)).collect()
            }
            _ => self.pattern_names(id),
        }
    }

    /// `if cond {`, `while cond {`: the condition is a value of its own and its
    /// calls are facts. `if let pat = value` is read on the value side, which is
    /// where the calls are.
    pub(crate) fn rust_condition(&mut self, id: u32) {
        for child in self.children(id) {
            match self.kind(child) {
                "if" | "while" => continue,
                "block" | "else_clause" => return,
                "let_condition" | "let_chain" => {
                    if let Some(value) = self.rust_let_value(child) {
                        let value = self.form(value);
                        if value != "0" {
                            let line = self.line(child);
                            self.event(&format!("[\"x\",{line},{value}]"));
                        }
                    }
                    return;
                }
                _ => {
                    if self.is_layout(child) {
                        continue;
                    }
                    let value = self.form(child);
                    if value != "0" {
                        let line = self.line(child);
                        self.event(&format!("[\"x\",{line},{value}]"));
                    }
                    return;
                }
            }
        }
    }

    /// `for value in values {`: the loop binds its pattern to the iterable, so
    /// the record gets an assignment event and a block, as the lexer writes.
    pub(crate) fn rust_for(&mut self, id: u32) {
        let children = self.children(id);
        let Some(at) = children.iter().position(|&child| self.kind(child) == "in") else {
            self.event("[\"b\"]");
            return;
        };
        self.event("[\"b\"]");
        let targets = children[..at]
            .iter()
            .filter(|&&child| !matches!(self.kind(child), "for" | "&" | "ref" | "mut"))
            .flat_map(|&child| self.rust_pattern_names(child))
            .collect();
        if let Some(&value) = children.get(at + 1) {
            let line = self.line(id);
            self.assignment_parts(targets, value, line, false);
        }
    }

    /// `use std::path::PathBuf;`, `use x as y;`, `use a::b::*;`,
    /// `use a::{b, c as d};` — the path import the lexer spells with dots.
    pub(crate) fn rust_import(&mut self, id: u32) {
        let line = self.line(id);
        for child in self.children(id) {
            match self.kind(child) {
                "use" | ";" => {}
                "use_as_clause" => {
                    let inner = self.children(child);
                    let Some(&path) = inner.first() else { continue };
                    let module = self.rust_module_text(path);
                    let alias = inner.last().filter(|&&last| last != path).map(|&last| self.text_string(last));
                    self.facts.imports.push(Import {
                        line,
                        module,
                        alias: alias.unwrap_or_default(),
                        kind: IMPORT_MODULE,
                        ..Import::default()
                    });
                }
                "use_wildcard" => {
                    let Some(&path) = self.children(child).first() else { continue };
                    self.facts.imports.push(Import {
                        line,
                        module: self.rust_module_text(path),
                        kind: IMPORT_STAR,
                        ..Import::default()
                    });
                }
                "scoped_use_list" => {
                    let inner = self.children(child);
                    let Some(&path) = inner.first() else { continue };
                    let module = self.rust_module_text(path);
                    let Some(&list) = inner.iter().find(|&&node| self.kind(node) == "use_list") else { continue };
                    self.rust_use_list(list, &module, line);
                }
                "use_list" => self.rust_use_list(child, "", line),
                _ => {
                    let module = self.rust_module_text(child);
                    if module.is_empty() {
                        continue;
                    }
                    let alias = module.rsplit('.').next().unwrap_or("").to_string();
                    self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
                }
            }
        }
    }

    pub(crate) fn rust_use_list(&mut self, id: u32, module: &str, line: u32) {
        for child in self.children(id) {
            match self.kind(child) {
                "use_as_clause" => {
                    let inner = self.children(child);
                    let Some(&path) = inner.first() else { continue };
                    let name = self.rust_module_text(path);
                    let alias = inner.last().filter(|&&last| last != path).map(|&last| self.text_string(last));
                    self.facts.imports.push(Import {
                        line,
                        module: module.into(),
                        name,
                        alias: alias.unwrap_or_default(),
                        kind: IMPORT_MODULE,
                    });
                }
                "identifier" | "scoped_identifier" | "type_identifier" | "self" => {
                    let name = self.rust_module_text(child);
                    let (module_name, member) = match module.rsplit_once('.') {
                        _ if module.is_empty() => (String::new(), name.clone()),
                        _ => (module.to_string(), name.clone()),
                    };
                    self.facts.imports.push(Import {
                        line,
                        module: module_name,
                        name: member.clone(),
                        alias: member,
                        kind: IMPORT_MODULE,
                    });
                }
                _ => {}
            }
        }
    }

    /// A path as the lexer spells it: `std::path::PathBuf` is `std.path.PathBuf`,
    /// `crate` and `super` keep their word.
    pub(crate) fn rust_module_text(&self, id: u32) -> String {
        match self.kind(id) {
            "use_wildcard" => self.children(id).first().map(|&child| self.rust_module_text(child)).unwrap_or_default(),
            _ => self.text_string(id).replace("::", "."),
        }
    }

    /// Rust's value forms, or `None` for a node the shared arms already read.
    pub(crate) fn rust_form(&mut self, id: u32) -> Option<String> {
        match self.kind(id) {
            "call_expression" => Some(self.rust_call(id)),
            "macro_invocation" => Some(self.rust_macro(id)),
            // `Facts { tag, lines }`: the constructor and its fields.
            "struct_expression" => Some(self.rust_struct_expression(id)),
            "field_initializer_list" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                Some(if items.is_empty() { "0".to_string() } else { self.merge_form(&items) })
            }
            "field_initializer" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.as_slice() {
                    [name, value] => {
                        let key = self.form(*name);
                        let value = self.form(*value);
                        Some(format!("[\"m\",[{key},{value}]]"))
                    }
                    _ => Some("0".to_string()),
                }
            }
            // `Language { a: 1, ..BASE }`: the spread names a value the lexer
            // records, and without a form here the struct literal lost it.
            "base_field_initializer" => {
                let kids: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && self.kind(child) != "..")
                    .collect();
                kids.into_iter().find_map(|child| {
                    let form = self.form(child);
                    (form != "0").then_some(form)
                })
            }
            "shorthand_field_initializer" => {
                let name = self.children(id).into_iter().next().map(|child| self.form(child)).unwrap_or_else(|| "0".to_string());
                Some(name)
            }
            "type_cast_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !matches!(self.kind(child), "as")).collect();
                Some(if items.is_empty() { "0".to_string() } else { self.merge_form(&items) })
            }
            // `&x`, `*x`, `x?`, `x.await`: the operator is not a value, the
            // operand is, and it is where the calls are.
            "reference_expression" | "try_expression" | "await_expression" => {
                self.expression_child(id).map(|child| self.form(child))
            }
            "range_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                Some(if items.is_empty() { "0".to_string() } else { self.merge_form(&items) })
            }
            "index_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.split_first() {
                    Some((base, rest)) => {
                        let base = self.form(*base);
                        if rest.is_empty() {
                            Some(base)
                        } else {
                            let mut all = vec![base];
                            all.extend(rest.iter().map(|&item| self.form(item)));
                            Some(format!("[\"m\",[{}]]", all.join(",")))
                        }
                    }
                    None => Some("0".to_string()),
                }
            }
            "tuple_expression" | "array_expression" | "tuple_type" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                Some(if items.is_empty() { "0".to_string() } else { self.tuple_form(&items) })
            }
            // `|x| self.string()`: the parameters and body form the closure value.
            "closure_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child) && self.kind(child) != "move" && self.kind(child) != "async").collect();
                let mut parts = Vec::new();
                for item in items {
                    if self.kind(item) == "closure_parameters" {
                        let params: Vec<u32> = self.children(item).into_iter().filter(|&child| !self.is_layout(child)).collect();
                        for param in params {
                            let value = self.form(param);
                            if value != "0" {
                                parts.push(value);
                            }
                        }
                    } else if !matches!(self.kind(item), "block") {
                        let value = self.form(item);
                        if value != "0" {
                            parts.push(value);
                        }
                    }
                }
                Some(match parts.as_slice() {
                    [] => "0".to_string(),
                    [one] => one.clone(),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                })
            }
            // `value >>= 7` / `bytes += src.len()`: the lexer reads the target
            // as the value, but the right side is still evaluated and its calls
            // are facts — reading only the target dropped `src.len` here.
            "compound_assignment_expr" => {
                let children = self.children(id);
                if let Some(&value) = children.last() {
                    self.form(value);
                }
                Some(children.first().map(|&first| self.form(first)).unwrap_or_else(|| "0".to_string()))
            }
            "break_expression" | "continue_expression" => {
                let word = if self.kind(id) == "break_expression" { "break" } else { "continue" };
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                if items.is_empty() {
                    Some(format!("[\"n\",\"{word}\"]"))
                } else {
                    let mut parts = vec![format!("[\"n\",\"{word}\"]")];
                    for item in items {
                        let value = self.form(item);
                        if value != "0" {
                            parts.push(value);
                        }
                    }
                    Some(if parts.len() == 1 { parts.pop().unwrap() } else { format!("[\"m\",[{}]]", parts.join(",")) })
                }
            }
            // A token tree is the macro's argument list; its names and nested
            // calls are values, its punctuation is not.
            "token_tree" | "token_tree_bracket" | "token_tree_brace" => Some(self.rust_tt_value(id)),
            "if_expression" | "else_clause" | "block" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                Some(match items.as_slice() {
                    [] => "0".to_string(),
                    [one] => self.form(*one),
                    _ => self.merge_form(&items),
                })
            }
            "match_expression" => {
                let mut parts = vec!["[\"n\",\"match\"]".to_string()];
                for child in self.children(id) {
                    if self.kind(child) == "match" {
                        continue;
                    }
                    let value = self.form(child);
                    if value != "0" {
                        parts.push(value);
                    }
                }
                Some(if parts.len() == 1 { parts.pop().unwrap() } else { format!("[\"m\",[{}]]", parts.join(",")) })
            }
            "match_block" | "match_arm" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child) && self.kind(child) != "=>").collect();
                Some(match items.as_slice() {
                    [] => "0".to_string(),
                    [one] => self.form(*one),
                    _ => self.merge_form(&items),
                })
            }
            "expression_statement" => {
                Some(self.expression_child(id).map(|child| self.form(child)).unwrap_or_else(|| "0".to_string()))
            }
            _ => None,
        }
    }

    pub(crate) fn rust_call(&mut self, id: u32) -> String {
        let children = self.children(id);
        let callee = children
            .iter()
            .copied()
            .find(|&child| !matches!(self.kind(child), "arguments" | "type_arguments" | "(" | ")"))
            .unwrap_or(id);
        let (path, receiver) = self.rust_callee(callee);
        let line = self.rust_call_line(callee, id);
        let index = self.push_call(path, line);
        let mut out = format!("[\"c\",{index},{receiver},");
        match children.iter().copied().find(|&child| self.kind(child) == "arguments") {
            Some(list) => out.push_str(&self.arguments(list)),
            None => out.push_str("[]"),
        }
        out.push(']');
        out
    }

    /// The called path and the receiver form: `render` -> (`render`, `0`),
    /// `self.bytes.get` -> (`self.bytes.get`, `["n","self.bytes"]`),
    /// `u32::from` -> (`u32.from`, `0`), `make().run` -> (`.run`, the call's
    /// form).
    pub(crate) fn rust_callee(&mut self, callee: u32) -> (String, String) {
        match self.kind(callee) {
            "field_expression" => {
                let children = self.children(callee);
                let name = self.member_name(callee);
                let Some(&object) = children.first() else {
                    return (format!(".{name}"), "0".to_string());
                };
                if matches!(self.kind(object), "call_expression" | "macro_invocation") {
                    let inner = self.form(object);
                    return (format!(".{name}"), inner);
                }
                // `w[i - 15].rotate_right(7)`: the receiver is an index into a
                // named value, and the lexer names the call by that value. Read
                // as a bare index the call has no name at all and the two
                // readers could not agree on it.
                let mut receiver = self.path_of(object);
                if receiver.is_empty() && self.kind(object) == "index_expression" {
                    receiver = self
                        .children(object)
                        .into_iter()
                        .find(|&child| !self.is_layout(child))
                        .map(|child| self.path_of(child))
                        .unwrap_or_default();
                }
                // `(value ^ u64::from(b)).wrapping_mul(k)`: the receiver is an
                // expression, and the lexer writes its value there. Reading the
                // call for its facts and then writing 0 as the receiver lost
                // every name in the parenthesised receiver.
                let formed = self.form(object);
                if receiver.is_empty() {
                    (format!(".{name}"), formed)
                } else if receiver.starts_with('.') {
                    (format!("{receiver}.{name}"), formed)
                } else {
                    let mut text = "[\"n\",".to_string();
                    push_string(&mut text, &receiver);
                    text.push(']');
                    (format!("{receiver}.{name}"), text)
                }
            }
            // `String::from(x)`: the qualifier is the receiver, as the lexer
            // writes it, so a reader can tell `String::from` from `Vec::from`.
            "scoped_identifier" | "scoped_type_identifier" => {
                let path = self.path_of(callee);
                match path.rsplit_once('.') {
                    Some((head, _)) if !head.is_empty() => {
                        let mut text = "[\"n\",".to_string();
                        push_string(&mut text, head);
                        text.push(']');
                        (path, text)
                    }
                    _ => (path, "0".to_string()),
                }
            }
            "generic_function" => {
                let inner = self.children(callee).into_iter().find(|&child| self.kind(child) != "type_arguments");
                match inner {
                    Some(inner) => self.rust_callee(inner),
                    None => (String::new(), "0".to_string()),
                }
            }
            "tuple_expression" | "parenthesized_expression" => {
                let items: Vec<u32> = self.children(callee).into_iter().filter(|&child| !self.is_layout(child)).collect();
                if items.len() == 1 {
                    let formed = self.form(items[0]);
                    (".call".to_string(), formed)
                } else {
                    (String::new(), "0".to_string())
                }
            }
            _ => (self.path_of(callee), "0".to_string()),
        }
    }

    /// The line a Rust call is written on: for `a.iter()\n  .map(f)` that is
    /// the line of `map`, which is what a reader points at and where the lexer
    /// stamps it.
    pub(crate) fn rust_call_line(&self, callee: u32, call: u32) -> u32 {
        match self.kind(callee) {
            "field_expression" => {
                let children = self.children(callee);
                if let Some(&name) = children.iter().rev().find(|&&child| {
                    self.node(child).first_child == NONE && !self.is_layout(child)
                }) {
                    return self.line(name);
                }
                self.line(call)
            }
            _ => self.line(callee),
        }
    }

    /// `assert_eq!(a, b)` is a merge of the macro's name and its arguments, not
    /// a call: the lexer reads the name as a name, and a resolver has no callee
    /// called `assert_eq!`.

    /// A macro's token tree, read the way the one-pass lexer reads it: a name
    /// followed by a tree is a call (`assert_eq!(…, shown())` calls `shown`),
    /// everything else is a value. The tokens are not code to the parser, but
    /// they are arguments to a macro that will evaluate them, so the calls in
    /// them are facts.
    pub(crate) fn rust_tt_path(&self, kids: &[u32], at: &mut usize) -> String {
        let mut path = String::new();
        while *at < kids.len() {
            let node = kids[*at];
            let kind = self.kind(node);
            let text = self.text_string(node);
            // The weak keywords are names wherever the grammar's `_name` and
            // `_macro_path` rules accept one, and a token tree is exactly where
            // nothing re-parses them: `matches!(self.raw(&t), …)` lexes `raw`
            // as its keyword, so a path that stopped at `identifier` lost the
            // call the macro will evaluate.
            if matches!(
                kind,
                "identifier"
                    | "scoped_identifier"
                    | "type_identifier"
                    | "self"
                    | "this"
                    | "super"
                    | "crate"
                    | "Self"
                    | "union"
                    | "default"
                    | "raw"
                    | "auto"
                    | "safe"
                    | "macro_rules"
            ) {
                if !path.is_empty() && !path.ends_with('.') {
                    break;
                }
                path.push_str(&text);
                *at += 1;
            } else if text == "." || text == "::" {
                if path.is_empty() || path.ends_with('.') {
                    break;
                }
                path.push('.');
                *at += 1;
            } else {
                break;
            }
        }
        if path.ends_with('.') {
            path.pop();
            *at -= 1;
        }
        path
    }

    /// Macro invocations and calls inside a macro's argument list: the calls in
    /// them are facts.
    pub(crate) fn rust_tt_value(&mut self, id: u32) -> String {
        let kids = self.children(id);
        let mut parts = Vec::new();
        let mut at = 0;
        while at < kids.len() {
            let start = at;
            let path = self.rust_tt_path(&kids, &mut at);
            // `size_of::<Node>()`: the turbofish sits between the path and the
            // argument list, so without stepping over it the walker never
            // reached the parentheses and the call was recorded as a name.
            if !path.is_empty() && at + 1 < kids.len()
                && self.text_string(kids[at]) == "::"
                && self.text_string(kids[at + 1]) == "<"
            {
                let mut depth = 0i32;
                while at < kids.len() {
                    let text = self.text_string(kids[at]);
                    if text == "<" {
                        depth += 1;
                    } else if text == ">" {
                        depth -= 1;
                        if depth == 0 {
                            at += 1;
                            break;
                        }
                    }
                    at += 1;
                }
            }
            if !path.is_empty() {
                if at + 1 < kids.len() && self.kind(kids[at]) == "!" && matches!(self.kind(kids[at + 1]), "token_tree" | "token_tree_bracket" | "token_tree_brace") {
                    let args = self.rust_tt_value(kids[at + 1]);
                    parts.push(format!("[\"m\",[[\"n\",\"{path}\"],{args}]]"));
                    at += 2;
                    continue;
                }
                if at < kids.len() && self.kind(kids[at]) == "token_tree" {
                    let line = self.line(kids[at]);
                    let index = self.push_call(path.clone(), line);
                    let receiver = match path.rsplit_once('.') {
                        Some((head, _)) if !head.is_empty() => format!("[\"n\",\"{head}\"]"),
                        _ => "0".to_string(),
                    };
                    let args = self.rust_tt_value(kids[at]);
                    parts.push(format!("[\"c\",{index},{receiver},[{args}]]"));
                    at += 1;
                    continue;
                }
                parts.push(format!("[\"n\",\"{path}\"]"));
                continue;
            }
            let node = kids[start];
            if !self.is_layout(node) {
                parts.push(self.form(node));
            }
            at = start + 1;
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            _ => {
                let tag = if self.kind(id) == "token_tree_bracket" { "t" } else { "m" };
                format!("[\"{tag}\",[{}]]", parts.join(","))
            }
        }
    }

    pub(crate) fn rust_macro(&mut self, id: u32) -> String {
        let mut items = Vec::new();
        for child in self.children(id) {
            match self.kind(child) {
                "token_tree" | "token_tree_bracket" | "token_tree_brace" => items.push(self.form(child)),
                "identifier" | "scoped_identifier" => {
                    let path = self.path_of(child);
                    let mut text = "[\"n\",".to_string();
                    push_string(&mut text, &path);
                    text.push(']');
                    items.push(text);
                }
                _ => {}
            }
        }
        if items.is_empty() {
            "0".to_string()
        } else {
            format!("[\"m\",[{}]]", items.join(","))
        }
    }


    pub(crate) fn rust_struct_expression(&mut self, id: u32) -> String {
        let mut items = Vec::new();
        let children = self.children(id);
        if let Some(&name) = children.first() {
            let path = self.path_of(name);
            if !path.is_empty() {
                let mut text = "[\"n\",".to_string();
                push_string(&mut text, &path);
                text.push(']');
                items.push(text);
            }
        }
        for child in children.iter().skip(1) {
            if self.kind(*child) == "field_initializer_list" {
                items.push(self.form(*child));
            }
        }
        if items.is_empty() {
            "0".to_string()
        } else {
            format!("[\"m\",[{}]]", items.join(","))
        }
    }

    /// Rust's path spellings, or `None` when the shared `path_of` arm is right
    /// (a plain identifier, a literal).
    pub(crate) fn rust_path(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "field_expression" => {
                let name = self.member_name(id);
                let object = self.children(id).first().copied()?;
                let base = self.path_of(object);
                Some(if base.is_empty() { format!(".{name}") } else { format!("{base}.{name}") })
            }
            // `u32::from` is `u32.from`, the lexer's spelling of a path.
            "scoped_identifier" | "scoped_type_identifier" => Some(self.text_string(id).replace("::", ".")),
            "call_expression" | "macro_invocation" => Some(String::new()),
            "struct_expression" => self.children(id).first().map(|&name| self.path_of(name)),
            "generic_function" => self.children(id).into_iter().find(|&child| self.kind(child) != "type_arguments").map(|child| self.path_of(child)),
            _ => None,
        }
    }
}
