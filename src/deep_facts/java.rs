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
            "block" | "class_body" | "enum_body" | "interface_body" | "constructor_body" | "module_body" => {
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
                "class_body" | "enum_body" | "interface_body" | "annotation_type_body" | "record_body" | "constructor_body"
            ) {
                continue;
            }
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        let value = match parts.len() {
            0 => return,
            1 => parts.pop().unwrap_or_default(),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        };
        let line = self.line(id);
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
        let line = self.line(id);
        self.lexer_event(b'x', line, &[], &value, false);
    }

    pub(crate) fn java_statements(&mut self, body: u32) {
        for child in self.children(body) {
            self.java_statement(child);
        }
    }

    pub(crate) fn java_statement(&mut self, id: u32) {
        match self.kind(id) {
            "local_variable_declaration" | "field_declaration" => self.java_declaration(id),
            "constant_declaration" => self.java_declaration(id),
            "expression_statement" => match self.expression_child(id) {
                Some(child) if self.kind(child) == "assignment_expression" => self.java_assign(child),
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
            "try_statement" => {
                self.java_statements(id);
            }
            _ => {}
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
        let value = self.decl_values(&parts[equal + 1..]);
        let targets = self.java_targets(id);
        let line = self.line(id);
        self.lexer_event(b'a', line, &targets, &value, false);
    }

    pub(crate) fn java_targets(&self, id: u32) -> Vec<String> {
        let mut parts: Vec<Vec<String>> = vec![Vec::new()];
        let mut leaves = Vec::new();
        self.collect_leaves(id, &mut leaves);
        for leaf in leaves {
            let text = self.text_string(leaf);
            if text == "=" {
                break;
            }
            if text == "," {
                parts.push(Vec::new());
                continue;
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
        let value = self.decl_values(&children[at + 1..]);
        let line = self.line(id);
        self.lexer_event(b'a', line, &targets, &value, operator != "=");
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
