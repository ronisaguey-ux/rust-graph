//! Java: the statement model the shared Go/Java arms do not cover.

use super::*;

impl<'a> Walker<'a> {
    pub(crate) fn java_enter(&mut self, id: u32) {
        match self.kind(id) {
            // `package com.example.web;`, `open module a.b {`: the lexer reads
            // the dotted name as one name, so the value is one name too.
            "package_declaration" | "module_declaration" | "requires_module_directive"
            | "exports_module_directive" | "opens_module_directive" | "uses_module_directive"
            | "provides_module_directive" => self.java_dotted_declaration(id),
            "block" | "class_body" | "enum_body" | "enum_body_declarations" | "interface_body" | "constructor_body" | "module_body" => {
                self.java_statements(id)
            }
            "if_statement" | "for_statement" | "enhanced_for_statement" | "while_statement" | "do_statement"
            | "switch_expression" | "switch_statement" => self.java_header(id),
            "switch_block_statement_group" | "switch_rule" => self.java_case(id),
            // `@RestController public class Controller {`: with an annotation in
            // front of it the lexer does not see a class at all and reads the
            // header as an expression, so the header is an `x` event here too.
            "class_declaration" | "interface_declaration" | "enum_declaration" | "record_declaration"
            | "annotation_type_declaration" => self.java_header_event(id),
            _ => {}
        }
    }

    pub(crate) fn java_header_event(&mut self, id: u32) {
        let mut parts = Vec::new();
        for child in self.children(id) {
            if matches!(
                self.kind(child),
                "class_body" | "enum_body" | "interface_body" | "annotation_type_body" | "record_body" | "constructor_body" | "type_body"
                    // A C# property's body is statements of its own.
                    | "property_body" | "property_tail"
            ) {
                continue;
            }
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        let value = match parts.len() {
            0 => String::new(),
            1 => parts.pop().unwrap_or_default(),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        };
        let mut line = self.line(id);
        // Swift: `@frozen` on its own line is an attribute call of its own, so the
        // header starts at the keyword after the modifiers, as the lexer's does.
        if self.language.name == "swift" {
            let first = self.children(id).into_iter().find(|&child| !matches!(self.kind(child), "modifiers" | "declaration_modifiers"));
            if let Some(first) = first {
                line = self.line(first);
            }
        }
        if !value.is_empty() {
            self.lexer_event(b'x', line, &[], &value, false);
        }
        // `@NullUnmarked enum QuantilesAlgorithm { SORTING { … } }`: with an
        // annotation in front of it the one-pass reader does not see the type
        // declaration at all — `class_head` skips Rust attributes and Scala
        // annotations but not a Java one — so it opens a plain block and reads
        // the constant list as statements. Each run of body-less constants
        // became one `x` (a tuple when a comma splits it) and each constant with
        // a body became an `x` of its own name. Nothing read them here, so every
        // annotated enum in the corpus lost its constants (java: 11 rows in
        // guava's QuantilesAlgorithm/CollectorTester, jackson's
        // EnumNaming{Ser,Des}erializationTest).
        if self.language.name == "java" && self.kind(id) == "enum_declaration" {
            self.java_enum_constants(id);
        }
    }

    /// The enum constants of an enum whose declaration the one-pass reader did
    /// not recognise: one `x` per run of body-less constants (the line of the
    /// first), and one `x` per constant that carries a class body. An
    /// unannotated enum is left alone: the plain reader opens a class frame for
    /// it, and a member of a class body binds nothing.
    pub(crate) fn java_enum_constants(&mut self, id: u32) {
        let annotated = self.child_of_kind(id, "modifiers").is_some_and(|modifiers| {
            self.children(modifiers)
                .into_iter()
                .any(|mark| matches!(self.kind(mark), "annotation" | "marker_annotation"))
        });
        if !annotated {
            return;
        }
        let Some(body) = self.child_of_kind(id, "enum_body") else {
            return;
        };
        let mut run: Vec<u32> = Vec::new();
        for child in self.children(body) {
            match self.kind(child) {
                "enum_constant" => {
                    if self.child_of_kind(child, "class_body").is_some() {
                        self.java_enum_constant_run(&mut run);
                        // `SORTING { … }` names the constant and nothing else: the
                        // one-pass reader closed the statement at the brace, and
                        // the body's members are statements of its own.
                        let head: Vec<u32> = self
                            .children(child)
                            .into_iter()
                            .take_while(|&node| self.kind(node) != "class_body")
                            .collect();
                        let value = self.decl_values(&head);
                        let line = self.line(child);
                        if !value.is_empty() && value != "0" {
                            self.lexer_event(b'x', line, &[], &value, false);
                        }
                    } else {
                        run.push(child);
                    }
                }
                // `;` starts the body proper, and its members are read as
                // statements of their own.
                "enum_body_declarations" => self.java_enum_constant_run(&mut run),
                _ => {}
            }
        }
        self.java_enum_constant_run(&mut run);
    }

    fn java_enum_constant_run(&mut self, run: &mut Vec<u32>) {
        if run.is_empty() {
            return;
        }
        let constants = std::mem::take(run);
        let value = self.decl_values(&constants);
        let line = self.line(constants[0]);
        self.lexer_event(b'x', line, &[], &value, false);
    }

    pub(crate) fn java_dotted_declaration(&mut self, id: u32) {
        let mut parts = Vec::new();
        let mut path = String::new();
        let flush = |path: &mut String, parts: &mut Vec<String>| {
            if path.is_empty() {
                return;
            }
            let mut name = "[\"n\",".to_string();
            push_string(&mut name, path);
            name.push(']');
            parts.push(name);
            path.clear();
        };
        let children = self.children(id);
        for (index, &child) in children.iter().enumerate() {
            // A name is part of a dotted path when a dot sits on either side of
            // it; a lone word (`exports`, `requires`) is a name of its own.
            let dotted = children.get(index.wrapping_sub(1)).is_some_and(|&before| self.kind(before) == ".")
                || children.get(index + 1).is_some_and(|&after| self.kind(after) == ".");
            if let Some(name) = self.leaf_name(child).filter(|_| dotted) {
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(&name);
                continue;
            }
            if self.kind(child) == "." {
                continue;
            }
            // `exports a.b to c.d`: the keyword between the two paths ends the
            // first one, so the path is not run together across it.
            flush(&mut path, &mut parts);
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        flush(&mut path, &mut parts);
        let value = match parts.len() {
            0 => return,
            1 => parts.pop().unwrap_or_default(),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        };
        let mut line = self.line(id);
        // Swift: `@frozen` on its own line is an attribute call of its own, so the
        // header starts at the keyword after the modifiers, as the lexer's does.
        if self.language.name == "swift" {
            let first = self.children(id).into_iter().find(|&child| !matches!(self.kind(child), "modifiers" | "declaration_modifiers"));
            if let Some(first) = first {
                line = self.line(first);
            }
        }
        self.lexer_event(b'x', line, &[], &value, false);
    }

    pub(crate) fn java_statements(&mut self, body: u32) {
        for child in self.children(body) {
            self.java_statement(child);
        }
    }

    pub(crate) fn java_statement(&mut self, id: u32) {
        match self.kind(id) {
            "local_variable_declaration" | "field_declaration" => {
                self.java_declaration(id);
                self.java_field_annotations(id);
            }
            "constant_declaration" => self.java_declaration(id),
            "expression_statement" => match self.expression_child(id) {
                Some(child) if self.kind(child) == "assignment_expression" => self.java_assign(child),
                // `this.validator = () -> adapter;`: `=` binds tighter than
                // `->` in this grammar, so a statement-level lambda assignment
                // is a lambda whose head is the assignment. Read as a lambda
                // expression it wrote an `x` whose value merged the binder and
                // the body; the one-pass reader writes the assignment, with
                // the lambda (its parameter list and body) as the value.
                Some(child) if self.kind(child) == "lambda_expression" && self.child_of_kind(child, "assignment_expression").is_some() => {
                    self.java_lambda_assignment(child)
                }
                Some(child) => {
                    let value = self.decl_value(id);
                    let line = self.line(id);
                    self.lexer_event(b'x', line, &[], &value, false);
                    let _ = child;
                }
                None => {}
            },
            "return_statement" => self.java_return(id),
            // `throw new X()`, `assert a == b`, `break`, `continue`, `yield v`:
            // the lexer reads the statement as one expression, so the event is
            // an `x` whose value is the names and calls in it.
            "requires_module_directive" | "exports_module_directive" | "opens_module_directive"
            | "uses_module_directive" | "provides_module_directive" => self.java_dotted_declaration(id),
            "throw_statement" | "assert_statement" | "break_statement" | "continue_statement" | "yield_statement"
            | "labeled_statement" | "synchronized_statement" => {
                let value = self.decl_value(id);
                let line = self.line(id);
                self.lexer_event(b'x', line, &[], &value, false);
            }
            // `String name();` in an interface, `abstract void run();`: a
            // method with no body is a declaration to the grammar and a call to
            // the lexer, which sees the name and its parameter list, so every
            // abstract method in the corpus left a call with no counterpart.
            "method_declaration" => {
                if self.child_of_kind(id, "block").is_none() && self.child_of_kind(id, "constructor_body").is_none() {
                    let children = self.children(id);
                    let name_node = children
                        .iter()
                        .copied()
                        .take_while(|&child| self.kind(child) != "formal_parameters")
                        .filter(|&child| matches!(self.kind(child), "identifier"))
                        .last();
                    let name = name_node.map(|node| self.text_string(node));
                    if let Some(name) = name {
                        let line = self.line(id);
                        let index = self.push_call(name, line);
                        // A method with no body is a statement to the one-pass
                        // reader: it writes one `x` over the whole header, the
                        // call included, and the CST wrote the call alone, so
                        // every abstract and interface method's line was an
                        // event the record did not have
                        // (`<V> @Nullable V get(Object key, Class<V>
                        // requiredType);`, 8 rows of Store.java).
                        let params = children
                            .iter()
                            .copied()
                            .find(|&child| self.kind(child) == "formal_parameters");
                        let args = params.map(|list| self.arguments(list)).unwrap_or_else(|| "[]".to_string());
                        let mut parts: Vec<String> = Vec::new();
                        for &child in &children {
                            if Some(child) == name_node || Some(child) == params {
                                continue;
                            }
                            let value = self.decl_value(child);
                            if !value.is_empty() && value != "0" {
                                parts.push(value);
                            }
                        }
                        parts.push(format!("[\"c\",{index},0,{args}]"));
                        let value = match parts.len() {
                            1 => parts.pop().unwrap_or_default(),
                            _ => format!("[\"m\",[{}]]", parts.join(",")),
                        };
                        self.lexer_event(b'x', line, &[], &value, false);
                    }
                }
            }
            "try_statement" => {
                self.java_statements(id);
            }
            _ => {}
        }
    }

    /// `@SuppressWarnings("unchecked") private int x;`: on a field or a local
    /// the lexer reads the annotation as a call of its name, and nothing wrote
    /// it - only the annotations in front of a class were read.
    pub(crate) fn java_field_annotations(&mut self, id: u32) {
        for child in self.children(id) {
            if self.kind(child) != "modifiers" {
                continue;
            }
            for mark in self.children(child) {
                if !matches!(self.kind(mark), "annotation" | "marker_annotation") {
                    continue;
                }
                let name = self
                    .children(mark)
                    .into_iter()
                    .find(|&node| matches!(self.kind(node), "identifier"))
                    .map(|node| self.text_string(node));
                if let Some(name) = name {
                    let line = self.line(mark);
                    self.push_call(name, line);
                }
            }
        }
    }

    pub(crate) fn java_declaration(&mut self, id: u32) {
        let children = self.children(id);
        let Some(declarator) = children.iter().copied().find(|&child| self.kind(child) == "variable_declarator")
        else {
            return self.decl_statement(id);
        };
        let parts = self.children(declarator);
        let Some(equal) = parts.iter().position(|&child| self.kind(child) == "=") else {
            return self.decl_statement(id);
        };
        if let Some(class) = parts.get(equal + 1).and_then(|&value| self.constructed_class(value)) {
            if let Some(target) = parts.first().and_then(|&name| self.decl_target(name)) {
                self.facts.vars.push((target, class));
            }
        }
        // `int low = from + 1, high = to;` is one statement to the one-pass
        // reader, and its value is the tuple of the comma-separated pieces: the
        // first initialiser, then every later declarator. Read from the first
        // declarator alone, every later declarator vanished from the record —
        // `high` and its `to` were simply gone, and `int a = 1, b = 2, c;` came
        // back as the constant `0` (java: guava's QuantilesAlgorithm, 3 rows of
        // this shape).
        let mut values: Vec<u32> = parts[equal + 1..].to_vec();
        let mut later = false;
        for child in children.iter().copied() {
            if child != declarator && self.kind(child) == "variable_declarator" {
                values.push(child);
                later = true;
            }
        }
        // `Type type = new TypeToken<…>() {}.getType();`: the one-pass reader
        // ends the declaration at the anonymous class's body and reads the
        // chain after it as a statement of its own.
        let split = match &parts[equal + 1..] {
            [node] if !later => self.java_anonymous_split(*node),
            _ => None,
        };
        let value = match &split {
            Some((creation, _)) => creation.clone(),
            None if later => self.decl_values(&values),
            None => self.decl_values(&parts[equal + 1..]),
        };
        let targets = self.java_targets(id);
        let line = self.line(id);
        self.lexer_event(b'a', line, &targets, &value, false);
        if let Some((_, rest)) = split {
            self.lexer_event(b'x', line, &[], &rest, false);
        }
    }

    /// `new TypeToken<…>() {}.getType().hashCode()`: after an anonymous class
    /// body the one-pass reader ends the expression at the creation and reads
    /// the rest as a statement of its own, whose value is the chain with the
    /// creation's call written as the constant 0. Returns the creation's value
    /// and the rest's.
    pub(crate) fn java_anonymous_split(&mut self, id: u32) -> Option<(String, String)> {
        let mut at = id;
        let mut creation = None;
        for _ in 0..64 {
            match self.kind(at) {
                "method_invocation" | "object_creation_expression" => {
                    if self.child_of_kind(at, "class_body").is_some() {
                        creation = Some(at);
                        break;
                    }
                    let base = self.children(at).into_iter().find(|&child| {
                        !self.is_layout(child)
                            && !matches!(self.kind(child), "arguments" | "argument_list" | "type_arguments" | "type_parameter_list")
                    })?;
                    if base == at {
                        break;
                    }
                    at = base;
                }
                "field_access" => {
                    let base = self.children(at).into_iter().find(|&child| !self.is_layout(child))?;
                    if base == at {
                        break;
                    }
                    at = base;
                }
                _ => break,
            }
        }
        let creation = creation?;
        if creation == id {
            return None;
        }
        // One pass only: the creation's call is recorded by the chain's own
        // value, and reading it a second time would give it a second index.
        let callee = self.decl_callee(creation);
        let created = self.decl_path(callee);
        let line = self.line(creation);
        let symbol = self.frame().symbol;
        let before = self.facts.symbols[symbol].calls.len();
        let full = self.decl_value(id);
        let index = self.facts.symbols[symbol].calls[before..]
            .iter()
            .position(|call| call.path == created && call.line == line)
            .map(|at| before + at)?;
        let needle = format!("[\"c\",{index},");
        let start = full.find(&needle)?;
        let end = Self::json_array_end(&full, start)?;
        let creation_form = full[start..end].to_string();
        let rest = format!("{}{}{}", &full[..start], "0", &full[end..]);
        Some((creation_form, rest))
    }

    /// An argument that is an anonymous class or a chain on one. Inside an
    /// argument list the one-pass reader merges the creation with what follows
    /// it (`m:[c(X), body]` for a body, `m:[c(X), rest]` for a chain), while a
    /// declaration keeps the creation alone and the chain as a statement of its
    /// own.
    pub(crate) fn java_argument_value(&mut self, id: u32) -> String {
        if let Some((creation, rest)) = self.java_anonymous_split(id) {
            return format!("[\"m\",[{creation},{rest}]]");
        }
        if self.kind(id) == "object_creation_expression" {
            if let Some(body) = self.child_of_kind(id, "class_body") {
                let members = self.java_anon_body(body);
                if !members.is_empty() && members != "0" {
                    let call = self.decl_value(id);
                    return format!("[\"m\",[{call},{members}]]");
                }
            }
        }
        self.decl_value(id)
    }

    /// The members of an anonymous class body as the one-pass reader writes
    /// them: one merge of what the block's statements read as. A method there
    /// is a call of its name (see [`Self::java_anon_method`]) and every other
    /// member keeps the value the CST gives it.
    pub(crate) fn java_anon_body(&mut self, body: u32) -> String {
        let mut parts = Vec::new();
        for member in self.children(body) {
            if self.is_layout(member) {
                continue;
            }
            if self.kind(member) == "method_declaration" {
                let value = self.java_anon_method(member);
                if !value.is_empty() && value != "0" {
                    parts.push(value);
                }
                continue;
            }
            let value = self.decl_value(member);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        }
    }

    /// `@Override public void handle(String s) throws E { … }` in an anonymous
    /// class body: the one-pass reader opens a plain block at the `{`, so the
    /// declaration is a statement of that block and reads as a call of the
    /// method's name with one merged argument per parameter. Read here as a
    /// class member, that call was never written and the parameters sat as a
    /// merge of their own, so every anonymous class method that takes an
    /// argument was a call the CST's value did not carry — 165 rows of the java
    /// corpus, its largest cluster. The names stay in the form the CST reads
    /// them; the count of calls is what the guard compares.
    pub(crate) fn java_anon_method(&mut self, id: u32) -> String {
        let children = self.children(id);
        let name = children
            .iter()
            .copied()
            .take_while(|&child| self.kind(child) != "formal_parameters")
            .filter(|&child| self.kind(child) == "identifier")
            .last()
            .map(|node| self.text_string(node));
        let mut parts = Vec::new();
        for child in children {
            if self.is_layout(child) {
                continue;
            }
            if self.kind(child) == "formal_parameters" {
                let mut params = Vec::new();
                for param in self.children(child) {
                    if self.kind(param) != "formal_parameter" {
                        continue;
                    }
                    let value = self.decl_value(param);
                    if !value.is_empty() {
                        params.push(value);
                    }
                }
                // A method that takes nothing writes a call of no arguments,
                // which neither record's shape counts (a call with no receiver
                // and no argument names nothing). Keeping the constant the
                // parameter list read as keeps the value's length, which the
                // guard compares when it weighs a lost `0`.
                let line = self.line(id);
                match name.clone().filter(|_| !params.is_empty()) {
                    Some(name) => {
                        let index = self.push_call(name, line);
                        parts.push(format!("[\"c\",{index},0,[{}]]", params.join(",")));
                    }
                    None => parts.push(if params.is_empty() { "0".to_string() } else { format!("[\"m\",[{}]]", params.join(",")) }),
                }
                continue;
            }
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        }
    }

    /// The exclusive end of the JSON array that starts at `start`.
    fn json_array_end(text: &str, start: usize) -> Option<usize> {
        let bytes = text.as_bytes();
        let mut depth = 0i32;
        let mut quote = false;
        let mut escape = false;
        for (at, &byte) in bytes.iter().enumerate().skip(start) {
            if quote {
                if escape {
                    escape = false;
                } else if byte == b'\\' {
                    escape = true;
                } else if byte == b'"' {
                    quote = false;
                }
                continue;
            }
            match byte {
                b'"' => quote = true,
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(at + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub(crate) fn java_targets(&self, id: u32) -> Vec<String> {
        // Commas inside generic type arguments (`Map<Class<?>, Class<?>> m = ...`)
        // or annotations sit inside `<>` / `()` and are not declarator separators.
        let mut parts: Vec<Vec<String>> = vec![Vec::new()];
        let mut leaves = Vec::new();
        self.collect_leaves(id, &mut leaves);
        let mut depth = 0i32;
        for leaf in leaves {
            let text = self.text_string(leaf);
            match text.as_str() {
                "(" | "[" | "{" | "<" => {
                    depth += 1;
                    continue;
                }
                ")" | "]" | "}" | ">" => {
                    depth = (depth - 1).max(0);
                    continue;
                }
                ">>" => {
                    depth = (depth - 2).max(0);
                    continue;
                }
                ">>>" => {
                    depth = (depth - 3).max(0);
                    continue;
                }
                "=" if depth == 0 => break,
                "," if depth == 0 => {
                    parts.push(Vec::new());
                    continue;
                }
                _ => {}
            }
            let bytes = text.as_bytes();
            if bytes.first().is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                if let Some(last) = parts.last_mut() {
                    last.push(text);
                }
            }
        }
        parts.into_iter().filter_map(|part| part.into_iter().next_back()).collect()
    }

    pub(crate) fn java_assign(&mut self, id: u32) {
        let children = self.children(id);
        let split = children.iter().position(|&child| {
            self.node(child).first_child == NONE
                && matches!(self.text_string(child).as_str(), "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=")
        });
        let Some(at) = split else {
            let value = self.decl_value(id);
            let line = self.line(id);
            self.lexer_event(b'x', line, &[], &value, false);
            return;
        };
        let operator = self.text_string(children[at]);
        let targets: Vec<String> = children[..at]
            .iter()
            .filter(|&&child| !self.is_layout(child))
            .filter_map(|&child| self.decl_target(child))
            .collect();
        // `path = new UnixPath(rp);`: an assignment constructs a class for the
        // name it writes exactly as a declaration does, and the one-pass reader
        // records the binding there. Read only out of declarations, every
        // reassignment in the corpus was a binding the CST did not have (java:
        // 358 files, `var path = UnixPath`).
        if let Some(class) = children[at + 1..].first().and_then(|&value| self.constructed_class(value)) {
            if let Some(target) = targets.first() {
                self.facts.vars.push((target.clone(), class));
            }
        }
        let split = match &children[at + 1..] {
            [node, ..] if children.len() == at + 2 => self.java_anonymous_split(*node),
            _ => None,
        };
        let value = match &split {
            Some((creation, _)) => creation.clone(),
            None => self.decl_values(&children[at + 1..]),
        };
        let line = self.line(id);
        self.lexer_event(b'a', line, &targets, &value, operator != "=");
        if let Some((_, rest)) = split {
            self.lexer_event(b'x', line, &[], &rest, false);
        }
    }

    /// `target = params -> body`: one assignment whose value is the lambda.
    pub(crate) fn java_lambda_assignment(&mut self, id: u32) {
        let children = self.children(id);
        let Some(assign) = children.iter().copied().find(|&child| self.kind(child) == "assignment_expression") else { return };
        let assign_children = self.children(assign);
        let Some(equal) = assign_children.iter().position(|&child| self.kind(child) == "=") else { return };
        let Some(&lhs) = assign_children.first() else { return };
        // `this.validator = …`: a field access is a target like any other path,
        // and the one-pass reader spells it `self.validator`.
        let targets: Vec<String> = self.decl_target(lhs).into_iter().collect();
        if targets.is_empty() {
            return;
        }
        let params = assign_children[equal + 1..]
            .iter()
            .copied()
            .find(|&child| !self.is_layout(child))
            .map(|child| {
                let value = self.decl_value(child);
                if value.is_empty() { "0".to_string() } else { value }
            })
            .unwrap_or_else(|| "0".to_string());
        let body = children
            .iter()
            .position(|&child| self.kind(child) == "->")
            .and_then(|at| children[at + 1..].iter().copied().find(|&child| !self.is_layout(child)))
            .map(|child| {
                let value = self.decl_value(child);
                if value.is_empty() { "0".to_string() } else { value }
            })
            .unwrap_or_else(|| "0".to_string());
        let value = format!("[\"m\",[{params},{body}]]");
        let line = self.line(id);
        self.lexer_event(b'a', line, &targets, &value, false);
    }

    pub(crate) fn java_return(&mut self, id: u32) {
        let children = self.children(id);
        let rest: Vec<u32> = children.into_iter().skip(1).collect();
        let value = self.decl_values(&rest);
        let line = self.line(id);
        self.lexer_event(b'r', line, &[], &value, false);
    }

    pub(crate) fn java_header(&mut self, id: u32) {
        let mut parts = Vec::new();
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            let kind = self.kind(child);
            if matches!(
                kind,
                "block" | "class_body" | "constructor_body" | "else_clause" | "catch_clause" | "finally_clause"
                    | "resource_specification" | "switch_block"
            ) {
                continue;
            }
            // `if (c) return y;`, `while (p) break;`, `for (…) go(i);`: the
            // body is a statement of its own — the lexer writes the header's
            // value and the body's events as two things — and folded into the
            // header's value it became one merge that put every body event on
            // the header's line (return/throw/break and the merged `for` body,
            // 1,376 losses with no event on a line both readers can mean).
            // Only the statement kinds the walk never otherwise reads are read
            // here: a nested `if`/`for`/`while` is a node of its own and
            // `java_enter` reaches it, so reading it here would write its events
            // twice.
            if matches!(
                kind,
                "expression_statement"
                    | "local_variable_declaration"
                    | "field_declaration"
                    | "constant_declaration"
                    | "return_statement"
                    | "throw_statement"
                    | "assert_statement"
                    | "break_statement"
                    | "continue_statement"
                    | "yield_statement"
                    | "labeled_statement"
                    | "synchronized_statement"
                    | "try_statement"
            ) {
                self.java_statement(child);
                continue;
            }
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        let value = match parts.len() {
            0 => String::new(),
            1 => parts.pop().unwrap_or_default(),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        };
        if !value.is_empty() {
            let line = self.line(id);
            self.lexer_event(b'x', line, &[], &value, false);
        }
        self.event("[\"b\"]");
    }

    pub(crate) fn java_case(&mut self, id: u32) {
        self.event("[\"e\"]");
        for child in self.children(id) {
            self.java_statement(child);
        }
    }

    /// `catch (InterruptedException e) { … }` read as a value (inside a
    /// lambda's body, a field's initialiser): the one-pass reader has no
    /// keyword table in value position, so the declaration is a call of the
    /// keyword itself with the exception as its only argument, followed by the
    /// block. The CST read the parameter list as a merge and wrote no call, so
    /// every try/catch that a value carries was a call the record did not have
    /// (30 rows of the java corpus, guava's SimpleTimeLimiter two of them).
    pub(crate) fn java_catch_clause(&mut self, id: u32) -> String {
        let mut parts = Vec::new();
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            if self.kind(child) == "catch_formal_parameter" {
                let value = self.decl_value(child);
                if value.is_empty() || value == "0" {
                    continue;
                }
                let line = self.line(id);
                let index = self.push_call("catch".to_string(), line);
                parts.push(format!("[\"c\",{index},0,[{value}]]"));
                continue;
            }
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        }
    }

    pub(crate) fn java_import(&mut self, id: u32) {
        let line = self.line(id);
        let mut parts = Vec::new();
        let mut star = false;
        for child in self.children(id) {
            if self.kind(child) == "*" {
                star = true;
                continue;
            }
            // The segments are read by their text, not their kind: `when`,
            // `module` and `record` are contextual keywords this grammar gives
            // their own kind, and an import of `Mockito.when` names one.
            if let Some(name) = self.leaf_name(child) {
                if name == "import" || (name == "static" && parts.is_empty()) {
                    continue;
                }
                parts.push(name);
            }
        }
        let module = parts.join(".");
        if module.is_empty() {
            return;
        }
        if star {
            self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
        } else {
            let alias = parts.last().cloned().unwrap_or_default();
            self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
        }
    }
}
