//! Go: statement models the shared arms do not cover. The declaration,
//! assignment and value helpers here are shared with Java.

use super::*;

impl<'a> Walker<'a> {
    // -- Go and Java --------------------------------------------------------
    //
    // The one-pass lexer reads these two languages as statements over tokens:
    // a `{` after a name opens a block, a `;` (or, in Go, a line end) ends a
    // statement, and every statement is an `a`/`r`/`x` event whose value is the
    // names and calls in it. The tree says exactly which statements exist, so
    // the arms below write the same events from the nodes rather than from a
    // token run. `decl_value` is the value side: the names the lexer would read
    // and the calls it would record, in the same shapes.

    pub(crate) fn go_enter(&mut self, id: u32) {
        match self.kind(id) {
            // Top-level declarations are statements to the lexer, exactly as a
            // block's are: `var version = "0.1.0"` is an assignment on the
            // module symbol.
            "source_file" => self.go_statements(id),
            // `package shop`, `type Repo struct {...}`: a declaration the lexer
            // keeps as a statement. Its value is the whole header, so a field
            // name in it is a name the lexer read on that line too.
            "package_clause" | "type_declaration" => self.decl_statement(id),
            // A struct field is its own statement to the lexer, because `struct
            // {` opened a block: `Name string` sits on the module symbol.
            "field_declaration" => self.decl_statement(id),
            // An interface method is a statement there too: `Get(id int) (...)`.
            "method_elem" => self.decl_statement(id),
            // `~int | ~int64` in a generic constraint is a statement of its own.
            "type_elem" => self.decl_statement(id),
            // `[]string{"GET /"}`: the `{` after `]string` opens a block to the
            // lexer, so the literal's elements are a statement on their own.
            "literal_value" => self.go_literal(id),
            "block" => self.go_statements(id),
            "for_statement" => match self.child_of_kind(id, "range_clause") {
                Some(_) => self.go_range_loop(id),
                None => self.go_header(id),
            },
            "if_statement" | "expression_switch_statement" | "type_switch_statement" | "switch_statement"
            | "select_statement" => self.go_header(id),
            "expression_case" | "default_case" | "type_case" | "communication_case" => self.go_case(id),
            "func_literal" => {
                if let Some(&index) = self.go_literal_frames.get(&id) {
                    let class = self.facts.symbols[index].owner.clone();
                    self.stack.push(Frame {
                        node: id,
                        symbol: index,
                        class,
                        receiver: String::new(),
                        container: false,
                        defines: Some(index),
                    });
                }
            }
            _ => {}
        }
    }


    /// The declaration's header (everything before its body) as one expression
    /// statement, which is what the lexer writes when it does not recognize the
    /// declaration as a class.

    /// A declaration whose name is dotted (`com.example.web`). The lexer joins
    /// the segments, so the value carries the joined name and each keyword
    /// before it as its own name.

    /// One statement-shaped node, as the lexer writes it: an assignment when
    /// there is a binder, a return when the word is `return`, an expression
    /// otherwise.
    pub(crate) fn decl_statement(&mut self, id: u32) {
        let value = self.decl_value(id);
        let line = self.line(id);
        self.lexer_event(b'x', line, &[], &value, false);
    }

    pub(crate) fn go_statements(&mut self, block: u32) {
        for child in self.children(block) {
            self.go_statement(child);
        }
    }


    /// Emit the event `id` would produce as a statement, if it is one. Returns
    /// whether it was: a header calls this for its init statement and falls
    /// back to a plain value for anything else.
    pub(crate) fn go_statement(&mut self, id: u32) -> bool {
        match self.kind(id) {
            "short_var_declaration" | "assignment_statement" => self.go_assign(id),
            "return_statement" => self.go_return(id),
            "var_declaration" | "const_declaration" => self.go_var(id),
            "send_statement" => self.go_send(id),
            // `inserted:` then `s.n += x` on the next line: the lexer reads the
            // label as a statement of its own, and the statement it labels is
            // a statement too. Dropped whole, the statement was a lost event.
            "labeled_statement" => {
                let parts: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child) && self.kind(child) != ":").collect();
                if let Some((&label, body)) = parts.split_first() {
                    let value = self.decl_value(label);
                    let line = self.line(label);
                    self.lexer_event(b'x', line, &[], &value, false);
                    for &child in body {
                        self.go_statement(child);
                    }
                }
            }
            "call_expression" | "defer_statement" | "go_statement" | "inc_statement" | "dec_statement" => {
                self.decl_statement(id)
            }
            _ => return false,
        }
        true
    }


    /// `private Encoding Encoding = new UTF8Encoding(false);`: a C# field
    /// initialised with a creation or a call records the call, and the field's
    /// own value stays the merge of names the lexer writes for a field. Read
    /// through `decl_value` the creation became the whole value instead, which
    /// traded the lost call for a lost event.
    pub(crate) fn field_initializer_calls(&mut self, id: u32) {
        let mut pending = vec![id];
        while let Some(node) = pending.pop() {
            for child in self.children(node) {
                let kind = self.kind(child);
                if matches!(
                    kind,
                    "object_creation" | "object_creation_expression" | "invocation_expression" | "call_expression"
                        | "generic_invocation" | "nameof_expression"
                ) {
                    let path = if kind == "nameof_expression" {
                        "nameof".to_string()
                    } else {
                        let callee = self.decl_callee(child);
                        self.decl_path(callee)
                    };
                    if !path.is_empty() && path != "new" {
                        let line = self.line(child);
                        self.push_call(path, line);
                    }
                } else if kind == "interpolated_string_literal" && self.language.name == "csharp" {
                    self.cs_interpolated_calls(child);
                }
                pending.push(child);
            }
        }
    }

    /// `public static readonly C Instance = new C();`: a C# field or local with
    /// a call initialiser is an assignment to the lexer - `a Instance = c(0;[])`
    /// - and the declaration header event it wrote instead had no counterpart.
    /// Returns whether it wrote one.
    pub(crate) fn cs_field_assignment(&mut self, id: u32) -> bool {
        let children = self.children(id);
        let Some(at) = children.iter().position(|&child| self.kind(child) == "=") else {
            return false;
        };
        let Some(&value) = children.get(at + 1) else { return false };
        if self.language.name == "csharp" && children.iter().filter(|&&c| self.kind(c) == "=").count() > 1 {
            return false;
        }
        if !matches!(
            self.kind(value),
            "invocation_expression" | "object_creation" | "object_creation_expression" | "call_expression"
                | "generic_invocation" | "nameof_expression"
        ) {
            return false;
        }
        if self.language.name == "csharp"
            && self.kind(value) == "object_creation"
            && !self.children(value).iter().any(|&c| matches!(self.kind(c), "arguments" | "argument_list"))
        {
            return false;
        }
        let Some(target) = self.cs_decl_target(&children[..at]) else {
            return false;
        };
        if self.language.name == "csharp" {
            for (idx, &child) in children.iter().enumerate() {
                if idx > at && self.kind(child) == "=" {
                    if let Some(&rhs) = children.get(idx + 1) {
                        let _ = self.form(rhs);
                        if idx > 0 {
                            if let Some(t) = self.leaf_name(children[idx - 1]) {
                                self.cs_bind_constructed(t, rhs);
                            }
                        }
