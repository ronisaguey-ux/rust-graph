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
            "call_expression" | "defer_statement" | "go_statement" | "inc_statement" => {
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
                        | "nameof_expression"
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
        if !matches!(
            self.kind(value),
            "invocation_expression" | "object_creation" | "object_creation_expression" | "call_expression"
                | "nameof_expression"
        ) {
            return false;
        }
        let Some(target) = children[..at]
            .iter()
            .rev()
            .find(|&&child| self.kind(child) == "identifier")
            .map(|&child| self.text_string(child))
        else {
            return false;
        };
        let line = self.line(id);
        // `const string S = nameof(M);`: the lexer's value is the call itself,
        // with the name it names as its argument.
        if self.kind(value) == "nameof_expression" {
            let index = self.push_call("nameof".to_string(), line);
            let mut args = String::new();
            for child in self.children(value) {
                for name in self.c_words_of(child) {
                    let mut form = String::from("[\"n\",");
                    push_string(&mut form, &name);
                    form.push(']');
                    if !args.is_empty() {
                        args.push(',');
                    }
                    args.push_str(&form);
                }
            }
            let form = format!("[\"c\",{index},0,[{args}]]");
            self.lexer_event(b'a', line, &[target], &form, false);
            return true;
        }
        self.assignment_parts(vec![target], value, line, false);
        // `new P { Name = new S(name, false) }`: the outer creation is the
        // value, and each creation inside the initialiser is a call of its own.
        // Read as part of the value's form they were never recorded.
        let mut nested = Vec::new();
        for child in self.children(value) {
            self.c_calls_in_pub(child, &mut nested);
        }
        for call in nested {
            self.form(call);
        }
        true
    }

    /// Calls written in a Scala lambda body. The generic walk reaches the
    /// lambda node but nothing forms its expression body, so a call there
    /// (`(x: T) => self.apply(x)`) stayed out of the call table. A nested
    /// lambda is left to its own visit so a call is not recorded twice.
    pub(crate) fn scala_calls_in_lambda(&self, id: u32, out: &mut Vec<u32>) {
        for child in self.children(id) {
            match self.kind(child) {
                "call" | "call_expression" => out.push(child),
                "lambda_expression" => {}
                _ => self.scala_calls_in_lambda(child, out),
            }
        }
    }

    /// Every call inside `id`, for readers that own the node in another file.
    pub(crate) fn c_calls_in_pub(&self, id: u32, out: &mut Vec<u32>) {
        for child in self.children(id) {
            match self.kind(child) {
                "call_expression" | "invocation_expression" | "object_creation" | "object_creation_expression"
                | "method_invocation" => out.push(child),
                "lambda_expression" | "class_specifier" | "function_definition" | "compound_statement" | "block" => {}
                _ => self.c_calls_in_pub(child, out),
            }
        }
    }

    /// `var Constructor = CreateParameterizedConstructor(t);`: the lexer files
    /// a binding whose value is a capitalised call as an instance of that name,
    /// so a `var` initialised from a factory lost its record (233 rows of one
    /// C# slice, the largest cluster left once the calls themselves were
    /// read). The name is the whole dotted path, as the lexer's chain is.
    pub(crate) fn cs_declared_vars(&mut self, id: u32) {
        let children: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
        // The grammar keeps no `=` in a declaration: `var webHost =
        // CreateHostBuilder(..)?.Build()` is `local_variable_declaration var
        // identifier invocation_expression`, so the initializer is the last
        // child and the declarator is the name right before it.
        let Some(&initializer) = children.last() else {
            return;
        };
        if matches!(
            self.kind(initializer),
            "identifier" | "predefined_type" | "generic_type" | "qualified_type" | "nullable_type" | "pointer_type"
        ) {
            return;
        }
        // `var context.Response.Headers = ToBase64String(x)` binds the dotted
        // path the one-pass reader writes, not just the last name.
        let Some(target) = children[..children.len() - 1]
            .iter()
            .copied()
            .rev()
            .find(|&child| {
                self.kind(child) == "identifier" || matches!(self.kind(child), "member_access" | "field_access")
            })
            .map(|child| self.path_of(child))
            .filter(|path| !path.is_empty())
        else {
            return;
        };
        self.cs_bind_constructed(target, initializer);
    }

    /// Bind `target` to the class of the call at the head of `value`: the
    /// one-pass reader names a binding after the first call it reads, even when
    /// the chain goes on past it (`CreateHostBuilder(..)?.Build()` binds
    /// `CreateHostBuilder`), and only a capitalised name is a class.
    pub(crate) fn cs_bind_constructed(&mut self, target: String, value: u32) {
        let mut pending = vec![value];
        let mut earliest: Option<(u32, u32)> = None;
        while let Some(node) = pending.pop() {
            if matches!(
                self.kind(node),
                "invocation_expression" | "object_creation" | "object_creation_expression" | "call_expression" | "generic_invocation"
            ) {
                let start = self.node(node).start;
                if earliest.map_or(true, |(at, _)| start < at) {
                    earliest = Some((start, node));
                }
            }
            for child in self.children(node) {
                pending.push(child);
            }
        }
        let Some((_, call)) = earliest else {
            return;
        };
        // `CreateHostBuilder(..)?.Build()`: the binding names the call at the
        // head of the chain, so the callee is followed to its leftmost call
        // before the name is read.
        let mut node = call;
        loop {
            let callee = self.decl_callee(node);
            if callee == node {
                break;
            }
            let Some(inner) = self.children(callee).into_iter().find(|&child| !self.is_layout(child)) else {
                break;
            };
            if !matches!(
                self.kind(inner),
                "invocation_expression" | "object_creation" | "object_creation_expression" | "call_expression" | "generic_invocation"
            ) {
                break;
            }
            node = inner;
        }
        let path = self.decl_path(node);
        let last = path.rsplit('.').next().unwrap_or("");
        if last.starts_with(|ch: char| ch.is_uppercase()) {
            self.facts.vars.push((target, last.to_string()));
        }
    }

    /// `int total = 0;`, `List<Item> items = f();`, `private Map<...> index;`.
    /// With an initializer the lexer writes an assignment to the declared name;
    /// without one the statement is an expression, and the type names are in it.

    /// The names a Java declaration binds, read the way the lexer reads them:
    /// every leaf before the initializer's `=`, split at commas, and the last
    /// name of each part. The commas inside a generic type split too, because
    /// the lexer's unpacker counts only brackets: `Map<K, List<Item>> items`
    /// binds `K` and `items` to it.

    /// Every leaf of a node, in source order.
    pub(crate) fn collect_leaves(&self, id: u32, out: &mut Vec<u32>) {
        let mut pending = vec![id];
        let mut order = Vec::new();
        while let Some(node) = pending.pop() {
            let mut child = self.node(node).first_child;
            let mut kids = Vec::new();
            while child != NONE {
                kids.push(child);
                child = self.node(child).next_sibling;
            }
            if kids.is_empty() {
                order.push(node);
            } else {
                for &kid in kids.iter().rev() {
                    pending.push(kid);
                }
            }
        }
        out.extend(order);
    }

    /// `for _, v := range xs { ... }`: the lexer's `{` after the clause is not a
    /// block (the clause holds a top-level assignment), so the loop is one
    /// statement and the event's value carries the body's names too.
    pub(crate) fn go_range_loop(&mut self, id: u32) {
        let mut targets = Vec::new();
        if let Some(clause) = self.child_of_kind(id, "range_clause") {
            for child in self.children(clause) {
                if matches!(self.text_string(child).as_str(), ":=" | "=") {
                    break;
                }
                if self.is_layout(child) {
                    continue;
                }
                if self.kind(child) == "expression_list" {
                    for part in self.children(child) {
                        if let Some(target) = self.decl_target(part) {
                            targets.push(target);
                        }
                    }
                } else if let Some(target) = self.decl_target(child) {
                    targets.push(target);
                }
            }
        }
        // The whole statement, body included: the lexer writes the range clause
        // and the loop body as one value (`for _, a := range xs { go f(a) }`
        // merges both), so narrowing this to the clause alone invented a
        // disagreement that was not there.
        let value = self.decl_value(id);
        let line = self.line(id);
        self.lexer_event(b'a', line, &targets, &value, false);
        self.event("[\"b\"]");
    }

    /// `a, b := f()`, `x = y`, `xs[i] += 1`.
    pub(crate) fn go_assign(&mut self, id: u32) {
        let children = self.children(id);
        let split = children.iter().position(|&child| {
            self.node(child).first_child == NONE
                && matches!(self.text_string(child).as_str(), "=" | ":=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=")
        });
        let Some(at) = split else { return };
        let operator = self.text_string(children[at]);
        let targets: Vec<String> = children[..at]
            .iter()
            .filter(|&&child| !self.is_layout(child))
            .filter_map(|&child| self.decl_target(child))
            .collect();
        if targets.len() == 1 {
            if let Some(class) = children.get(at + 1).and_then(|&value| self.constructed_class(value)) {
                self.facts.vars.push((targets[0].clone(), class));
            }
        }
        // `verifyString := func(s string) { … }`: the one-pass lexer opens a
        // function symbol for the name a literal is bound to, because a `func(`
        // that follows a binder reads as a definition to it. The tree keeps
        // only the literal, so the name was the one fact every Go file with a
        // callback lost — 1,162 symbol losses, one per file.
        if let [target] = targets.as_slice() {
            let literal = children[at + 1..]
                .iter()
                .copied()
                .find(|&child| self.kind(child) == "func_literal");
            if let Some(literal) = literal {
                let params = self
                    .child_of_kind(literal, "parameter_list")
                    .map(|list| self.params(literal, list))
                    .unwrap_or_default();
                if !target.is_empty() && target != "self" {
                    let line = self.line(id);
                    self.add_symbol(target.clone(), String::new(), KIND_FUNCTION, line, params);
                }
            }
        }
        let value = self.decl_values(&children[at + 1..]);
        let line = self.line(id);
        let weak = !matches!(operator.as_str(), "=" | ":=");
        self.lexer_event(b'a', line, &targets, &value, weak);
    }


    pub(crate) fn go_return(&mut self, id: u32) {
        let children = self.children(id);
        let rest: Vec<u32> = children.into_iter().skip(1).collect();
        let value = self.decl_values(&rest);
        let line = self.line(id);
        self.lexer_event(b'r', line, &[], &value, false);
    }


    /// `var zero V` is an expression to the lexer; `var x = f()` is an
    /// assignment to `x`; a `var (...)` group is one expression statement.
    pub(crate) fn go_var(&mut self, id: u32) {
        let children = self.children(id);
        if children.iter().any(|&child| self.kind(child) == "var_spec_list") {
            return self.decl_statement(id);
        }
        let Some(spec) = children.iter().copied().find(|&child| matches!(self.kind(child), "var_spec" | "const_spec"))
        else {
            return self.decl_statement(id);
        };
        let parts = self.children(spec);
        let Some(equal) = parts.iter().position(|&child| self.kind(child) == "=") else {
            return self.decl_statement(id);
        };
        let value = self.decl_values(&parts[equal + 1..]);
        let line = self.line(id);
        // `var a, b int = 1, 2` binds two names, and the lexer writes one event
        // per declarator; reading the first name alone left every later binder
        // with no counterpart and the file fell back. A name is the identifier
        // that opens a comma-separated group: the one after it is the type.
        let mut declared: Vec<String> = Vec::new();
        let mut expect_name = true;
        for &part in &parts[..equal] {
            let kind = self.kind(part);
            if kind == "," {
                expect_name = true;
                continue;
            }
            match (expect_name, kind) {
                (true, "identifier") => {
                    declared.extend(self.decl_target(part));
                    expect_name = false;
                }
                _ => break,
            }
        }
        if declared.is_empty() {
            let target = parts.first().and_then(|&name| self.decl_target(name)).into_iter().collect::<Vec<_>>();
            self.lexer_event(b'a', line, &target, &value, false);
            return;
        }
        for name in declared {
            self.lexer_event(b'a', line, std::slice::from_ref(&name), &value, false);
        }
    }

    /// `ch <- v`: the lexer reads the arrow as an assignment and names the
    /// channel as its target, so the CST has to write an `a` row with the same
    /// target and the sent value; as a plain expression the marker never
    /// matched and every send statement lost its line.
    pub(crate) fn go_send(&mut self, id: u32) {
        let children = self.children(id);
        let Some(arrow) = children
            .iter()
            .position(|&child| self.kind(child) == "<-" || self.text_string(child) == "<-")
        else {
            return self.decl_statement(id);
        };
        let target = children[..arrow]
            .iter()
            .filter(|&&child| !self.is_layout(child))
            .find_map(|&child| self.decl_target(child))
            .into_iter()
            .collect::<Vec<_>>();
        let value = self.decl_values(&children[arrow + 1..]);
        let line = self.line(id);
        self.lexer_event(b'a', line, &target, &value, false);
    }

    /// `if cond {`, `for i := 0; i < n; i++ {`, `switch x {`: every statement the
    /// lexer writes before the body is written here, one event each, and a
    /// branch marker for the body.
    pub(crate) fn go_header(&mut self, id: u32) {
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            let kind = self.kind(child);
            if matches!(kind, "block" | "else_clause" | "literal_value" | "field_declaration_list") {
                continue;
            }
            // `for i := 0; i < n; i++ {`: the lexer ends a statement at every
            // `;`, so the init is an `a`, the condition an `x` and the post an
            // `x`. Read as one header value they were a single `x` the record
            // never wrote.
            if kind == "for_clause" {
                for part in self.children(child) {
                    if self.is_layout(part) || self.kind(part) == ";" {
                        continue;
                    }
                    if self.go_statement(part) {
                        continue;
                    }
                    let value = self.decl_value(part);
                    let line = self.line(part);
                    self.lexer_event(b'x', line, &[], &value, false);
                }
                continue;
            }
            if self.go_statement(child) {
                continue;
            }
            let value = self.decl_value(child);
            let line = self.line(child);
            self.lexer_event(b'x', line, &[], &value, false);
        }
        // `switch v := f(x); v {` and `if v := f(x); p(v) {`: the lexer writes
        // one condition event over the whole header as well, and a value that
        // holds the names of all of it covers that event.
        if let Some(value) = self.header_value(id) {
            let line = self.line(id);
            self.lexer_event(b'x', line, &[], &value, false);
        }
        self.event("[\"b\"]");
    }

    /// The header's values as one expression, or `None` when there is nothing
    /// but the block left.
    pub(crate) fn header_value(&mut self, id: u32) -> Option<String> {
        let mut parts = Vec::new();
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            let kind = self.kind(child);
            if matches!(
                kind,
                "block" | "else_clause" | "literal_value" | "field_declaration_list" | "switch_block"
                    | "catch_clause" | "finally_clause"
            ) {
                continue;
            }
            let value = self.decl_value(child);
            if !value.is_empty() && value != "0" {
                parts.push(value);
            }
        }
        match parts.len() {
            0 => None,
            1 => parts.pop(),
            _ => Some(format!("[\"m\",[{}]]", parts.join(","))),
        }
    }

    /// `[]string{"a", "b"}`: the elements, at the line of the first one, which
    /// is the line the lexer's statement starts on.
    pub(crate) fn go_literal(&mut self, id: u32) {
        let line = self
            .children(id)
            .into_iter()
            .find(|&child| !matches!(self.kind(child), "{" | "}" | ","))
            .map(|child| self.line(child))
            .unwrap_or_else(|| self.line(id));
        let value = self.decl_value(id);
        self.lexer_event(b'x', line, &[], &value, false);
    }

    /// Java keeps a header on one line and reads it as one expression, so the
    /// whole condition is one `x` event, however many values it holds.

    /// `case "x":`, `default:`. The lexer files the arm marker and then reads
    /// the statements after the colon.
    pub(crate) fn go_case(&mut self, id: u32) {
        self.event("[\"e\"]");
        let children = self.children(id);
        let colon = children.iter().position(|&child| {
            self.node(child).first_child == NONE && self.text(child) == b":"
        });
        let after = colon.map(|at| at + 1).unwrap_or(children.len());
        for child in children[after..].iter().copied() {
            if self.is_layout(child) {
                continue;
            }
            if self.go_statement(child) {
                continue;
            }
            let value = self.decl_value(child);
            let line = self.line(child);
            self.lexer_event(b'x', line, &[], &value, false);
        }
    }


    /// The value the lexer writes for a run of nodes: one merge of them, or the
    /// single value when there is only one.
    pub(crate) fn decl_values(&mut self, ids: &[u32]) -> String {
        let mut parts = Vec::new();
        for &id in ids {
            let value = self.decl_value(id);
            if !value.is_empty() {
                parts.push(value);
            }
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            // A run of values from a comma-separated list is a tuple to the
            // lexer, not a merge.
            _ => format!("[\"t\",[{}]]", parts.join(",")),
        }
    }

    /// The names the lexer reads in these nodes, as a flow value. Every leaf
    /// whose text is a word is a name (the lexer has no keyword table for these
    /// two grammars: `type`, `package` and `private` are names to it), a
    /// literal is `0`, and a call is one call in the symbol's table. A dotted
    /// name (`sync.Mutex`, `this.index`) is written as the path the lexer
    /// builds from it.
    pub(crate) fn decl_value(&mut self, id: u32) -> String {
        let node = self.node(id);
        if node.first_child == NONE {
            if node.start == node.end {
                return String::new();
            }
            let text = self.text(id);
            let first = text[0];
            if first.is_ascii_digit() || first == b'"' || first == b'\'' || first == b'`' {
                return "0".to_string();
            }
            if first.is_ascii_alphabetic() || first == b'_' {
                // `this` is the flow form's `self`, exactly as `form`'s leaf arm
                // and `path_of` spell it: the lexer names the receiver `self` in
                // every language, and read verbatim the java record wrote
                // `n:this` for the one value the lexer called `n:self` — one
                // loss per `this`, `return this;` and `new X(this)` alike
                // (1,046 losses, the largest java cluster left after the
                // declarations).
                // A Go method receiver is written with its own name, so a bare
                // `p` in a condition is the receiver; `decl_path` already spells
                // it `self` and the lexer does too, so the leaf has to agree.
                let receiver = self.frame().receiver.as_str();
                let name: &[u8] = if text == b"this" || (!receiver.is_empty() && text == receiver.as_bytes()) {
                    b"self"
                } else {
                    text
                };
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &String::from_utf8_lossy(name));
                out.push(']');
                return out;
            }
            return String::new();
        }
        let kind = self.kind(id);
        // A call is a call in the flow form the lexer writes: the index into
        // the symbol's call table, with the names and arguments beside it.
        // A call is the whole value: the lexer writes `["c",index,recv,args]`
        // and nothing beside it. Descending into the call's children as well
        // produced a merge of the callee's name and the raw arguments, which
        // never equals the lexer's event and made every Go file look like it had
        // lost a fact.
        // `nameof(x)` is a call to the lexer, but the *statement* it sits in is
        // still a name to it, so the call is recorded here and the value keeps
        // falling through to the plain-name path.
        if kind == "nameof_expression" {
            let line = self.line(id);
            self.push_call("nameof".to_string(), line);
        }
        // `new int[n]`, `new byte[len]`: an array creation states a type and
        // the sizes it is made with, and it calls nothing. Written as an
        // anonymous call (the callee is a primitive, the size is a
        // `dimensions_expr` and not an `arguments`) the value named nothing at
        // all, so the type the one-pass reader keeps had no counterpart in the
        // record (java: 152 files, `n:int` against `c(0;[])`).
        if kind == "object_creation_expression"
            && self
                .children(id)
                .iter()
                .any(|&child| self.kind(child) == "dimensions_expr")
        {
            let mut parts = Vec::new();
            for child in self.children(id) {
                let child_kind = self.kind(child);
                if child_kind == "dimensions_expr" {
                    let value = self.decl_value(child);
                    if !value.is_empty() && value != "0" {
                        parts.push(value);
                    }
                    continue;
                }
                if let Some(name) = self.leaf_name(child) {
                    if name == "new" {
                        continue;
                    }
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &name);
                    out.push(']');
                    parts.push(out);
                }
            }
            return match parts.len() {
                0 => "0".to_string(),
                1 => parts.pop().unwrap_or_else(|| "0".to_string()),
                _ => format!("[\"m\",[{}]]", parts.join(",")),
            };
        }
        if matches!(kind, "call_expression" | "method_invocation" | "object_creation_expression") {
            let callee = self.decl_callee(id);
            // `NewBuilder(foo).WithCount(count(baz)).Build()`, `makeFn(prep(a))(b)`,
            // `getItems(makeKey(x))[pickIdx(x)].Render()`: when the receiver chain
            // carries calls of its own, form that receiver expression directly so
            // every intermediate call, its arguments, and its names reach the record.
            let receiver = match self.decl_receiver_expr(id, callee) {
                Some(expr) => self.decl_value(expr),
                None => {
                    let path = self.decl_path(callee);
                    match path.rsplit_once('.') {
                        Some((head, _)) if !head.is_empty() => {
                            let mut out = "[\"n\",".to_string();
                            push_string(&mut out, head);
                            out.push(']');
                            out
                        }
                        _ => "0".to_string(),
                    }
                }
            };
            let index = self.decl_call(id);
            let args = self
                .children(id)
                .into_iter()
                .find(|&child| matches!(self.kind(child), "arguments" | "argument_list"))
                .map(|list| self.arguments(list))
                .unwrap_or_else(|| "[]".to_string());
            let call = format!("[\"c\",{index},{receiver},{args}]");
            // `new X() { … }`: the anonymous class body is part of the creation,
            // and the lexer merges it with the call (`m:[c(X), body]`, `0` for an
            // empty body). Dropping it lost every java `new` with a body that sat
            // in an argument list.
            if self.language.name == "java" && kind == "object_creation_expression" {
                if let Some(body) = self.children(id).into_iter().find(|&child| self.kind(child) == "class_body") {
                    let body = self.decl_value(body);
                    let body = if body.is_empty() { "0".to_string() } else { body };
                    return format!("[\"m\",[{call},{body}]]");
                }
            }
            return call;
        }
        // C# `[Attr(…)]`: the lexer reads an attribute section as a value with
        // one element per attribute - a call when the attribute has arguments,
        // a bare name when it has none - and records every attribute that has
        // arguments as a call of its dotted name. The tree flattened the whole
        // section into name words, so every attributed declaration in the
        // corpus lost the call: `[Obsolete(…)]`, `[DllImport(…)]`,
        // `[LibraryImport(…)]` and `[DefaultDllImportSearchPaths(…)]` were
        // 1,081 of the corpus files on their own.
        if self.language.name == "csharp" && matches!(kind, "attribute_section" | "attribute_list") {
            return self.csharp_attribute_section(id, false);
        }
        let mut parts = Vec::new();
        if matches!(
            kind,
            // `JObject.Parse(x)`: the C# grammar keeps a member access as
            // `member_access` with two name leaves, and without it every call
            // through a type or an object lost its receiver and the whole
            // dotted name with it - 592 rows of one C# slice.
            "member_access"
                | "selector_expression" | "qualified_type" | "field_access" | "dotted_name" | "scoped_identifier"
                | "navigation_expression"
                | "scoped_type_identifier" | "nested_type_identifier" | "class_literal" | "method_reference"
        ) {
            let path = self.decl_path(id);
            if !path.is_empty() {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &path);
                out.push(']');
                parts.push(out);
            }
        }
        for child in self.children(id) {
            let value = self.decl_value(child);
            if !value.is_empty() {
                parts.push(value);
            }
        }
        let comma = self.children(id).iter().any(|&child| self.kind(child) == ",");
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            _ if comma => format!("[\"t\",[{}]]", parts.join(",")),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        }
    }

    /// One C# attribute section as the lexer writes it: `["t",[ … ]]` with a
    /// call per attribute that has arguments and a name per attribute that has
    /// none. `record_only` writes the calls and no value, which is what a
    /// second section on one declaration does: the lexer keeps its calls and
    /// never puts it in the header event.
    /// Append a C# declaration's attribute list to its header event. Only the
    /// first section is written; every later one is record-only, because that
    /// is where the one-pass reader puts it.
    pub(crate) fn csharp_attributes(&mut self, id: u32, attributed: &mut bool, parts: &mut Vec<String>) {
        if matches!(self.kind(id), "attribute_section" | "attribute_list") {
            let value = self.csharp_attribute_section(id, *attributed);
            *attributed = true;
            if !value.is_empty() {
                parts.push(value);
            }
            return;
        }
        for section in self.children(id) {
            if matches!(self.kind(section), "attributes" | "attribute_section" | "attribute_list") {
                self.csharp_attributes(section, attributed, parts);
            }
        }
    }

    pub(crate) fn csharp_attribute_section(&mut self, id: u32, record_only: bool) -> String {
        if let Some(text) = self.call_texts.get(&id) {
            return if record_only { String::new() } else { text.clone() };
        }
        let mut pieces: Vec<String> = Vec::new();
        for child in self.children(id) {
            if self.kind(child) != "attribute" {
                continue;
            }
            // `[return: NotNull]`, `[assembly: X]`: the target is part of the
            // section, not part of the attribute's name.
            let mut members = self.children(child);
            if let Some(at) = members.iter().position(|&node| self.kind(node) == ":") {
                members = members[(at + 1)..].to_vec();
            }
            let list = members.iter().copied().find(|&node| {
                matches!(self.kind(node), "attribute_arguments" | "arguments" | "argument_list")
            });
            let mut words: Vec<String> = Vec::new();
            for node in members.iter().copied() {
                if Some(node) == list || matches!(self.kind(node), "." | "(") {
                    continue;
                }
                if let Some(word) = self.leaf_name(node) {
                    words.push(word);
                }
            }
            let path = words.join(".");
            let mut items: Vec<u32> = Vec::new();
            if let Some(list) = list {
                for node in self.children(list) {
                    if self.is_layout(node) || matches!(self.kind(node), "(" | ")" | ",") {
                        continue;
                    }
                    items.push(node);
                }
            }
            if items.is_empty() {
                // `[Obsolete]` is a name to the lexer and never a call.
                if !record_only && !path.is_empty() {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &path);
                    out.push(']');
                    pieces.push(out);
                }
                continue;
            }
            let line = self.line(child);
            let index = self.push_call(path.clone(), line);
            let mut args = String::from("[");
            for (nth, node) in items.iter().copied().enumerate() {
                if nth > 0 {
                    args.push(',');
                }
                // `Value = "a"`: a named attribute argument is the lexer's
                // keyword form `["k",name,value]`.
                let inner = self.children(node);
                // `EntryPoint = "getppid"`: the C# grammar keeps a named
                // attribute argument as an assignment, and the lexer reads it
                // as a keyword argument, so the name is part of the value.
                let named = matches!(self.kind(node), "assignment" | "assignment_expression" | "named_attribute_argument")
                    || inner.iter().any(|&row| self.kind(row) == ":");
                let value = if named {
                    let name = inner
                        .iter()
                        .copied()
                        .filter(|&row| self.kind(row) != ":" && self.leaf_name(row).is_some())
                        .next()
                        .and_then(|row| self.leaf_name(row))
                        .unwrap_or_default();
                    let value = inner
                        .iter()
                        .copied()
                        .rev()
                        .find(|&row| self.kind(row) != ":" && !self.is_layout(row) && Some(row) != inner.first().copied())
                        .map(|row| self.form(row))
                        .unwrap_or_else(|| "0".to_string());
                    let mut keyed = "[\"k\",".to_string();
                    push_string(&mut keyed, &name);
                    keyed.push(',');
                    keyed.push_str(&value);
                    keyed.push(']');
                    keyed
                } else {
                    self.form(node)
                };
                args.push_str(&value);
            }
            args.push(']');
            let receiver = match path.rsplit_once('.') {
                Some((head, _)) if !head.is_empty() => {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, head);
                    out.push(']');
                    out
                }
                _ => "0".to_string(),
            };
            pieces.push(format!("[\"c\",{index},{receiver},{args}]"));
        }
        let text = if pieces.is_empty() { String::new() } else { format!("[\"t\",[{}]]", pieces.join(",")) };
        self.call_texts.insert(id, text.clone());
        if record_only { String::new() } else { text }
    }

    /// The value a C# lambda writes into the statement that holds it: its
    /// body's value, with every call in that body recorded. An empty block, or
    /// a body that writes nothing, is `0` - which is what the one-pass reader
    /// writes for `() => { }`.
    pub(crate) fn cs_lambda_value(&mut self, id: u32) -> String {
        let Some(body) = self.children(id).into_iter().rev().find(|&child| !self.is_layout(child)) else {
            return "0".to_string();
        };
        if self.kind(body) != "block" {
            return self.form(body);
        }
        let mut parts: Vec<String> = Vec::new();
        for statement in self.children(body) {
            if self.is_layout(statement) {
                continue;
            }
            let value = self.form(statement);
            if !value.is_empty() {
                parts.push(value);
            }
        }
        match parts.len() {
            0 => "0".to_string(),
            1 => parts.pop().unwrap_or_else(|| "0".to_string()),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        }
    }

    /// The value a C# generic invocation writes: the callee's name, then each
    /// type argument's name, then the arguments' value, merged. Every call
    /// inside the arguments is recorded on the way, because it is part of the
    /// same statement to the one-pass reader.
    pub(crate) fn cs_generic_invocation(&mut self, id: u32) -> String {
        let children = self.children(id);
        let mut parts: Vec<String> = Vec::new();
        for (index, &child) in children.iter().enumerate() {
            let kind = self.kind(child);
            if index == 0 {
                let path = self.path_of(child);
                if !path.is_empty() {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &path);
                    out.push(']');
                    parts.push(out);
                }
                continue;
            }
            if kind == "type_arguments" {
                for arg in self.children(child) {
                    if self.is_layout(arg) {
                        continue;
                    }
                    let path = self.path_of(arg);
                    if path.is_empty() {
                        continue;
                    }
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &path);
                    out.push(']');
                    parts.push(out);
                }
                continue;
            }
            if matches!(kind, "arguments" | "argument_list") {
                let mut items: Vec<String> = Vec::new();
                for arg in self.children(child) {
                    if self.is_layout(arg) {
                        continue;
                    }
                    items.push(self.form(arg));
                }
                let value = match items.len() {
                    0 => continue,
                    1 => items.pop().unwrap_or_else(|| "0".to_string()),
                    _ => format!("[\"t\",[{}]]", items.join(",")),
                };
                parts.push(value);
                continue;
            }
            if self.is_layout(child) {
                continue;
            }
            let value = self.form(child);
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

    /// One call, at the line its callee is written on, named the way the source
    /// spells it. Returns its index in the symbol's call table, which is what
    /// the flow form's `["c",id,...]` names.
    pub(crate) fn decl_call(&mut self, id: u32) -> usize {
        let callee = self.decl_callee(id);
        let path = self.decl_path(callee);
        let line = self.call_line(callee, id);
        self.push_call(path, line)
    }


    /// The receiver expression of a chained call when it contains a call of its
    /// own (`a(x).b(y)`, `f(x)(y)`, `getItems(k)[idx()].Render()`).
    pub(crate) fn decl_receiver_expr(&self, call: u32, callee: u32) -> Option<u32> {
        if callee == call {
            return None;
        }
        match self.kind(callee) {
            "call_expression" | "method_invocation" | "object_creation_expression" => Some(callee),
            "selector_expression" | "member_expression" | "field_access" => {
                let object = self.children(callee).into_iter().find(|&child| !self.is_layout(child))?;
                (object != call && self.decl_has_call(object)).then_some(object)
            }
            _ => None,
        }
    }

    /// Whether `id` contains a call node outside nested closures/blocks.
    fn decl_has_call(&self, id: u32) -> bool {
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            match self.kind(node) {
                "call_expression" | "method_invocation" | "object_creation_expression" => return true,
                "func_literal" | "block" | "lambda_expression" | "arrow_function" => {}
                _ => {
                    for child in self.children(node) {
                        stack.push(child);
                    }
                }
            }
        }
        false
    }

    /// The callee of a call node: the expression before the argument list.
    pub(crate) fn decl_callee(&self, id: u32) -> u32 {
        let children = self.children(id);
        let kind = self.kind(id);
        if kind == "object_creation_expression" || kind == "object_creation" {
            return children
                .iter()
                .copied()
                .find(|&child| !matches!(self.kind(child), "new" | "arguments" | "type_arguments"))
                .unwrap_or(id);
        }
        // Stop at the call's argument list, not `type_arguments`: Java's
        // `Foo.<T>bar(x)` places `type_arguments` before the method name, and
        // the reverse scan below already skips `type_arguments` when it follows
        // the callee (e.g. `foo<T>(x)`).
        let args = children
            .iter()
            .position(|&child| matches!(self.kind(child), "arguments" | "argument_list"));
        let end = args.unwrap_or(children.len());
        // The callee is read by its text, so a member this grammar gives a
        // keyword kind (`Mockito.when(x)`, `obj.record`) still has a name.
        children[..end]
            .iter()
            .copied()
            .rev()
            .find(|&child| {
                self.leaf_name(child).is_some()
                    || (!self.is_layout(child) && !matches!(self.kind(child), "type_arguments" | "type_parameter_list" | "index"))
            })
            .unwrap_or(id)
    }

    /// A dotted name built the way the lexer builds one, with the enclosing
    /// method's receiver spelled `self` as both paths spell it.
    pub(crate) fn decl_path(&self, id: u32) -> String {
        match self.kind(id) {
            "identifier" | "type_identifier" | "field_identifier" | "this" | "super" => {
                let text = self.text_string(id);
                let receiver = self.frame().receiver.as_str();
                if text == "this" || (!receiver.is_empty() && text == receiver) {
                    "self".to_string()
                } else {
                    text
                }
            }
            "selector_expression" | "qualified_type" | "field_access" | "scoped_identifier" | "dotted_name"
            | "scoped_type_identifier" | "nested_type_identifier" | "class_literal" | "method_reference"
            // Kotlin's `user.title` in a template hole is a navigation; the
            // path is the receiver and the member, like every other dotted name.
            | "navigation_expression"
            // `JObject.Parse`: the C# member access carries two name leaves and
            // no dotted kind at all, so every call through a type lost its
            // receiver - 592 rows of one C# slice.
            | "member_access" => {
                let children = self.children(id);
                // The last segment is read by its text: `Mockito.when` names a
                // member called `when`, which this grammar gives a keyword kind.
                let name = children
                    .iter()
                    .copied()
                    .rev()
                    .find(|&child| self.kind(child) != "." && self.leaf_name(child).is_some())
                    .map(|child| self.decl_path(child))
                    .unwrap_or_default();
                let base = children
                    .first()
                    .copied()
                    .filter(|&first| self.kind(first) != ".")
                    .map(|first| self.decl_path(first))
                    .unwrap_or_default();
                let base = if base.is_empty() { String::new() } else { format!("{base}.") };
                format!("{base}{name}")
            }
            "method_invocation" | "invocation_expression" | "object_creation" | "object_creation_expression" => {
                // A call has no name of its own: the name belongs to its callee.
                let callee = self.decl_callee(id);
                if callee == id {
                    return String::new();
                }
                self.decl_path(callee)
            }
            "call_expression" => {
                let callee = self.decl_callee(id);
                if callee == id {
                    return String::new();
                }
                self.decl_path(callee)
            }
            "index_expression" | "array_access" => {
                self.children(id).first().map(|&base| self.decl_path(base)).unwrap_or_default()
            }
            // `[]byte(x)`, `*T`, `map[K]V`: a conversion names its type, and the
            // lexer reads the type's own name as the callee.
            "array_type" | "pointer_type" | "map_type" | "slice_type" | "channel_type" => self
                .last_name(id)
                .or_else(|| self.leaf_name(id))
                .unwrap_or_default(),
            "parenthesized_expression" | "unary_expression" | "type_assertion" => self
                .expression_child(id)
                .map(|child| self.decl_path(child))
                .unwrap_or_default(),
            "generic_type" => self.children(id).first().map(|&base| self.decl_path(base)).unwrap_or_default(),
            _ => self.leaf_name(id).unwrap_or_default(),
        }
    }

    /// The name an assignment target binds: `x`, `this.index`, `xs[0]` -> `xs`
    /// (`x, y := ...` splits into two children before this is called).
    pub(crate) fn decl_target(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "identifier" | "field_identifier" | "this" | "selector_expression" | "field_access" => {
                let path = self.decl_path(id);
                (!path.is_empty()).then_some(path)
            }
            // `ek[3] |= 2`: a subscript names its receiver, and Java's grammar
            // spells it `array_access` where Go's says `index_expression`. With
            // no arm the target list was empty, so every `a[i] = v` statement
            // was a target list the CST never wrote (1,008 rows over 89 files,
            // the largest java cluster left).
            "index_expression" | "array_access" => {
                let path = self.children(id).first().map(|&base| self.decl_path(base)).unwrap_or_default();
                (!path.is_empty()).then_some(path)
            }
            "parenthesized_expression" => self.expression_child(id).and_then(|child| self.decl_target(child)),
            // `*out = *in`: a dereference writes through the name it points at,
            // and the one-pass lexer files that name as the statement's target.
            // With no arm the CST wrote an empty target list for every `*p = v`
            // statement - `*out = ...` in each deepcopy and conversion function
            // of the generated kubernetes files - so the guard read a lost
            // statement and kept the lexer on the whole file.
            "unary_expression" | "pointer_expression"
                if self.children(id).first().map(|&op| self.text_string(op)).as_deref() == Some("*") =>
            {
                self.children(id)
                    .into_iter()
                    .skip(1)
                    .find(|&child| !self.is_layout(child))
                    .and_then(|child| self.decl_target(child))
            }
            _ => None,
        }
    }

    /// The class a value constructs, by the lexer's convention: a capitalised
    /// callee, a composite literal's type, or anything after `new`.
    pub(crate) fn constructed_class(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "object_creation_expression" => {
                let path = self.decl_path(self.decl_callee(id));
                let last = path.rsplit('.').next().unwrap_or("").to_string();
                last.starts_with(|ch: char| ch.is_uppercase()).then_some(last)
            }
            "call_expression" | "method_invocation" => {
                // The lexer's convention: a callee whose first *and* last names
                // are capitalised constructs that class (`Repo()`, `New()`); a
                // composite literal is not a call and binds nothing.
                let path = self.decl_path(id);
                let first = path.split('.').next().unwrap_or("");
                let last = path.rsplit('.').next().unwrap_or("").to_string();
                (first.starts_with(|ch: char| ch.is_uppercase())
                    && last.starts_with(|ch: char| ch.is_uppercase())
                    && !last.contains('('))
                .then_some(last)
            }
            _ => None,
        }
    }

    /// One flow event, written the way `crate::expr` documents it.
    pub(crate) fn lexer_event(&mut self, tag: u8, line: u32, targets: &[String], value: &str, weak: bool) {
        let text = match tag {
            b'a' => {
                let mut text = format!("[\"a\",{line},[");
                for (index, name) in targets.iter().enumerate() {
                    if index > 0 {
                        text.push(',');
                    }
                    push_string(&mut text, name);
                }
                text.push_str(&format!("],{value},{}]", u8::from(weak)));
                text
            }
            _ => format!("[\"{}\",{line},{value}]", tag as char),
        };
        self.event(&text);
    }

    /// `func (c *Cache[K, V]) Get`: the receiver's own name and its type's.
    pub(crate) fn go_receiver(&self, list: u32) -> (String, String) {
        let Some(param) = self.children(list).into_iter().find(|&child| self.kind(child) == "parameter_declaration")
        else {
            return (String::new(), String::new());
        };
        let children = self.children(param);
        let name = children
            .iter()
            .copied()
            .find(|&child| self.kind(child) == "identifier")
            .map(|child| self.text_string(child))
            .unwrap_or_default();
        let owner = children
            .iter()
            .copied()
            .rev()
            .find_map(|child| self.receiver_type(child))
            .unwrap_or_default();
        (name, owner)
    }

    /// The type a receiver declaration names, unwrapping a pointer and taking
    /// a generic's base name: `*Cache[K, V]` -> `Cache`.
    pub(crate) fn receiver_type(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "identifier" | "type_identifier" => Some(self.text_string(id)),
            "pointer_type" | "generic_type" => self.children(id).into_iter().find_map(|child| self.receiver_type(child)),
            "qualified_type" => self.last_name(id),
            _ => None,
        }
    }

    pub(crate) fn pattern_names(&self, id: u32) -> Vec<String> {
        let mut out = Vec::new();
        let mut walk = vec![id];
        while let Some(node) = walk.pop() {
            match self.kind(node) {
                "shorthand_property_identifier_pattern" | "shorthand_property_identifier" => {
                    out.push(self.text_string(node))
                }
                // `{ a, b: c } = obj` binds `a` and `c`. The key `b` is how the
                // source names the property, and both the one-pass reader and
                // the resolver want the binders only.
                // `{ a, b: c } = obj` binds `a` and `c`. The key `b` is how the
                // source names the property; the one-pass reader and the
                // resolver both want the binders only.
                "pair" | "pair_pattern" => {
                    if let Some(last) = self.children(node).into_iter().last() {
                        walk.push(last);
                    }
                }
                "object_assignment_pattern" | "assignment_pattern" => {
                    if let Some(first) = self.children(node).into_iter().next() {
                        walk.push(first);
                    }
                }
                "subscript" | "subscript_expression" | "index_expression" => {
                    if let Some(first) = self.children(node).into_iter().next() {
                        walk.push(first);
                    }
                }
                "member_expression" | "attribute" => {
                    let path = self.path_of(node);
                    if !path.is_empty() {
                        out.push(path);
                    }
                }
                "identifier" | "property_identifier" | "private_property_identifier" => {
                    out.push(self.text_string(node).trim_start_matches('#').to_string())
                }
                // The TypeScript grammar gives `is`, `as` and `satisfies` their
                // own node kind because they are type-position keywords, so
                // `const is = require('./get')` bound nothing and the import
                // vanished from the record. A binder is a binder whatever the
                // keyword table calls the word.
                // `const { offset, type, source, end } = x` binds four names in
                // that order; the grammar spells `type` with its own keyword
                // kind, and reading it as a leaf dropped it from the targets.
                // `const { from, to } = x` binds `from`, and the grammar gives
                // `from` (like `type`) its own kind because both are keywords in
                // another position. A blanket rule for keyword leaves also
                // picked up `catch` in `try {…} catch (e)`, so the list stays
                // explicit.
                "is" | "as" | "satisfies" | "get" | "set" | "static" | "async" | "readonly" | "declare" | "type"
                | "from" | "of" | "module" | "keyof" => {
                    out.push(self.text_string(node))
                }
                _ => {
                    // Children are pushed in reverse so the stack pops them in
                    // source order: the binding order is part of the record.
                    let mut kids = Vec::new();
                    let mut child = self.node(node).first_child;
                    while child != NONE {
                        kids.push(child);
                        child = self.node(child).next_sibling;
                    }
                    for &kid in kids.iter().rev() {
                        walk.push(kid);
                    }
                }
            }
        }
        out
    }

    // -- statements ---------------------------------------------------------

    pub(crate) fn expression_statement(&mut self, id: u32) {
        let Some(mut child) = self.expression_child(id) else { return };
        // `@Suppress("DEPRECATION") coreLibrariesVersion = ...`: Kotlin's
        // annotation wraps the statement in an `annotated_expression` whose
        // line is the annotation's. Unwrap it so the inner assignment (and its
        // line) dispatches as an assignment instead of a generic expression.
        if self.is_kotlin() {
            while self.kind(child) == "annotated_expression" {
                let Some(inner) = self
                    .children(child)
                    .into_iter()
                    .rev()
                    .find(|&c| !self.is_layout(c) && !matches!(self.kind(c), "annotation" | "annotation_item"))
                else {
                    return;
                };
                child = inner;
            }
        }
        if let Some((lhs, module)) = self.require_assignment(child) {
            let line = self.line(id);
            if matches!(self.kind(lhs), "object_pattern" | "object") {
                self.js_require_destructured(lhs, &module, line);
            } else if let Some(alias) = self.last_name(lhs) {
                self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
            }
            return;
        }
        if let Some(module) = self.require_module(child) {
            let line = self.line(id);
            self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
            if self.direct_require_module(child).is_some() {
                return;
            }
        }
        match self.kind(child) {
            "assignment" | "assignment_expression" => self.assignment(child, false),
            // Rust's `=` is the same statement: the lexer records the target,
            // the value and the class the value constructs.
            _ if self.is_rust() && self.kind(child) == "assignment_expression" => {
                self.assignment(child, false)
            }
            "augmented_assignment" | "augmented_assignment_expression" | "compound_assignment_expr" => self.assignment(child, true),
            _ if self.is_rust()
                && matches!(
                    self.kind(child),
                    "if_expression" | "while_expression" | "loop_expression" | "for_expression" | "match_expression"
                ) => {}
            // `x: T = f()`: the targets are the names and the value is the last
            // child, as in an assignment. `x: T` alone binds nothing and has no
            // value, so only the annotated form that carries `=` is read.
            "annotated" => {
                if self.children(child).into_iter().any(|part| self.kind(part) == "=") {
                    self.assignment(child, false);
                } else {
                    // `rule_id: str` binds nothing, but the lexer reads the
                    // name and the annotation as one expression and files an
                    // event for it. Emitting nothing here lost that event in
                    // every dataclass-shaped class body.
                    let mut parts = Vec::new();
                    for part in self.children(child) {
                        if self.is_layout(part) || self.kind(part) == ":" {
                            continue;
                        }
                        let value = self.form(part);
                        // `some_b: 'B'`: a string annotation is a constant, and
                        // the lexer writes the slot the value reader gives it.
                        // Dropping it left the merge one item short.
                        if value != "0" || self.kind(part) == "string" {
                            parts.push(value);
                        }
                    }
                    if !parts.is_empty() {
                        let line = self.line(child);
                        self.event(&format!("[\"x\",{line},[\"m\",[{}]]]", parts.join(",")));
                    }
                }
            }
            // `pos++;` as a statement is the operand's name, which is what the
            // one-pass lexer writes. Read only here, not in `form`: an update
            // inside a larger expression is not a statement of its own.
            "update_expression" | "postfix_expression" => {
                if let Some(operand) = self.expression_child(child) {
                    let value = self.form(operand);
                    if value != "0" {
                        let line = self.line(child);
                        self.event(&format!("[\"x\",{line},{value}]"));
                    }
                }
            }
            _ => {
                let value = self.form(child);
                if value != "0" {
                    let line = self.line(child);
                    self.event(&format!("[\"x\",{line},{value}]"));
                }
            }
        }
    }

    /// A statement whose whole content is values: `raise`, `assert`, `del`.
    /// The keywords (`from`, `assert`, `del`) are layout and are skipped by
    /// [`Self::is_layout`]; each remaining child is an expression to read.
    pub(crate) fn statement_values(&mut self, id: u32) {
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            self.form(child);
        }
    }

    pub(crate) fn return_statement(&mut self, id: u32) {
        let line = self.line(id);
        let exprs: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
        let value = match exprs.as_slice() {
            [] => "0".to_string(),
            [child] => {
                if let Some(module) = self.require_module(*child) {
                    self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
                }
                self.form(*child)
            }
            many => {
                let parts: Vec<String> = many
                    .iter()
                    .map(|&child| {
                        if let Some(module) = self.require_module(child) {
                            self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
                        }
                        self.form(child)
                    })
                    .collect();
                format!("[\"m\",[{}]]", parts.join(","))
            }
        };
        self.event(&format!("[\"r\",{line},{value}]"));
    }

    /// Extracts the module string if `node` itself is a `require('...')` call.
    pub(crate) fn direct_require_module(&self, node: u32) -> Option<String> {
        let kind = self.kind(node);
        if matches!(kind, "call_expression" | "call" | "import_require_clause") {
            let children = self.children(node);
            if let Some(&callee) = children.first() {
                if self.kind(callee) == "identifier" && self.text_string(callee) == "require" {
                    if let Some(&args) = children.iter().find(|&&c| self.kind(c) == "arguments") {
                        let inner: Vec<u32> = self.children(args).into_iter().filter(|&c| !self.is_layout(c)).collect();
                        if inner.len() == 1 && self.kind(inner[0]) == "string" {
                            return Some(self.string_value(inner[0]));
                        }
                    }
                }
            }
        }
        None
    }

    /// `const x = ...;` without an initializer is an expression, as the lexer
    /// has it; with one it is an assignment.
    /// `require('x')` is an import, not a call: the lexer path agrees, and a
    /// resolver treats the module string, not a callee named `require`.
    pub(crate) fn require_module(&self, root: u32) -> Option<String> {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let kind = self.kind(node);
            // A `require` nested in an argument list still counts: the one-pass
            // lexer records `_interopRequireWildcard(require("react"))` as an
            // import of `react`, so refusing the nested form cost 276 JS files.
            if let Some(module) = self.direct_require_module(node) {
                return Some(module);
            }
            // Do not cross into nested scopes even inside an argument list:
            // `memoize(() => { const fs = require("graceful-fs"); })` is a
            // call to `memoize`, not a top-level import of `graceful-fs`.
            for child in self.children(node).into_iter().rev() {
                let ck = self.kind(child);
                if matches!(ck, "statement_block" | "block" | "class_body")
                    || self.kinds.definitions.iter().any(|def| def.kind == ck)
                {
                    continue;
                }
                stack.push(child);
            }
        }
        None
    }

    pub(crate) fn require_assignment(&self, root: u32) -> Option<(u32, String)> {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let kind = self.kind(node);
            if matches!(kind, "assignment" | "assignment_expression") {
                let children = self.children(node);
                if let (Some(&lhs), Some(&rhs)) = (children.first(), children.last()) {
                    if lhs != rhs && !self.kinds.definitions.iter().any(|def| def.kind == self.kind(rhs)) {
                        if let Some(module) = self.require_module(rhs) {
                            return Some((lhs, module));
                        }
                    }
                }
            }
            for child in self.children(node).into_iter().rev() {
                let ck = self.kind(child);
                if matches!(ck, "statement_block" | "block" | "class_body" | "arguments")
                    || self.kinds.definitions.iter().any(|def| def.kind == ck)
                {
                    continue;
                }
                stack.push(child);
            }
        }
        None
    }

    pub(crate) fn js_require_destructured(&mut self, pat: u32, module: &str, line: u32) {
        for child in self.children(pat) {
            match self.kind(child) {
                "shorthand_property_identifier_pattern" | "shorthand_property_identifier" | "identifier" => {
                    let name = self.text_string(child);
                    self.facts.imports.push(Import {
                        line,
                        module: module.into(),
                        name: name.clone(),
                        alias: name,
                        kind: IMPORT_MODULE,
                    });
                }
                "object_assignment_pattern" | "assignment_pattern" => {
                    if let Some(&first) = self.children(child).first() {
                        let name = self.text_string(first);
                        self.facts.imports.push(Import {
                            line,
                            module: module.into(),
                            name: name.clone(),
                            alias: name,
                            kind: IMPORT_MODULE,
                        });
                    }
                }
                "pair_pattern" | "pair" => {
                    let kids = self.children(child);
                    let Some(&first) = kids.first() else { continue };
                    let name = self.text_string(first);
                    let Some(&last) = kids.last().filter(|&&l| l != first) else { continue };
                    if matches!(self.kind(last), "object_pattern" | "object") {
                        self.facts.imports.push(Import {
                            line,
                            module: module.into(),
                            name,
                            alias: "{".into(),
                            kind: IMPORT_MODULE,
                        });
                        self.js_require_destructured(last, module, line);
                    } else {
                        let alias_node = if matches!(self.kind(last), "assignment_pattern" | "object_assignment_pattern") {
                            self.children(last).first().copied().unwrap_or(last)
                        } else {
                            last
                        };
                        let alias = self.text_string(alias_node);
                        self.facts.imports.push(Import {
                            line,
                            module: module.into(),
                            name,
                            alias,
                            kind: IMPORT_MODULE,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn declarator(&mut self, id: u32) {
        let children = self.children(id);
        let Some(&name_node) = children.first() else { return };
        let equal = children.iter().position(|&child| self.kind(child) == "=");
        let Some(equal) = equal else {
            let line = self.line(id);
            let names = self.pattern_names(name_node);
            if names.len() == 1 {
                let mut text = format!("[\"x\",{line},[\"n\",");
                push_string(&mut text, &names[0]);
                text.push_str("]]");
                self.event(&text);
            }
            return;
        };
        let Some(&value) = children.get(equal + 1) else { return };
        // `const f = () => {}`: the binding is a definition, not a value, and
        // the walk opens the symbol when it reaches the function node.
        if self.kinds.definitions.iter().any(|def| def.kind == self.kind(value)) {
            let names = self.pattern_names(name_node);
            if let Some(name) = names.into_iter().next() {
                self.bound.push((value, name));
            }
            return;
        }
        if self.kind(value) == "call_expression" {
            if let Some(callee) = self.children(value).into_iter().next() {
                // `const f = function() {}()` and `const f = function() {}.bind()`:
                // the function expression is the callee or its receiver, so it
                // takes the binding's name rather than `<anonymous>`.
                let func = if self.kind(callee) == "member_expression" {
                    self.children(callee).into_iter().next()
                } else {
                    Some(callee)
                };
                if let Some(func) = func.filter(|&node| self.kind(node) == "function_expression") {
                    if let Some(name) = self.pattern_names(name_node).into_iter().next() {
                        self.bound.push((func, name));
                    }
                }
            }
        }
        if let Some(module) = self.require_module(value) {
            // `const { a, b } = require('m')` takes named members; `const x =
            // require('m')` binds the module itself, so it has no member name.
            let line = self.line(id);
            if matches!(self.kind(name_node), "object_pattern" | "object") {
                self.js_require_destructured(name_node, &module, line);
                return;
            }
            let destructured = self.kind(name_node) == "array_pattern";
            let names = self.pattern_names(name_node);
            for name in names {
                let (member, alias) = if destructured { (name.clone(), name) } else { (String::new(), name) };
                self.facts
                    .imports
                    .push(Import { line, module: module.clone(), name: member, alias, kind: IMPORT_MODULE });
            }
            return;
        }
        let line = self.line(id);
        let targets = self.pattern_names(name_node);
        self.assignment_parts(targets, value, line, false);
    }

    /// `a.lineStartAngle = function () { … }`: an assignment whose value is a
    /// definition. The function node names itself from nothing — it is an
    /// expression — so the target is the only place its name exists, exactly as
    /// the `const f = () => {}` declarator above reads its own binding.
    pub(crate) fn assignment_definition(&mut self, id: u32) {
        let children = self.children(id);
        let equal = children.iter().position(|&child| self.kind(child) == "=");
        let Some(equal) = equal else { return };
        let Some(&value) = children.get(equal + 1) else { return };
        if !self.kinds.definitions.iter().any(|def| def.kind == self.kind(value)) {
            return;
        }
        let Some(&target) = children.first() else { return };
        let Some(name) = self.last_name(target) else { return };
        self.bound.push((value, name));
    }

    /// A class field: `#store = new Map()` writes on the enclosing symbol, the
    /// way the lexer treats a statement in a class body.
    pub(crate) fn field(&mut self, id: u32) {
        let children = self.children(id);
        // `static count = 0` has the modifier first: the name is the first child
        // that is not layout, not the first child outright. A decorator is also
        // not the name: `@dec\nfield: number = 1` opens with the decorator node,
        // and reading it as the binder wrote `a dec` on the decorator's line
        // while the member's own event vanished.
        let Some(&name_node) = children.iter().find(|&&child| {
            !self.is_layout(child)
                && !matches!(
                    self.kind(child),
                    "decorator" | "decorator_call_expression" | "readonly" | "accessor"
                )
        }) else {
            return
        };
        let Some(equal) = children.iter().position(|&child| self.kind(child) == "=") else { return };
        let Some(&value) = children.get(equal + 1) else { return };
        let targets = self.pattern_names(name_node);
        if self.kinds.definitions.iter().any(|def| def.kind == self.kind(value)) {
            if let Some(name) = targets.first() {
                // A private field holding a function keeps its `#`: the CST's
                // `private_property_identifier` node carries the bare name and
                // pattern_names strips the marker, so `class A { #m = () => 1 }`
                // opened the symbol `A.m` while the one-pass reader opened
                // `A.#m`. Only the definition's name is marked: a plain private
                // field writes its bare name on both sides.
                let name = if self.kind(name_node) == "private_property_identifier" {
                    format!("#{}", name)
                } else {
                    name.clone()
                };
                self.bound.push((value, name));
            }
        }
        self.assignment_parts(targets, value, self.line(id), false);
    }

    /// A `for` header's initializer. The grammar wraps a non-declaration init in
    /// an `expressions` node, so `for (i = length; …)` files its assignment one
    /// level down where a statement reader never looks.
    pub(crate) fn for_init(&mut self, id: u32) {
        if self.is_layout(id) {
            return;
        }
        match self.kind(id) {
            "assignment" | "assignment_expression" => self.assignment(id, false),
            "augmented_assignment" | "augmented_assignment_expression" | "compound_assignment_expr" => {
                self.assignment(id, true)
            }
            // `for (a = 1, b = 2; …)`: one node holds both assignments.
            "expressions" | "sequence_expression" | "expression_list" => {
                for item in self.children(id) {
                    self.for_init(item);
                }
            }
            _ => {
                let _ = self.form(id);
            }
        }
    }

    pub(crate) fn assignment(&mut self, id: u32, augmented: bool) {
        let children = self.children(id);
        let Some(&first) = children.first() else { return };
        let Some(&last) = children.last() else { return };
        if first == last {
            return;
        }
        let line = self.line(id);
        if std::env::var("RG_ASSIGN_DEBUG").is_ok() {
            let dump: Vec<String> = children.iter().map(|&c| format!("{}:{:?}", self.kind(c), self.text_string(c).chars().take(8).collect::<String>())).collect();
            eprintln!("assign: {:?}", dump);
        }
        // The left-hand side can itself carry calls (e.g. `c[f() ? 0 : 1] = v`);
        // `assignment_targets` only extracts the target name, so walk `first`
        // as an expression to record those calls in source order.
        let _ = self.form(first);
        let targets = self.assignment_targets(first);
        let weak = augmented || matches!(self.kind(first), "subscript" | "subscript_expression" | "index_expression");
        // `a = b = value` arrives flat: two `=` and the middle binder is an
        // ordinary child. The one-pass lexer reads the line left to right, so its
        // value names that binder as well; reading only the last child dropped
        // it and every chained assignment in the corpus fell back.
        let equals: Vec<usize> = children
            .iter()
            .enumerate()
            .filter(|(_, &child)| self.kind(child) == "=")
            .map(|(at, _)| at)
            .collect();
        // The chain arrives nested: `a = (b = value)` is two assignment nodes,
        // and the inner binder is the first child of the inner one.
        // `a = b = c = value` nests one assignment per binder, and the one-pass
        // lexer reads the line left to right into one flat merge. Walking the
        // chain to its end and naming every binder keeps the two forms equal
        // instead of nesting a merge inside a merge.
        let mut mids = Vec::new();
        let mut tail = last;
        loop {
            if !matches!(self.kind(tail), "assignment" | "assignment_expression") {
                break;
            }
            let inner = self.children(tail);
            let Some(&binder) = inner.first() else { break };
            if binder == tail {
                break;
            }
            // The binder itself is an expression: `a[i] = b[f(x)] = v` calls
            // `f` in the middle of the chain, and naming the binder alone left
            // that call unrecorded.
            let _ = self.form(binder);
            if let Some(name) = self.value_path(binder) {
                mids.push(name);
            }
            let Some(&next) = inner.last().filter(|&&child| child != binder) else { break };
            if next == tail {
                break;
            }
            tail = next;
        }
        let nested = tail != last;
        if equals.len() > 1 {
            for pair in equals.windows(2) {
                for &child in &children[pair[0] + 1..pair[1]] {
                    if let Some(name) = self.value_path(child) {
                        mids.push(name);
                    }
                }
            }
        }
        if !mids.is_empty() {
            {
                let expr = self.form(if nested { tail } else { last });
                let mut merged = String::from("[\"m\",[");
                for name in &mids {
                    merged.push_str("[\"n\",");
                    push_string(&mut merged, name);
                    merged.push_str("],");
                }
                merged.push_str(&expr);
                merged.push_str("]]");
                let mut list = String::from("[");
                for (index, name) in targets.iter().enumerate() {
                    if index > 0 {
                        list.push(0x2c as char);
                    }
                    push_string(&mut list, name);
                }
                list.push(']');
                self.event(&format!("[\"a\",{line},{list},{merged}]"));
                return;
            }
        }
        self.assignment_parts(targets, last, line, weak);
    }

    pub(crate) fn assignment_targets(&self, id: u32) -> Vec<String> {
        match self.kind(id) {
            "subscript" | "subscript_expression" | "index_expression" => {
                let base = self.children(id).first().copied().unwrap_or(id);
                self.assignment_targets(base)
            }
            "array" | "array_pattern" | "object" | "object_pattern" => self.pattern_names(id),
            "pattern_list" | "tuple_pattern" | "list_pattern" | "tuple" | "list"
            // `(a, b) = compute()` in Rust: the left side is a plain
            // `tuple_expression`, the same node a value uses, so a tuple
            // target bound only when the grammar called it a pattern. Without
            // it the event lost its targets and the lexer's `a ["a","b"]` had
            // no counterpart.
            | "tuple_expression" | "expression_list" => self.children(id).into_iter().flat_map(|child| self.unpacked_names(child)).collect(),
            // `(fd, path) = mkstemp()`: the Python grammar keeps a parenthesised
            // target list as a `parenthesized_expression` with one child per
            // name, so it fell to the path arm below and bound only the first.
            "parenthesized_expression" => {
                let inner: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                if inner.len() == 1 {
                    self.assignment_targets(inner[0])
                } else {
                    inner.into_iter().flat_map(|child| self.unpacked_names(child)).collect()
                }
            }
            // The left side of a Python multi-target assignment (`a, b = g(x),
            // 1`) is a `pattern_list` here, and this arm maps it, so the
            // `["a",2,["a","b"],...]` event the lexer writes is written here
            // too. The arm has always listed the kind, but the note it carried
            // said the mapping was left out because `pipeline::loses` compared
            // event *counts* and so could not see the value the CST cannot read
            // on such a file's f-string return (`["r",3,0]` where the lexer
            // follows `["r",3,["n","a"]]`); with a count-only guard, mapping the
            // target would have kept that record. That condition is now met:
            // the guard compares targets and value shapes, so the same file
            // falls back on the value it lost. `tests/guard_truth.rs` checks
            // both halves — the target kind is `pattern_list` and both names
            // reach the record — and the full suite is green with the mapping
            // on. The `expression_list` in the arm above is the *value* form,
            // read by `form`, and adds nothing to a target.
            // `x: Dict[str, int] = {}`: the target is the name inside the
            // annotation, not the annotation. Reading it keeps the event an
            // assignment (`["a",...]`) instead of a bare expression (`["x",...]`).
            "annotated" => self.children(id).first().and_then(|&first| self.value_path(first)).into_iter().collect(),
            // `*slot = slot.wrapping_add(value)`: the target is the value the
            // pointer refers to, so the name is the operand. Read as a bare
            // unary expression the assignment has no target at all and the
            // event cannot be paired with the lexer's.
            "unary_expression" | "unary" | "pointer_expression" => self
                .children(id)
                .into_iter()
                .filter(|&child| !self.is_layout(child) && self.kind(child) != "*")
                .find_map(|child| {
                    let targets = self.assignment_targets(child);
                    (!targets.is_empty()).then_some(targets)
                })
                .unwrap_or_default(),
            // `meter.exhausted = …`: a field path is a target like any other
            // path, and `value_path` does not read one.
            _ => match self.value_path(id) {
                Some(path) if !path.is_empty() => vec![path],
                _ => {
                    let path = self.path_of(id);
                    if path.is_empty() { Vec::new() } else { vec![path] }
                }
            },
        }
    }

    /// Every name of one element of an unpacking target list. `head, *rest =
    /// xs` binds `rest` too — the star is how the list is split, not a
    /// different kind of binder — and `typ, [data] = xs` binds both, so an
    /// element that is itself a group contributes every name it holds instead
    /// of one path its node does not have. Read through `value_path` alone the
    /// group named nothing and the statement's target list lost a binder.
    fn unpacked_names(&self, id: u32) -> Vec<String> {
        // The star is how the list is split, not a different kind of binder,
        // so `*rest` names `rest` and `*_` names `_`.
        if self.kind(id) == "list_splat" {
            return self.children(id).into_iter().filter_map(|child| self.value_path(child)).collect();
        }
        match self.kind(id) {
            "pattern_list"
            | "tuple_pattern"
            | "list_pattern"
            | "array_pattern"
            | "object_pattern"
            | "parenthesized_expression"
            | "tuple"
            | "list"
            | "array"
            | "object" => self.assignment_targets(id),
            _ => self.value_path(id).into_iter().collect(),
        }
    }

    pub(crate) fn assignment_event(&mut self, targets: Vec<String>, value: u32, line: u32, augmented: bool) {
        self.assignment_parts(targets, value, line, augmented);
    }

    /// `provider = lambda client="": dashboard.payload(...)` is a definition,
    /// not a value: the one-pass lexer opens a symbol for it, so the CST walk
    /// does too, and its body is read inside that symbol so the calls land on
    /// it rather than on the enclosing function.
    pub(crate) fn bound_lambda(&mut self, name: &str, node: u32, line: u32) -> bool {
        if self.kind(node) != "lambda" {
            return false;
        }
        let owner = if self.frame().container { self.frame().class.clone() } else { String::new() };
        let params = self
            .child_of_kind(node, "lambda_parameters")
            .map(|list| self.params(node, list))
            .unwrap_or_default();
        let index = self.add_symbol(name.to_string(), owner.clone(), KIND_FUNCTION, line, params);
        let body = self.children(node).into_iter().rev().find(|&child| !self.is_layout(child));
        self.stack.push(Frame { node, symbol: index, class: owner, receiver: String::new(), container: false, defines: Some(index) });
        if let Some(body) = body {
            if !matches!(self.kind(body), "block" | "statement_block") {
                let at = self.line(body);
                let value = self.form(body);
                if value != "0" {
                    self.event(&format!("[\"r\",{at},{value}]"));
                }
            }
            self.facts.symbols[index].end_line = self.end_line(body);
        }
        self.stack.pop();
        true
    }

    pub(crate) fn assignment_parts(&mut self, targets: Vec<String>, value: u32, line: u32, augmented: bool) {
        if !augmented && targets.len() == 1 && self.bound_lambda(&targets[0].clone(), value, line) {
            return;
        }
        // `module.exports = function (v) { … }`: the value is a definition the
        // walk opens on its own, so its name comes from the assignment's target,
        // exactly as a declarator's does. The one-pass lexer names the same
        // symbol from the same target, so without this the two records disagree
        // on the name of a function they both see.
        if !augmented && self.is_js() && targets.len() == 1 {
            // The name is the assignment's target path as written — `module.exports`
            // stays `module.exports`, not `exports`. Taking the last segment made
            // two different bindings share one symbol id (`exports`, then another
            // `exports`) and merged their flows; the corpus went 863 -> 1144.
            let target = targets[0].clone();
            if self.kinds.definitions.iter().any(|def| def.kind == self.kind(value)) {
                self.bound.push((value, target));
                return;
            }
        }
        let weak = augmented || targets.is_empty();
        let expr = self.form(value);
        if targets.is_empty() {
            if expr != "0" {
                self.event(&format!("[\"x\",{line},{expr}]"));
            }
            return;
        }
        let mut text = format!("[\"a\",{line},[");
        for (index, name) in targets.iter().enumerate() {
            if index > 0 {
                text.push(',');
            }
            push_string(&mut text, name);
        }
        text.push_str(&format!("],{expr},{}]", u8::from(weak)));
        self.event(&text);
        if let Some(class) = self.constructed(value) {
            for target in &targets {
                self.facts.vars.push((target.clone(), class.clone()));
            }
        } else {
            // `s, t = Counter(a), Counter(b)`: one value per target, so the
            // class a target holds is its own value's. Read as a tuple the
            // whole value constructs nothing and every binding was dropped,
            // while the lexer names the last one.
            let items: Vec<u32> = match self.kind(value) {
                "expression_list" | "tuple" | "tuple_expression" | "pattern_list" => {
                    self.children(value).into_iter().filter(|&child| !self.is_layout(child)).collect()
                }
                _ => Vec::new(),
            };
            if items.len() == targets.len() {
                for (target, item) in targets.iter().zip(items) {
                    if let Some(class) = self.constructed(item) {
                        self.facts.vars.push((target.clone(), class));
                    }
                }
            }
        }
    }

    /// The class a call constructs, by the same convention the lexer uses: a
    /// capitalised callee, or anything after `new`.
    pub(crate) fn constructed(&self, value: u32) -> Option<String> {
        // `Path(__file__).resolve().parent`: the value is an attribute chain
        // whose base is the call. The lexer reads the first token, so the
        // chain is unwrapped to the call that starts it.
        let call = self.value_root_call(value)?;
        if self.kind(call) == "new_expression" {
            let path = self.children(call).into_iter().find_map(|child| {
                matches!(self.kind(child), "identifier" | "member_expression" | "attribute").then(|| self.path_of(child))
            })?;
            let last = path.rsplit('.').next().unwrap_or(&path);
            return last.starts_with(|ch: char| ch.is_uppercase()).then_some(last.to_string());
        }
        let path = self.root_callee(call)?;
        let last = path.rsplit('.').next().unwrap_or("").to_string();
        (last.starts_with(|ch: char| ch.is_uppercase()) && !last.contains('(')).then_some(last)
    }

    /// The call a value starts with: `a().b` -> the `a()` node, `a.b()` ->
    /// `a.b()`, `(a)` -> the call in the parentheses.
    pub(crate) fn value_root_call(&self, id: u32) -> Option<u32> {
        match self.kind(id) {
            "call" | "call_expression" | "new_expression" => Some(id),
            "attribute" | "member_expression" | "subscript" | "subscript_expression" | "parenthesized_expression" => {
                self.children(id).first().and_then(|&object| self.value_root_call(object))
            }
            // `candidate = Path(x).parent / candidate`: a binary value starts
            // with its left operand, which is what the lexer's first token is.
            "binary_operator" | "binary_expression" | "boolean_operator" | "conditional_expression"
            | "ternary_expression" => self.children(id).first().and_then(|&left| self.value_root_call(left)),
            // `new Foo() as any`, `new Foo() satisfies T`, `new Foo()!`: a type
            // assertion or non-null postfix does not change the constructed class.
            "as_expression" | "satisfies_expression" | "non_null_expression" => {
                self.expression_child(id).and_then(|expr| self.value_root_call(expr))
            }
            _ => None,
        }
    }

    /// The callee at the root of a call chain: `a.b().c()` -> `a.b`, `f()()` ->
    /// `f`, `a.b()` -> `a.b`.
    pub(crate) fn root_callee(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "call" | "call_expression" | "new_expression" => {
                let callee = self.children(id).into_iter().find(|&child| {
                    !matches!(self.kind(child), "new" | "argument_list" | "arguments" | "type_arguments" | "(" | ")")
                })?;
                (callee != id).then(|| self.root_callee(callee)).flatten()
            }
            "attribute" | "member_expression" | "navigation_expression" => {
                let object = self.children(id).first().copied()?;
                let path = self.path_of(id);
                // `Config.Array(...).pipe` starts with a call: unwrap to that
                // call so the class is the one the lexer's first tokens name.
                // Otherwise the whole path is the answer: `Context.Reference`
                // constructs Reference, not Context, and returning the object
                // root reported every namespaced constructor as a lost var.
                if path.contains("()") || matches!(self.kind(object), "call" | "call_expression" | "new_expression") {
                    return self.root_callee(object);
                }
                (!path.is_empty()).then_some(path)
            }
            _ => {
                let path = self.path_of(id);
                (!path.is_empty()).then_some(path)
            }
        }
    }

    // -- control flow -------------------------------------------------------

    pub(crate) fn condition_before(&mut self, id: u32) {
        // `if x:` / `while x:` / `async with x:`: the first child that is not a
        // keyword or punctuation is the test.
        for child in self.children(id) {
            if self.is_layout(child) {
                continue;
            }
            if matches!(self.kind(child), "block" | "statement_block" | "else_clause" | "elif_clause" | "catch_clause" | "finally_clause") {
                return;
            }
            let value = self.form(child);
            if value != "0" {
                let mut expr = child;
                while self.kind(expr) == "parenthesized_expression" {
                    match self.expression_child(expr) {
                        Some(inner) => expr = inner,
                        None => break,
                    }
                }
                let line = self.line(expr);
                self.event(&format!("[\"x\",{line},{value}]"));
            }
            return;
        }
    }

    /// `for a, b in xs` / `for (const a of xs)` -> targets and iterable.
    pub(crate) fn loop_binding(&mut self, id: u32) -> Option<(Vec<String>, u32)> {
        let children = self.children(id);
        // Python writes `with open(p) as f:` and `except E as error:` with the
        // `as` inside an `as_pattern` of its own, so the scan below never saw
        // it and the name the statement binds had no event at all.
        if let Some(pattern) = children.iter().copied().find(|&child| self.kind(child) == "as_pattern") {
            let inner = self.children(pattern);
            let at = inner.iter().position(|&child| self.kind(child) == "as")?;
            let value = *inner[..at].iter().rev().find(|&&child| !self.is_layout(child))?;
            let mut targets = Vec::new();
            for &child in &children {
                if self.kind(child) == "as_pattern" {
                    let parts = self.children(child);
                    if let Some(as_at) = parts.iter().position(|&part| self.kind(part) == "as") {
                        if let Some(&target) = parts.get(as_at + 1) {
                            targets.extend(self.pattern_names(target));
                        }
                    }
                }
            }
            return Some((targets, value));
        }
        let mut split = None;
        for (index, &child) in children.iter().enumerate() {
            if matches!(self.kind(child), "in" | "of" | "as") {
                split = Some(index);
                break;
            }
        }
        // JS keeps the header inside `for_header`: `for (let x of y)`.
        if split.is_none() {
            let header = children.iter().copied().find(|&child| self.kind(child) == "for_header")?;
            let inner = self.children(header);
            let at = inner.iter().position(|&child| matches!(self.kind(child), "in" | "of"))?;
            let targets = inner[..at]
                .iter()
                .filter(|&&child| !matches!(self.kind(child), "(" | "let" | "const" | "var" | ";" | ","))
                .flat_map(|&child| {
                    if self.kind(child) == "variable_declarator" {
                        self.children(child).first().copied().map(|first| self.pattern_names(first)).unwrap_or_default()
                    } else {
                        self.pattern_names(child)
                    }
                })
                .collect();
            let value = *inner.get(at + 1)?;
            return Some((targets, value));
        }
        let at = split?;
        if self.kind(children[at]) == "as" {
            // `with open(p) as handle`: the value is before `as`, the binding
            // after it.
            let value = *children[..at]
                .iter()
                .rev()
                .find(|&&child| !self.is_layout(child) && self.kind(child) != "async")?;
            let targets: Vec<String> = children.get(at + 1).map(|&child| self.pattern_names(child)).unwrap_or_default();
            return Some((targets, value));
        }
        let targets = children[..at]
            .iter()
            .filter(|&&child| !matches!(self.kind(child), "for" | "async" | "(" | ")"))
            .flat_map(|&child| self.pattern_names(child))
            .collect();
        let value = *children.get(at + 1)?;
        Some((targets, value))
    }

    // -- imports ------------------------------------------------------------

    pub(crate) fn import(&mut self, id: u32) {
        let kind = self.kind(id);
        match kind {
            "import_declaration" if self.kinds.family == Family::Go => self.go_import(id),
            "import_declaration" if self.kinds.family == Family::Java => self.java_import(id),
            // C# writes `using System.Collections.Generic;`, `using static
            // System.Math;` and `using Alias = Some.Type;`. Every one of them
            // was invisible: nothing matched `using_directive`, so a corpus of
            // 22k files lost its imports on almost every file.
            "using_directive" | "global_using_directive" => self.cs_using(id),
            // `import org.gradle.api.Project` and `import io.ktor.http as kt`:
            // the Kotlin grammar gives every one its own node, nothing matched
            // it, and 2539 files of the kotlin corpus lost their import - most
            // of them lost nothing else.
            "import_header" => self.kt_import(id),
            "package_header" => self.kt_package(id),
            // Scala and Swift spell an import as a flat run of identifiers
            // (`import java.io.Serializable`) or with selectors
            // (`import a.b.{C, D}`), and neither had an arm: 147 files of the
            // scala corpus lost every import.
            "import_declaration" if self.kinds.family == Family::Plain => self.scala_import(id),
            "import_statement" | "import_from_statement" => {
                if self.is_js() {
                    self.js_import(id);
                } else {
                    self.python_import(id);
                }
            }
            "export_statement" => {
                // `export ... from "x"` re-exports; a plain `export` holds a
                // declaration, which the walk reaches on its own; `export
                // default <expr>` is the expression.
                if self.child_of_kind(id, "string").is_some() {
                    self.js_import(id);
                } else if !self.children(id).into_iter().any(|child| {
                    matches!(
                        self.kind(child),
                        "function_declaration" | "class_declaration" | "lexical_declaration" | "variable_declaration"
                            | "generator_function_declaration" | "type_alias_declaration" | "interface_declaration"
                    )
                }) {
                    self.expression_statement(id);
                }
            }
            _ => {}
        }
    }

    /// `import ("fmt"; str "strings")`: one import per spec, the module as
    /// written and the alias the spec binds — an explicit name, or the last
    /// path segment. Go gives all of them the import clause's line, as the
    /// lexer does, because the clause is the statement.
    pub(crate) fn go_import(&mut self, id: u32) {
        let line = self.line(id);
        let mut specs = Vec::new();
        let mut walk = vec![id];
        while let Some(node) = walk.pop() {
            for child in self.children(node) {
                if self.kind(child) == "import_spec" {
                    specs.push(child);
                } else if matches!(self.kind(child), "import_spec_list") {
                    walk.push(child);
                }
            }
        }
        for spec in specs {
            let children = self.children(spec);
            let Some(string) = children
                .iter()
                .copied()
                .find(|&child| matches!(self.kind(child), "interpreted_string_literal" | "raw_string_literal"))
            else {
                continue;
            };
            let module = self.string_value(string);
            let alias = children
                .iter()
                .copied()
                .find(|&child| self.kind(child) == "identifier")
                .map(|child| self.text_string(child))
                .unwrap_or_else(|| module.rsplit('/').next().unwrap_or(&module).to_string());
            self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
        }
    }

    /// `using System.Collections.Generic;` and its two variants.
    ///
    /// The lexer reads the same line and files the last segment as the alias
    /// when no `as`-style name is spelled, so the alias is written here the same
    /// way: the records have to mean the same thing for the guard to keep the
    /// tree.
    pub(crate) fn cs_using(&mut self, id: u32) {
        let line = self.line(id);
        let raw = self.text_string(id);
        let text = raw.trim().trim_end_matches(';').trim();
        let text = text.strip_prefix("global ").unwrap_or(text).trim();
        let text = text.strip_prefix("using ").unwrap_or(text).trim();
        let text = text.strip_prefix("static ").unwrap_or(text).trim();
        let (alias, module) = match text.split_once('=') {
            Some((alias, module)) => (alias.trim().to_string(), module.trim().to_string()),
            None => (String::new(), text.to_string()),
        };
        if module.is_empty() {
            return;
        }
        let alias = if alias.is_empty() {
            module.rsplit('.').next().unwrap_or("").to_string()
        } else {
            alias
        };
        self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
    }

    /// `import java.util.List;` binds the last segment; `import java.io.*;`
    /// binds the package, which is a star import with no local name.

    pub(crate) fn python_import(&mut self, id: u32) {
        let line = self.line(id);
        let children = self.children(id);
        if self.kind(id) == "import_from_statement" {
            // `from <module> import <bindings>`. The module may be relative
            // (`.`, `..pkg.mod`) and the bindings may sit in a parenthesised
            // `import_list`, which is one list of names, not a wrapper to skip.
            let module_at = children
                .iter()
                .position(|&child| matches!(self.kind(child), "dotted_name" | "identifier" | "relative_import"));
            let module = module_at.map(|at| self.text_string(children[at])).unwrap_or_default();
            let bindings = module_at.map(|at| &children[at + 1..]).unwrap_or(&children[..]);
            self.import_bindings(bindings, &module, line);
            return;
        }
        // `import a.b as c, d`: each clause is a module and an optional alias.
        // The alias is a name of its own, not the module of the next clause.
        let mut index = 0;
        while index < children.len() {
            let child = children[index];
            let aliased = index > 0 && self.kind(children[index - 1]) == "as";
            if !aliased && matches!(self.kind(child), "dotted_name" | "identifier") {
                let module = self.text_string(child);
                let alias = children
                    .get(index + 2)
                    .filter(|_| children.get(index + 1).is_some_and(|&n| self.kind(n) == "as"))
                    .map(|&n| self.text_string(n))
                    .unwrap_or_else(|| module.clone());
                self.facts.imports.push(Import { line, module, alias, kind: IMPORT_MODULE, ..Import::default() });
            }
            index += 1;
        }
    }

    /// `import java.io.Serializable`, `import scala.concurrent.duration._`,
    /// `import a.b.{C, D}`: one import per name bound, with the path as
    /// written, the member when a selector names one, and the local name.
    pub(crate) fn scala_import(&mut self, id: u32) {
        let line = self.line(id);
        let mut parts: Vec<String> = Vec::new();
        let mut selectors: Vec<u32> = Vec::new();
        for child in self.children(id) {
            match self.kind(child) {
                "import" => {}
                "import_selectors" => selectors = self.children(child),
                "identifier" | "simple_identifier" | "type_identifier" | "_" | "wildcard" => {
                    parts.push(self.text_string(child))
                }
                _ => {}
            }
        }
        if selectors.is_empty() {
            if parts.is_empty() {
                return;
            }
            let alias = parts.last().cloned().unwrap_or_default();
            self.facts.imports.push(Import {
                line,
                module: parts.join("."),
                name: String::new(),
                alias,
                kind: IMPORT_MODULE,
            });
            return;
        }
        let module = parts.join(".");
        for selector in selectors {
            if self.is_layout(selector) || matches!(self.kind(selector), "import_selectors" | "," | "{" | "}") {
                continue;
            }
            // `{Failure => Bad}`: one selector node carrying both names. Emitted as
            // two imports before, which is not what the source states and left every
            // Scala rename disagreeing with the lexer.
            if self.kind(selector) == "import_selector" {
                let names: Vec<String> = self
                    .children(selector)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && self.kind(child) != "=>")
                    .map(|child| self.text_string(child))
                    .collect();
                if let Some(name) = names.first().cloned() {
                    let alias = names.get(1).cloned().unwrap_or_else(|| name.clone());
                    self.facts.imports.push(Import {
                        line,
                        module: module.clone(),
                        name,
                        alias,
                        kind: IMPORT_MODULE,
                    });
                    continue;
                }
            }
            let name = self.text_string(selector);
            self.facts.imports.push(Import {
                line,
                module: module.clone(),
                name: name.clone(),
                alias: name,
                kind: IMPORT_MODULE,
            });
        }
    }

    /// `import a.b.C` and `import a.b.C as D`: one module, and the alias the
    /// lexer files — the name after `as`, or the last segment when none is
    /// spelled.
/// One segment of a dotted name: letters, digits, `_`, and anything non-ASCII
/// (an identifier in the source's own language). A node whose text fails this is
/// not a segment — an argument list, a star, an `as` clause.
fn dotted_piece(text: &str) -> bool {
    if text.starts_with('`') && text.ends_with('`') && text.len() > 2 {
        return true;
    }
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_' || !c.is_ascii())
}

    pub(crate) fn kt_import(&mut self, id: u32) {
        let line = self.line(id);
        let mut parts: Vec<String> = Vec::new();
        let mut alias: Option<String> = None;
        let mut after_as = false;
        for child in self.children(id) {
            let kind = self.kind(child);
            if kind == "as" {
                after_as = true;
                continue;
            }
            // A path segment can be a contextual keyword: Kotlin's grammar names
            // 39 of them (`annotation`, `field`, `value`, …), and
            // `import java.lang.annotation.Inherited` is a real import whose middle
            // segment is one. Filtering on `identifier` dropped it, so the record
            // said `java.lang.Inherited` — 3,882 rows of the kotlin corpus, the
            // single largest loss class in the language.
            let text = self.text_string(child);
            let piece = text.trim();
            // The directive's own keyword is a child of the header and is not a
            // segment; every other word here is one.
            if piece == "import" || !Self::dotted_piece(piece) {
                continue;
            }
            if after_as {
                alias = Some(piece.to_string());
            } else {
                parts.push(piece.to_string());
            }
        }
        if parts.is_empty() {
            return;
        }
        let module = parts.join(".");
        // `import a.b.*`: a star import binds no name, and the lexer files it
        // with an empty alias rather than the last segment.
        let star = self.text_string(id).trim_end().ends_with('*');
        let alias = alias.unwrap_or_else(|| if star { String::new() } else { parts.last().cloned().unwrap_or_default() });
        let kind = if star { IMPORT_STAR } else { IMPORT_MODULE };
        self.facts.imports.push(Import { line, module, alias, kind, ..Import::default() });
    }

    /// The statements inside a Kotlin lambda, read one at a time: the body is a
    /// list of statements, not an expression, and reading it as a value turned
    /// `repositories { … }` into an anonymous call.
    pub(crate) fn kt_lambda_statements(&mut self, id: u32) {
        for child in self.children(id) {
            match self.kind(child) {
                "lambda_body" | "lambda_items" | "statements" => self.kt_lambda_statements(child),
                "expression_statement" => self.expression_statement(child),
                "script_statement" => self.kt_script_statement(child),
                "property_declaration" => self.kt_property(child),
                "call_expression" => {
                    self.form(child);
                }
                _ => {}
            }
        }
    }

    /// `rootProject.name = "demo"`, `group = "com.x"`: the flat statement a
    /// Kotlin script keeps, written as the assignment the lexer files.
    pub(crate) fn kt_script_statement(&mut self, id: u32) {
        let line = self.line(id);
        let mut names: Vec<String> = Vec::new();
        let mut value: Option<u32> = None;
        let mut arguments: Option<u32> = None;
        let mut recovered = false;
        for child in self.children(id) {
            let kind = self.kind(child);
            if kind == "ERROR" || kind.starts_with("ERROR") || kind.contains("MISSING") {
                recovered = true;
            }
            if value.is_none() && arguments.is_none() && matches!(kind, "identifier" | "simple_identifier" | "file") {
                names.push(self.text_string(child));
                continue;
            }
            if self.is_layout(child) || kind == "=" {
                continue;
            }
            if kind == "arguments" || kind == "argument_list" {
                arguments = Some(child);
                continue;
            }
            value = Some(child);
        }
        if names.is_empty() {
            return;
        }
        // `include(":shared")`, `apply(plugin = "x")`: a script line that is a
        // head and its argument list, nothing else, is a call — and the one-pass
        // reader records it as one, but the arm below only wrote its value, so
        // every Gradle settings file lost the `include` call to the guard (60 of
        // 72 fallbacks on a .kts sample, the largest `call … line 1` cluster in
        // the corpus). Only the plain shape counts: a line with a value of its
        // own (`… = this == null || isEmpty()`) or one the grammar recovered
        // (`RoutingContext.()` after a broken typealias) keeps the arm below,
        // which walks that value and records the calls inside it.
        if let Some(args) = arguments.filter(|_| value.is_none() && !recovered) {
            let path = names.join(".");
            let index = self.push_call(path, line);
            let receiver = if names.len() > 1 {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &names[..names.len() - 1].join("."));
                out.push(']');
                out
            } else {
                "0".to_string()
            };
            // The event is the call itself, not an assignment: the one-pass
            // reader files `include(":x")` as `c(0;[0])` on that line, so a
            // merge of the name and the argument value paired with nothing.
            let args_form = self.arguments(args);
            self.event(&format!("[\"x\",{line},[\"c\",{index},{receiver},{args_form}]]"));
            return;
        }
        match value {
            // `tasks.test { … }`: a trailing lambda is not a value, and the
            // lexer files the line as the name alone. The lambda's own body is
            // walked here because this arm claims the node.
            Some(node) if matches!(self.kind(node), "lambda_literal" | "annotated_lambda" | "lambda_expression") => {
                self.kt_lambda_statements(node);
                let mut text = "[[\"n\",".to_string();
                push_string(&mut text, &names.join("."));
                text.push_str("]]");
                self.event(&format!("[\"x\",{line},[\"m\",{text}]]"));
            }
            Some(node) => self.assignment_parts(vec![names.join(".")], node, line, false),
            None => {
                let mut text = "[[\"n\",".to_string();
                push_string(&mut text, &names.join("."));
                text.push_str("]]");
                let value = format!("[\"m\",{text}]");
                self.event(&format!("[\"x\",{line},{value}]"));
            }
        }
    }

    /// `true` when the type after the `:` carries a `?` anywhere. The lexer's
    /// Kotlin statement reader stops the type at that `?`, so the property is
    /// left as a declaration (`x`) rather than the assignment (`a`) an untyped
    /// or non-nullable property gets.
    fn kt_type_nullable(&self, id: u32) -> bool {
        if self.kind(id) == "?" {
            return true;
        }
        self.children(id).into_iter().any(|child| self.kt_type_nullable(child))
    }

    /// The identifier leaves of a type, in source order, as names:
    /// `List<String>` is `n:List` then `n:String`, the run of names the lexer
    /// reads in the type.
    fn kt_type_names(&self, id: u32, parts: &mut Vec<String>) {
        if self.node(id).first_child == NONE {
            if self.kind(id) == "identifier" {
                let mut out = String::from("[\"n\",");
                push_string(&mut out, &self.text_string(id));
                out.push(']');
                parts.push(out);
            }
            return;
        }
        for child in self.children(id) {
            self.kt_type_names(child, parts);
        }
    }

    /// `var x: List<String>? = null`: the modifiers, the name, the type's names
    /// and a name or call on the right of the `=`, in that order, as the one
    /// declaration event the lexer files for the statement.
    fn kt_nullable_property(&mut self, id: u32, name: &str, ty: u32, line: u32) {
        let mut parts: Vec<String> = Vec::new();
        for child in self.children(id) {
            match self.kind(child) {
                "modifiers" => {
                    for modifier in self.children(child) {
                        if self.kind(modifier) == "annotation" {
                            // An annotation above the declaration is its own
                            // event; only the ones written on the head's line
                            // belong to the run of words the lexer reads.
                            if self.line(modifier) < line {
                                continue;
                            }
                            let word =
                                self.children(modifier).into_iter().find(|&leaf| self.kind(leaf) == "identifier");
                            if let Some(word) = word {
                                let mut out = String::from("[\"n\",");
                                push_string(&mut out, &self.text_string(word));
                                out.push(']');
                                parts.push(out);
                            }
                            continue;
                        }
                        let value = self.decl_value(modifier);
                        if !value.is_empty() && value != "0" {
                            parts.push(value);
                        }
                    }
                }
                // `val` is a name to the one-pass reader and `var` is not:
                // `val x: Int` files `m:[n:val,n:x,n:Int]` while
                // `private var y: String` files `m:[n:private,n:y,n:String]`.
                "val" => {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, "val");
                    out.push(']');
                    parts.push(out);
                }
                "var" | ":" | "=" | "by" | "lateinit" | "const" => {}
                "property_head" => {
                    let mut out = String::from("[\"n\",");
                    push_string(&mut out, name);
                    out.push(']');
                    parts.push(out);
                }
                _ if child == ty => self.kt_type_names(ty, &mut parts),
                _ => {
                    let value = self.decl_value(child);
                    if !value.is_empty() && value != "0" {
                        parts.push(value);
                    }
                }
            }
        }
        if parts.is_empty() {
            return;
        }
        let value = match parts.len() {
            1 => parts.pop().unwrap_or_default(),
            _ => format!("[\"m\",[{}]]", parts.join(",")),
        };
        self.lexer_event(b'x', line, &[], &value, false);
    }

    /// `val a = 1`, `private val b: Int = 2`: the declared name and the value,
    /// as the assignment the lexer files.
    pub(crate) fn kt_property(&mut self, id: u32) {
        let children = self.children(id);
        // `@Deprecated(…)` written on its own line is an event of its own - the
        // lexer files it as the annotation's call - and the declaration starts
        // on the line of its head. Measured from the node's start, every
        // annotated property claimed the annotation's line and lost the
        // declaration the lexer writes under the head.
        let line = children
            .iter()
            .copied()
            .find(|&child| matches!(self.kind(child), "property_head" | "val" | "var"))
            .map(|child| self.line(child))
            .unwrap_or_else(|| self.line(id));
        let name = children
            .iter()
            .copied()
            .find(|&child| self.kind(child) == "property_head")
            .and_then(|head| {
                // A property named with one of Kotlin's soft keywords
                // (`val value = …`, `val data = …`) carries that keyword as its
                // head's only child, not an `identifier`. A property declared on
                // an extension receiver (`val ApplicationRequest.uri`) writes
                // the receiver first, so the name is the last segment.
                self.children(head)
                    .into_iter()
                    // `val file = …`: the Kotlin grammar keeps `file` as a
                    // keyword token (a use-site target and a script head), but
                    // it is a legal property name. Without it a property named
                    // `file` had no head, so its initialiser's call never
                    // reached the record (MavenVerifierExtensions.kt).
                    .filter(|&child| {
                        matches!(self.kind(child), "identifier" | "file") || self.kinds.soft.contains(&self.kind(child))
                    })
                    .next_back()
            })
            .map(|node| self.text_string(node))
            .or_else(|| {
                self.children(id)
                    .into_iter()
                    .find(|&child| matches!(self.kind(child), "identifier" | "simple_identifier" | "file"))
                    .map(|node| self.text_string(node))
            });
        let Some(name) = name else { return };
        // The initialiser is the child after `=` (or `by`), and nothing else.
        // Taking "the last child that is not a type word" made the type of a
        // property that has no initialiser (`val x: String`) read as its
        // value, so the line was written as an assignment the lexer never
        // makes instead of the declaration header it does.
        let value = if self.is_kotlin() {
            children
                .iter()
                .position(|&child| matches!(self.kind(child), "=" | "by"))
                .and_then(|at| children.get(at + 1).copied())
        } else {
            children
                .iter()
                .copied()
                .rev()
                .find(|&child| {
                    !matches!(
                        self.kind(child),
                        "val" | "var" | "property_head" | "modifiers" | ":" | "=" | "by" | "const" | "lateinit" | "override"
                            | "private" | "public" | "internal" | "protected" | "type_identifier" | "user_type" | "nullable_type"
                            | "function_type" | "generic_type" | "unsafe_cast" | "anonymous_function"
                    )
                })
        };
        let Some(value) = value else {
            // `public val log: Logger`: a typed property with no initialiser is
            // a declaration whose value is the run of names it read - the
            // modifiers, `val`, the name and the type's names. Read as an
            // assignment it wrote nothing at all, and every uninitialised
            // property in the corpus lost its line.
            if self.is_kotlin() {
                let ty = children
                    .iter()
                    .position(|&child| self.kind(child) == ":")
                    .and_then(|at| children.get(at + 1).copied())
                    .filter(|&child| !self.is_layout(child));
                let attached = children.iter().any(|&child| matches!(self.kind(child), "by" | "accessor"));
                if let (Some(ty), false) = (ty, attached) {
                    self.kt_nullable_property(id, &name, ty, line);
                    return;
                }
                // `val x: String` with its getter on the next line: there is no
                // initialiser to assign, so the lexer's header event has no
                // counterpart in the record and is written as one.
                self.java_header_event(id);
            }
            return;
        };
        // `val ApplicationRequest.uri: String get() = origin.uri`: the getter is
        // written on the declaration's line, and its expression is the value the
        // lexer files for the property.
        let value = if self.kind(value) == "accessor" {
            let inner = self.children(value);
            match inner.iter().copied().rev().find(|&child| {
                !self.is_layout(child)
                    && !matches!(self.kind(child), "get" | "set" | "(" | ")" | "=" | "block" | "accessor_parameters")
            }) {
                Some(node) => node,
                None => return,
            }
        } else {
            value
        };
        self.assignment_parts(vec![name], value, line, false);
    }

    /// `package a.b`: the lexer files the clause as one event whose value is the
    /// keyword and the dotted name; nothing wrote it, and a Kotlin file with a
    /// package clause lost the first line of most of them.
    pub(crate) fn kt_package(&mut self, id: u32) {
        let line = self.line(id);
        let mut parts: Vec<String> = Vec::new();
        for child in self.children(id) {
            let kind = self.kind(child);
            if matches!(kind, "identifier") || self.kinds.soft.contains(&kind) {
                parts.push(self.text_string(child));
            }
        }
        if parts.is_empty() {
            return;
        }
        let mut text = format!("[\"x\",{line},[\"m\",[[\"n\",\"package\"],[\"n\",");
        push_string(&mut text, &parts.join("."));
        text.push_str("]]]]");
        self.event(&text);
    }

    /// The bindings of a `from ... import` clause, in order. `from m import
    /// (a, b as c)`, `from m import *` and `from m import a, b` all land here.
    pub(crate) fn import_bindings(&mut self, children: &[u32], module: &str, line: u32) {
        let mut index = 0;
        while index < children.len() {
            let child = children[index];
            match self.kind(child) {
                "import_list" => {
                    let inner = self.children(child);
                    self.import_bindings(&inner, &module, line);
                }
                "wildcard_import" | "*" => {
                    self.facts.imports.push(Import { line, module: module.into(), kind: IMPORT_STAR, ..Import::default() });
                }
                "aliased_import" => {
                    let inner = self.children(child);
                    let Some(&name_node) = inner.first() else {
                        index += 1;
                        continue;
                    };
                    let name = self.text_string(name_node);
                    let alias = inner
                        .last()
                        .filter(|&&last| last != name_node)
                        .map(|&last| self.text_string(last))
                        .unwrap_or_else(|| name.clone());
                    self.facts.imports.push(Import { line, module: module.into(), name, alias, kind: IMPORT_MODULE });
                }
                "dotted_name" | "identifier" => {
                    let name = self.text_string(child);
                    let aliased = children.get(index + 1).is_some_and(|&next| self.kind(next) == "as");
                    let alias = if aliased {
                        children.get(index + 2).map(|&next| self.text_string(next)).unwrap_or_else(|| name.clone())
                    } else {
                        name.clone()
                    };
                    self.facts.imports.push(Import { line, module: module.into(), name, alias, kind: IMPORT_MODULE });
                    if aliased {
                        index += 2;
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    pub(crate) fn js_import(&mut self, id: u32) {
        let line = self.line(id);
        let children = self.children(id);
        let module = children
            .iter()
            .find(|&&child| self.kind(child) == "string")
            .map(|&child| self.string_value(child))
            .unwrap_or_default();
        if module.is_empty() {
            if let Some(&alias_node) = children.iter().find(|&&c| self.kind(c) == "import_alias") {
                let inner = self.children(alias_node);
                if let Some(&name_node) = inner.first() {
                    if let Some(req_mod) = self.require_module(alias_node) {
                        let alias = self.text_string(name_node);
                        self.facts.imports.push(Import {
                            line,
                            module: req_mod,
                            alias,
                            kind: IMPORT_MODULE,
                            ..Import::default()
                        });
                    }
                }
            }
            return;
        }
        if let Some(star_at) = children.iter().position(|&child| self.kind(child) == "*") {
            let alias = if children.get(star_at + 1).is_some_and(|&child| self.kind(child) == "as") {
                children.get(star_at + 2).map(|&child| self.text_string(child)).unwrap_or_default()
            } else {
                String::new()
            };
            let kind = if alias.is_empty() { IMPORT_STAR } else { IMPORT_MODULE };
            self.facts.imports.push(Import { line, module, alias, kind, ..Import::default() });
            return;
        }
        for child in children {
            match self.kind(child) {
                "import_clause" | "export_clause" => self.js_bindings(child, &module, line),
                "import_specifier" | "export_specifier" => self.js_specifier(child, &module, line),
                "string" | "import" | "export" | "from" | ";" => {}
                _ => self.js_bindings(child, &module, line),
            }
        }
    }

    pub(crate) fn js_bindings(&mut self, id: u32, module: &str, line: u32) {
        for child in self.children(id) {
            match self.kind(child) {
                "identifier" | "default" | "type" => {
                    let alias = self.text_string(child);
                    if alias != "type" {
                        self.facts.imports.push(Import { line, module: module.into(), alias, kind: IMPORT_MODULE, ..Import::default() });
                    }
                }
                // The TypeScript grammar gives these words their own node kind,
                // so `import get from 'lodash/get'` bound nothing on this side
                // while the lexer recorded the alias.
                "is" | "as" | "satisfies" | "get" | "set" | "static" | "readonly" | "declare" | "async" => {
                    let alias = self.text_string(child);
                    self.facts.imports.push(Import { line, module: module.into(), alias, kind: IMPORT_MODULE, ..Import::default() });
                }
                "named_imports" | "named_exports" => {
                    for specifier in self.children(child) {
                        if matches!(self.kind(specifier), "import_specifier" | "export_specifier") {
                            self.js_specifier(specifier, module, line);
                        }
                    }
                }
                "import_specifier" | "export_specifier" => self.js_specifier(child, module, line),
                "namespace_import" | "namespace_export" => {
                    let alias = self.children(child).into_iter().rev().find_map(|node| {
                        (self.kind(node) == "identifier").then(|| self.text_string(node))
                    });
                    self.facts.imports.push(Import {
                        line,
                        module: module.into(),
                        alias: alias.unwrap_or_default(),
                        kind: IMPORT_MODULE,
                        ..Import::default()
                    });
                }
                _ => {}
            }
        }
    }

    pub(crate) fn js_specifier(&mut self, id: u32, module: &str, line: u32) {
        let children = self.children(id);
        let Some(&first) = children.first() else { return };
        let name = self.text_string(first);
        let alias = children
            .iter()
            .position(|&child| self.kind(child) == "as")
            .and_then(|at| children.get(at + 1))
            .map(|&node| self.text_string(node))
            .unwrap_or_else(|| name.clone());
        self.facts.imports.push(Import { line, module: module.into(), name, alias, kind: IMPORT_MODULE });
    }

    pub(crate) fn string_value(&self, id: u32) -> String {
        let text = String::from_utf8_lossy(self.text(id));
        text.trim_matches(|ch| matches!(ch, '"' | '\'' | '`')).to_string()
    }

    // -- values -------------------------------------------------------------

    /// The flow form of one expression, recording every call it contains, in
    /// the same shapes `crate::expr` documents:
    ///
    /// ```text
    /// 0                    a constant
    /// ["n","a.b"]          a name or attribute path
    /// ["m",[e,...]]        values combined (operator, format string, mapping)
    /// ["t",[e,...]]        a tuple or list, element by element
    /// ["c",id,recv,[arg]]  a call; `id` indexes the symbol's call table
    /// ["k","name",e]       a keyword argument, only inside a call's arguments
    /// ```
    pub(crate) fn form(&mut self, id: u32) -> String {
        let kind = self.kind(id);

        if self.is_rust() {
            if let Some(out) = self.rust_form(id) {
                return out;
            }
        }
        // `(socklen_t)ss_size`: a cast is the type's words and the operand's
        // value to the one-pass reader, and the shared fallbacks below read
        // the node as its first child, which is the type alone.
        if self.is_c() {
            if let Some(out) = self.c_cast_form(id) {
                return out;
            }
            if let Some(out) = self.c_designated_form(id) {
                return out;
            }
        }
        if self.kinds.calls.contains(&kind) {
            // `repositories { mavenCentral() }`: the Kotlin DSL idiom, a call
            // whose only argument is a lambda. The lexer files the line as the
            // name, not as a call, so the value is the name and the call below
            // is still recorded for the resolver.
            if self.is_kotlin() {
                if self.in_kotlin_expr_body {
                    if let Some(out) = self.kotlin_expr_call_form(id) {
                        return out;
                    }
                } else if let Some(name) = self.kotlin_dsl_name(id) {
                    let _ = self.call_form_link(id);
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &name);
                    out.push(']');
                    return out;
                }
            }
            return self.call_form(id);
        }
        // `val on = false`: the lexer writes a boolean or `null` literal as the
        // name it spells, and the tree keeps the word as its own leaf kind. Read
        // through the default arm the value was the constant 0, so every
        // property initialised to `false` disagreed with the lexer.
        if self.is_kotlin() && matches!(kind, "true" | "false" | "null") {
            let mut out = "[\"n\",".to_string();
            push_string(&mut out, kind);
            out.push(']');
            return out;
        }
        // Python has no boolean keyword in this grammar, but the lexer reads
        // `True`, `False` and `None` as names, so they are names here too.
        if matches!(kind, "True" | "False" | "None") {
            let text = self.text_string(id);
            let mut out = "[\"n\",".to_string();
            push_string(&mut out, &text);
            out.push(']');
            return out;
        }
        // Kotlin's and Scala's `true`, `false` and `null` are keyword leaves
        // here, and the lexer reads each of them as a name: `channel = null`
        // is `a:[n:channel,n:null]` there and formed as the constant 0 here,
        // which lost every such assignment.
        if (self.is_kotlin() || self.language.name == "scala") && matches!(kind, "true" | "false" | "null") {
            let text = self.text_string(id);
            let mut out = "[\"n\",".to_string();
            push_string(&mut out, &text);
            out.push(']');
            return out;
        }
        // `resiliencyChecks.creatingOutOfBoundsIndicesBehavior = .trap`: Swift's
        // `.trap` is an implicit member, a bare name with no receiver. The lexer
        // reads it as the name `n:trap`; read through the default arm it was the
        // constant 0. A call on one (`.macOS("10")`) is a call_expression, so it
        // never reaches this arm.
        if self.language.name == "swift" && kind == "implicit_member_expression" {
            let name = self.member_name(id);
            if !name.is_empty() {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &name);
                out.push(']');
                return out;
            }
        }
        if let Some(name) = self.soft_name(id) {
            let mut out = "[\"n\",".to_string();
            push_string(&mut out, &name);
            out.push(']');
            return out;
        }
        match kind {
            // `Foo::class`, `Foo::bar`: a callable reference is one dotted name
            // to the lexer (`n:Foo.class`), and the tree writes the receiver and
            // the referenced name as two nodes.
            "callable_reference" if self.is_kotlin() => {
                let children = self.children(id);
                let mut text = String::new();
                for child in children {
                    if self.is_layout(child) || self.kind(child) == "::" {
                        continue;
                    }
                    if !text.is_empty() {
                        text.push('.');
                    }
                    text.push_str(&self.text_string(child));
                }
                if text.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &text);
                    out.push(']');
                    out
                }
            }
            // `get() = ::periodPatternCategory`: a bare callable reference has
            // no receiver, so the grammar files it as `absolute_reference`
            // rather than `callable_reference`. The lexer reads the reference
            // name as the accessor's value; read as the constant 0 the getter
            // line lost its only name (`override val … get() = ::name`).
            "absolute_reference" if self.is_kotlin() => {
                let children = self.children(id);
                let mut text = String::new();
                for child in children {
                    if self.is_layout(child) || self.kind(child) == "::" {
                        continue;
                    }
                    text.push_str(&self.text_string(child));
                }
                if text.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &text);
                    out.push(']');
                    out
                }
            }
            // `object : Base()`: an anonymous object is the name `object`
            // merged with its supertype constructor's call, which is how the
            // lexer writes it. Nothing read the node, so the value was the
            // constant 0 and the constructor call was lost with it.
            "object_expression" if self.is_kotlin() => {
                let mut parts: Vec<String> = vec![String::from("[\"n\",\"object\"]")];
                if let Some(list) = self.child_of_kind(id, "supertypes") {
                    for child in self.children(list) {
                        if self.is_layout(child) {
                            continue;
                        }
                        let part = self.form(child);
                        if part != "0" {
                            parts.push(part);
                        }
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // `object : Base by delegate()`: a delegation call is the fact the
            // lexer records on the object's line. The grammar builds a
            // `supertype_delegation` node whose delegate is a name plus an
            // argument list, not a `call_expression`, so no arm walked it and
            // the call was missing from the CST's record. Keep the value 0 —
            // the object's own event already reads as `n:object` — and only
            // record the call.
            "supertype_delegation" if self.is_kotlin() => {
                let children = self.children(id);
                if let Some(at) = children.iter().position(|&child| self.kind(child) == "by") {
                    let rest = &children[at + 1..];
                    if let Some(&call) = rest.iter().find(|&&child| self.kind(child) == "call_expression") {
                        let _ = self.form(call);
                    } else {
                        let args = rest.iter().copied().find(|&child| matches!(self.kind(child), "arguments" | "argument_list"));
                        let mut name = String::new();
                        for &child in rest {
                            if matches!(self.kind(child), "arguments" | "argument_list") {
                                break;
                            }
                            match self.kind(child) {
                                "." | "?." => name.push('.'),
                                k if self.is_layout(child) || k == "by" => {}
                                _ => name.push_str(&self.text_string(child)),
                            }
                        }
                        if !name.is_empty() {
                            self.push_call(name, self.line(id));
                            if let Some(list) = args {
                                let _ = self.arguments(list);
                            }
                        }
                    }
                }
                "0".to_string()
            }
            // `object : Base()`, `class C : Base()`: a supertype constructor is
            // a call of its type's name with its own arguments.
            "superclass_constructor" if self.is_kotlin() => self.call_form(id),
            // `Assert.Throws<T>(() => reader.Read())`: the one-pass reader
            // writes a generic invocation as the callee's name, the type
            // argument's name and the arguments' value - never as a call of its
            // own - and a call inside the arguments belongs to the same
            // statement. Read as a member access the tree wrote the name alone
            // and dropped every lambda argument (636 rows, the largest C#
            // cluster left after the attributes).
            "generic_invocation" => return self.cs_generic_invocation(id),
            // `() => reader.Read()` as an argument: the one-pass reader files
            // the body's value in the statement and records the call inside
            // it. The tree kept the lambda node and nothing read its body, so
            // every lambda argument lost what it contains.
            "lambda_expression" if self.language.name == "csharp" => return self.cs_lambda_value(id),

            // `T(text = writer.toString())`: a named argument is its value. Read
            // as its first child, the parameter name `text` swallowed the call
            // on the right of the `=`, and every named call argument was lost.
            "named_argument" if self.is_kotlin() => {
                let children = self.children(id);
                match children.iter().copied().rev().find(|&child| self.kind(child) != "=" && !self.is_layout(child)) {
                    Some(value) => self.form(value),
                    None => "0".to_string(),
                }
            }
            // `InternalAPI::class`: the one-pass reader spells a class literal
            // as the dotted path `InternalAPI.class`, so the literal is that
            // name. Formed as the constant 0 it made the argument of every
            // `@OptIn(X::class)` disagree with the lexer's.
            "class_literal" if self.is_kotlin() => {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &self.decl_path(id));
                out.push(']');
                out
            }
            // `(unsigned short)(port & 0xffff)` and
            // `sizeof(struct sockaddr_in6)`: the one-pass reader writes the
            // type words in front of the operand as the value, and the CST
            // formed the constant `0` - every cast or sizeof in a value
            // position disagreed, the C corpus's third-largest cluster. The
            // operand follows the type words, so a cast names both and the
            // words the lexer read are covered.
            "cast_expression" | "user_cast_expression" | "sizeof_expression" if self.is_c() => {
                let mut parts: Vec<String> = Vec::new();
                let children = self.children(id);
                for (at, &child) in children.iter().enumerate() {
                    let child_kind = self.kind(child);
                    if matches!(child_kind, "cast_type" | "sizeof_operand" | "user_cast_head") {
                        for word in self.c_words_of(child) {
                            let mut form = String::from("[\"n\",");
                            push_string(&mut form, &word);
                            form.push(']');
                            parts.push(form);
                        }
                        continue;
                    }
                    if self.is_layout(child) {
                        continue;
                    }
                    if at + 1 == children.len() {
                        let form = self.form(child);
                        if form != "0" {
                            parts.push(form);
                        }
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // `val h = object : Base() { … }`: an object expression is the
            // keyword and its supertype's constructor call, which is the call
            // the lexer records on that line. With no arm the node formed the
            // constant 0, so every anonymous implementation lost its call.
            "object_expression" => {
                let mut parts: Vec<String> = Vec::new();
                let mut word = String::from("[\"n\",");
                push_string(&mut word, "object");
                word.push(']');
                parts.push(word);
                for child in self.children(id) {
                    if self.kind(child) != "supertypes" {
                        continue;
                    }
                    for entry in self.children(child) {
                        if matches!(self.kind(entry), "superclass_constructor" | "call_expression") {
                            let form = self.call_form(entry);
                            if form != "0" {
                                parts.push(form);
                            }
                        }
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // `return { type, value, line }`: a shorthand property whose name
            // is a keyword in another position carries that keyword's kind, so
            // it read as a leaf and the property vanished from the value.
            // `resiliencyChecks.x = .trap`: an implicit member names the member
            // the lexer reads (`n:trap`). Its children are the `.` and the name,
            // so the default arm found no value child and wrote the constant 0.
            "implicit_member_expression" => {
                let name = self.member_name(id);
                if name.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &name);
                    out.push(']');
                    out
                }
            }
            "identifier" | "this" | "super" | "self" | "shorthand_property_identifier" | "_" | "type" | "from"
            | "of" | "satisfies" => {
                let text = self.text_string(id);
                let text = if text == "this" { "self".to_string() } else { text };
                if text.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &text);
                    out.push(']');
                    out
                }
            }
            "lifetime" => {
                let text = self.text_string(id);
                let text = text.trim_start_matches('\'').trim_start_matches("r#");
                if text.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, text);
                    out.push(']');
                    out
                }
            }
            // `String::class` written where a value goes: a callable reference
            // names the path with a dot, the way the one-pass reader spells
            // `::`, and read as the constant 0 it left the annotation's own
            // argument disagreeing with the lexer (`@OptIn(X::class)`).
            "callable_reference" => {
                let mut path = String::new();
                for child in self.children(id) {
                    if self.is_layout(child) {
                        continue;
                    }
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(&self.text_string(child));
                }
                if path.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &path);
                    out.push(']');
                    out
                }
            }
            // Swift's `.trap` as a value (`x.mode = .trap`) is an implicit
            // member: a name with no receiver. The lexer writes it as the
            // member's name, and read through the default arm it was the
            // constant 0, so every assignment of an enum case lost its value.
            "implicit_member_expression" => {
                let name = self.member_name(id);
                if name.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &name);
                    out.push(']');
                    out
                }
            }
            // `field_expression` is Rust's spelling of the same path; without
            // this arm `self.skipped_groups == 0` read both sides as the
            // constant and the name left the record.
            // Swift's `.trap` names a member of the contextual type: the lexer
            // reads the name, and the constant 0 here lost every such value.
            "implicit_member_expression" => match self.children(id).iter().rev().find_map(|&child| self.leaf_name(child)) {
                Some(name) => {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &name);
                    out.push(']');
                    out
                }
                None => "0".to_string(),
            },
            // `MemoryLayout<Int>.size`: a member of a specialized type. Its base
            // is a generic specialization that has no path, so the default arm
            // below formed only the type arguments and dropped the member. The
            // lexer reads the type's names and the member, so both are kept.
            "navigation_expression"
                if self.language.name == "swift"
                    && self.children(id).first().is_some_and(|&base| self.kind(base) == "generic_specialization") =>
            {
                let base = self.children(id)[0];
                let mut parts = Vec::new();
                for child in self.children(base) {
                    if self.is_layout(child) || matches!(self.kind(child), "<" | ">" | ",") {
                        continue;
                    }
                    let value = self.form(child);
                    if value != "0" {
                        parts.push(value);
                    }
                }
                let name = self.member_name(id);
                if !name.is_empty() {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &name);
                    out.push(']');
                    parts.push(out);
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            "attribute" | "member_expression" | "member_access_expression" | "member_access" | "subscript" | "subscript_expression" | "field_expression" | "meta_property"
            // `Ordering::Acquire` in an argument list is a path, not a constant.
            | "scoped_identifier" | "scoped_type_identifier"
            // Kotlin's `user.title` in a `"${user.title}"` hole.
            | "navigation_expression" => {
                let path = self.form_path(id);
                if path.is_empty() || path.starts_with('.') {
                    // `f(x)[0].y`: the base is a call, so there is no path, and
                    // the value is the base's — a subscript of a value is that
                    // value. Writing 0 here lost the call's argument.
                    match self.children(id).first().copied() {
                        Some(base) => self.form(base),
                        None => "0".to_string(),
                    }
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &path);
                    out.push(']');
                    out
                }
            }
            // `register: .rflags`: a Swift implicit member names a member of
            // the contextual type, so it is a name, not a constant. Formed as
            // the default 0, every labeled argument written with a leading dot
            // lost its value on the CST path only.
            "implicit_member_expression" => {
                let name = self.member_name(id);
                if name.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &name);
                    out.push(']');
                    out
                }
            }
            // `val Normal = FSM.Normal`: Scala spells a dotted name as a
            // `select` node. It is not in the member-access list above, so the
            // default arm formed it as the constant 0 and every binding to a
            // qualified name lost its value (`FSM.Normal`, `java.lang.Math.PI`,
            // `ManagedActorEventBus.this.system`).
            "select" if self.language.name == "scala" => {
                let path = self.form_path(id);
                if path.is_empty() || path.starts_with('.') {
                    match self.children(id).first().copied() {
                        Some(base) => self.form(base),
                        None => "0".to_string(),
                    }
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &path);
                    out.push(']');
                    out
                }
            }
            // `resiliencyChecks.x = .trap`: a leading-dot member has no
            // receiver, so it is a name, not the constant 0 the default arm
            // wrote. The lexer names it by its member alone (`n:trap`).
            "implicit_member_expression" if self.language.name == "swift" => {
                let name = self.member_name(id);
                if name.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &name);
                    out.push(']');
                    out
                }
            }
            "qualified_type" if self.is_kotlin() => {
                let path = self.path_of(id);
                if path.is_empty() {
                    "0".to_string()
                } else {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &path);
                    out.push(']');
                    out
                }
            }
            // `longStrings[toInt()]`: Kotlin's indexed access is the base's
            // value, and a call written in the subscript is a fact of its own.
            // With no arm the node formed the constant 0 and the call in the
            // index was lost on every `xs[f(..)]` in the corpus.
            // Swift, Go and the rest spell an indexed access differently; this
            // arm is the Kotlin node only.
            "index_expression" if self.is_kotlin() => {
                let children = self.children(id);
                for child in &children {
                    if self.kind(*child) != "subscript" {
                        continue;
                    }
                    for inner in self.children(*child) {
                        if !self.is_layout(inner) {
                            self.form(inner);
                        }
                    }
                }
                match children.first() {
                    Some(&base) => self.form(base),
                    None => "0".to_string(),
                }
            }
            // `(0, _createSvgIcon.default)(...)`: the comma idiom every
            // transpiler emits is a sequence expression, and it was forming as
            // the constant 0, so every CommonJS `exports.default = ...` in the
            // corpus lost its value.
            "sequence_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.tuple_form(&items),
                }
            }
            "argument_list" | "arguments" | "parenthesized_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.tuple_form(&items),
                }
            }
            "binary_operator" | "comparison_operator" | "boolean_operator" | "binary_expression" | "ternary_expression"
            // `f() ?: g()`: the elvis holds two values the way a binary
            // expression does, and read as a wrapper the right operand left the
            // record - every `?: error("...")` lost the call in its fallback.
            | "elvis_expression"
            // `a .. b`, `a ..< b`: Kotlin's range expression holds two
            // operands, and read through the unmapped fallback it kept only the
            // left leaf and dropped any calls on the right.
            | "range_expression"
            // `a if c else b` holds three values and every one is read; the
            // condition is where `x if p(v) else y` calls `p`. `(n := f())` is
            // the same: the value is on the right, the name on the left.
            | "conditional_expression" | "named_expression"
            // `xs[lo():hi():st()]`: a slice and a subscript list are lists of
            // values, not wrappers, so nothing may be dropped.
            | "slice" | "subscript_list"
            // Ruby and Scala name their binary expression `binary` too, and
            // keep the default arm; only Swift's reads through this one.
            | "binary" if kind != "binary" || self.language.name == "swift" => {
                // `type + 'x'`: `type` is a keyword in this grammar, so the
                // layout filter drops it and the binary expression keeps only
                // its right operand — the lexer reads `m:[n:type,0]` and the
                // CST wrote `0`. A word that is not one of this arm's operators
                // is an operand.
                let mut items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| self.binary_operand(child))
                    .collect();
                if kind == "binary" {
                    let first = items.first().map(|&child| self.line(child));
                    let (here, below): (Vec<u32>, Vec<u32>) = items.into_iter().partition(|&child| Some(self.line(child)) == first);
                    for child in below {
                        let value = self.form(child);
                        if value != "0" {
                            let line = self.line(child);
                            self.event(&format!("[\"x\",{line},{value}]"));
                        }
                    }
                    items = here;
                }
                if items.len() == 1 {
                    return self.form(items[0]);
                }
                self.merge_form(&items)
            }
            // `val a = if (c) f(0) else g`: Scala's `if` is an expression
            // whose value is either branch. Read through the default arm the
            // value was the condition alone, so both branches' names and calls
            // left the record (`acc.history(0)` and `acc.current`, 8 rows in
            // the scala corpus's jdk accumulators). The `if` and `else`
            // keywords are not operands.
            "if_statement" if self.language.name == "scala" => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| self.binary_operand(child) && !matches!(self.text_string(child).as_str(), "if" | "else"))
                    .collect();
                if items.len() == 1 {
                    return self.form(items[0]);
                }
                self.merge_form(&items)
            }
            // `x ?? throw new ArgumentNullException(nameof(x))`: a throw
            // expression is its operand, and the call the lexer reads on that
            // line is inside it.
            "throw_expression" | "throw_statement" => match self.expression_child(id) {
                Some(child) => self.form(child),
                None => "0".to_string(),
            },
            // `f() && let Some(x) = g()`: a let-chain operand of a binary
            // expression. The pattern is a binding, not a value, so the node
            // renders as the expression to the right of `=` — reading the node
            // as an unmapped child rendered the whole operand as the constant 0
            // and lost the call inside it.
            "let_condition" | "let_chain" => match self.rust_let_value(id) {
                Some(value) => self.form(value),
                None => "0".to_string(),
            },
            // `"Foo" as NSString`: Swift's cast names the value and its target
            // type, as the one-pass reader does; the default arm kept only the
            // literal and dropped the type.
            "cast" if self.language.name == "swift" => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && !matches!(self.kind(child), "as" | "is" | "?" | "!"))
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `x as Foo`, `x as? Foo.Bar`: Kotlin's cast names both the value
            // and the target type to the one-pass reader; the shared unary arm
            // kept only the operand and dropped the type.
            "as_expression" | "is_expression" if self.is_kotlin() => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && !matches!(self.kind(child), "as" | "is" | "!is" | "?"))
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            "lambda_literal" | "annotated_lambda" | "labeled_lambda" | "lambda_body" | "lambda_items" | "destructuring_parameter"
                if self.is_kotlin() =>
            {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| {
                        !self.is_layout(child)
                            && !matches!(
                                self.kind(child),
                                "{" | "}" | "(" | ")" | "->" | "," | ":" | ";" | "LINE_END" | "@"
                            )
                    })
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            "when_statement" | "when_subject" | "when_body" | "when_entry" if self.is_kotlin() => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| {
                        !self.is_layout(child)
                            && !matches!(
                                self.kind(child),
                                "when" | "(" | ")" | "val" | "var" | "=" | "{" | "}" | ";" | "LINE_END"
                                    | "else" | "in" | "!in" | "is" | "!is" | "->" | ","
                            )
                    })
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `val M = if (h <= 0) 0L else acc.cumulative(h-1)`: Scala's `if`
            // is a value too, and the default arm read only its condition, so
            // both branches (and the calls in them) left the CST's value.
            "if_statement" if self.is_kotlin() || self.language.name == "scala" => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && !matches!(self.kind(child), "if" | "else"))
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `if (p) null else f()`: a branch is an `expression_statement`
            // whose child may be a bare literal. The generic default only
            // reads a child that has children, so a `null` branch became the
            // constant 0 and the getter's merged value lost the name the lexer
            // reads for it (`get() = if (…) null else f()`).
            "expression_statement" if self.is_kotlin() => {
                match self.children(id).into_iter().find(|&child| !self.is_layout(child)) {
                    Some(child) => self.form(child),
                    None => "0".to_string(),
                }
            }
            "block" if self.is_kotlin() => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && !matches!(self.kind(child), "{" | "}" | ";" | "LINE_END"))
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            "labeled_this" if self.is_kotlin() => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && self.kind(child) != "@")
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `val x = f()`: a property declaration inside an expression-valued
            // lambda or block still evaluates its initializer, so `form` must
            // walk the expression after `=` or `by` rather than stopping at
            // `property_head` through the default child fallback.
            "property_declaration" if self.is_kotlin() => {
                let children = self.children(id);
                match children
                    .iter()
                    .position(|&child| matches!(self.kind(child), "=" | "by"))
                    .and_then(|at| children.get(at + 1).copied())
                {
                    Some(value) => self.form(value),
                    None => "0".to_string(),
                }
            }
            "not_null_expression" if self.is_kotlin() => match self.expression_child(id) {
                Some(child) => self.form(child),
                None => "0".to_string(),
            },
            // `into: &thread`: Swift's `&` on an argument marks it inout, and
            // the value is the operand itself. The one-pass lexer reads the
            // bare name, so wrapping it as a merge left every inout argument
            // a loss on the CST path only. `!` is the same: `!$Embedded` and
            // `!foo` read the operand, and the default arm read nothing.
            "prefix_expression" if self.language.name == "swift" && matches!(self.text_string(id).chars().next(), Some('&' | '!')) => match self.expression_child(id) {
                Some(child) => self.form(child),
                None => "0".to_string(),
            },
            "not_operator" | "unary_operator" | "unary_expression" | "await" | "await_expression" | "yield" | "yield_expression"
            | "typeof" | "void" | "non_null_expression" | "as_expression" | "satisfies_expression" | "type_assertion" | "delete_expression" => {
                match self.expression_child(id) {
                    Some(child) => {
                        let inner = self.form(child);
                        format!("[\"m\",[{inner}]]")
                    }
                    None => "0".to_string(),
                }
            }
            // `f() ? "a" : "b"`: the condition and both branches are the value,
            // as the one-pass reader merges them. The default arm read only the
            // condition, so the branches (and any call in them) were lost.
            // The grammar also files an optional type's `?` (`-> UInt8? in`) as a
            // ternary; only a node with its `:` is a conditional.
            "ternary" if self.language.name == "swift" && self.children(id).iter().any(|&child| self.kind(child) == ":") => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && !matches!(self.kind(child), "?" | ":"))
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `a if p(x) else b`: the condition is an expression of its own and
            // the call in it is a fact. The lexer's flow form reads the whole
            // conditional, so the CST must too.
            "conditional_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `if (acc.hIndex > 0) acc.history(0) else acc.current`: Scala's
            // `if_expression` is not a `conditional_expression`, so the default
            // arm read only its first child (the condition) and every name and
            // call in the branches was missing from the value the guard checks.
            "if_statement" if self.language.name == "scala" => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && !matches!(self.kind(child), "if" | "else"))
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `x match { case A => a; case B => b }`: Scala's `match_expression`
            // has the scrutinee, the `match` keyword and the `match_block`, and
            // the default arm read only the scrutinee. A binding initialised
            // with a match wrote the constant 0 against the lexer's merge.
            "match_expression" if self.language.name == "scala" => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !matches!(self.kind(child), "{" | "}"))
                    .collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `case P => body`: the arm's value is its body. The pattern is a
            // match, not a value, and reading the whole clause formed only the
            // pattern, so `case None => DefaultPositive` lost its name.
            "case_clause" if self.language.name == "scala" => {
                let children = self.children(id);
                let body: Vec<u32> = match children.iter().position(|&child| self.text_string(child) == "=>") {
                    Some(at) => children[at + 1..].iter().copied().filter(|&child| !self.is_layout(child)).collect(),
                    None => Vec::new(),
                };
                match body.len() {
                    0 => "0".to_string(),
                    1 => self.form(body[0]),
                    _ => self.merge_form(&body),
                }
            }
            "match_block" if self.language.name == "scala" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // `xs[1:min(len(xs), n)]`, `for x in a, b`: a slice or an
            // expression list is a run of expressions and every one of them
            // can carry a call. Reading only the first would drop the bound.
            "slice" | "open_slice" | "subscript_list" | "expression_list" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.tuple_form(&items),
                }
            }
            // `z: "Thing"` with no value: the statement is the annotation, and
            // the name in it is what the lexer reads as the expression. With a
            // value (`z: "Thing" = f()`) the value is what the statement is.
            "annotated_expression" => self
                .children(id)
                .into_iter()
                .rev()
                .find(|&child| !self.is_layout(child) && self.kind(child) != "annotation")
                .map(|child| self.form(child))
                .unwrap_or_else(|| "0".to_string()),
            "annotated" => {
                let children = self.children(id);
                let has_value = children.iter().any(|&child| self.kind(child) == "=");
                match (has_value, children.last()) {
                    (true, Some(&last)) => self.form(last),
                    (false, Some(&first)) => self.form(first),
                    _ => "0".to_string(),
                }
            }
            // `path := f(x)`: a walrus expression is the value, and the name
            // is a binding.
            "named_expression" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => {
                        let value = items[items.len() - 1];
                        self.form(value)
                    }
                }
            }
            // `lambda r, proxy=url: ...`: a defaulted parameter is a keyword
            // argument to the lexer — `["k","proxy",["n","url"]]` — and read as
            // a merge of the two names it lost the keyword every time a lambda
            // defaulted a parameter (request's `lambda proxy=url, type=type`).
            "lambda_parameter" if self.children(id).iter().any(|&child| self.kind(child) == "=") => {
                let children = self.children(id);
                let name = children.first().map(|&node| self.path_of(node)).unwrap_or_default();
                let inner = children.last().map(|&node| self.form(node)).unwrap_or_else(|| "0".to_string());
                let mut out = "[\"k\",".to_string();
                push_string(&mut out, &name);
                out.push(',');
                out.push_str(&inner);
                out.push(']');
                out
            }
            // `lambda x: f(x)`: the parameters and body form the lambda value.
            "lambda" | "lambda_parameter" | "lambda_star_parameter" | "lambda_parameters" => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| {
                        !self.is_layout(child)
                            && !matches!(self.kind(child), "lambda" | ":" | "," | "=" | "*" | "**" | "/")
                    })
                    .collect();
                let mut parts = Vec::new();
                for (at, &child) in items.iter().enumerate() {
                    let value = self.form(child);
                    // `lambda x: ""` and `lambda: 0`: a body that is a literal
                    // is the constant 0 and it is still the lambda's value.
                    // Dropping every 0 left the parameter as the whole form, so
                    // calendar's `_months.insert(0, lambda x: "")` lost the body.
                    if value != "0" || at + 1 == items.len() {
                        parts.push(value);
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // `["a": "1", "b": "Foo"]`: Swift's grammar leaves a pair's key and
            // value as siblings with the `:` between them, so each pair is
            // read as one merged item, as the one-pass reader does. Groups are
            // split on the `,` and a group of two is a `key: value` pair.
            "collection_literal" if self.language.name == "swift" && self.swift_has_pair_colon(id) => {
                let mut groups: Vec<Vec<u32>> = Vec::new();
                let mut current: Vec<u32> = Vec::new();
                for child in self.swift_collection_children(id) {
                    match self.kind(child) {
                        "," => {
                            if !current.is_empty() {
                                groups.push(std::mem::take(&mut current));
                            }
                        }
                        ":" => {}
                        _ if self.is_layout(child) => {}
                        _ => current.push(child),
                    }
                }
                if !current.is_empty() {
                    groups.push(current);
                }
                if groups.is_empty() {
                    "0".to_string()
                } else {
                    let mut forms: Vec<String> = Vec::new();
                    for group in &groups {
                        if group.len() == 1 {
                            forms.push(self.form(group[0]));
                            continue;
                        }
                        // A pair's cast value is spliced into the pair, as the
                        // one-pass reader writes `"b": "Foo" as NSString`.
                        let mut parts: Vec<String> = Vec::new();
                        for &item in group {
                            if self.kind(item) == "cast" {
                                let inner: Vec<u32> = self
                                    .children(item)
                                    .into_iter()
                                    .filter(|&child| !self.is_layout(child) && !matches!(self.kind(child), "as" | "is" | "?" | "!"))
                                    .collect();
                                parts.extend(inner.into_iter().map(|child| self.form(child)));
                            } else {
                                parts.push(self.form(item));
                            }
                        }
                        forms.push(format!("[\"m\",[{}]]", parts.join(",")));
                    }
                    format!("[\"t\",[{}]]", forms.join(","))
                }
            }
            "collection_literal" | "tuple" | "list" | "set" | "pattern_list" | "array" | "tuple_pattern" | "list_pattern" | "array_pattern" => {
                // Swift wraps a literal's elements in one `collection_body`
                // node, so `[1, 2, 3]` had one child and formed as `[0]`. The
                // elements are that node's children, one per element.
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .flat_map(|child| {
                        if self.kind(child) == "collection_body" {
                            self.children(child)
                        } else {
                            vec![child]
                        }
                    })
                    .filter(|&child| !self.is_layout(child))
                    .collect();
                if items.is_empty() {
                    "0".to_string()
                } else {
                    self.tuple_form(&items)
                }
            }
            "list_comprehension" | "set_comprehension" | "generator_expression" | "dictionary_comprehension" => {
                // `[f(x) for x in xs if p(x)]`: the element and every clause's
                // value are what the reader needs, and the clauses carry calls.
                let mut items: Vec<u32> = Vec::new();
                for child in self.children(id) {
                    if self.is_layout(child) {
                        continue;
                    }
                    items.push(child);
                }
                if items.is_empty() {
                    "0".to_string()
                } else {
                    self.tuple_form(&items)
                }
            }
            "for_in_clause" => {
                // `for x in xs`: both the target and the iterable are written
                // in the source and the one-pass lexer reads both. Reading
                // only the iterable dropped the name the clause binds, and
                // every comprehension over a name it also uses fell back.
                let children = self.children(id);
                match children.iter().position(|&child| self.kind(child) == "in") {
                    Some(at) => {
                        let mut items: Vec<u32> =
                            children[..at].iter().copied().filter(|&child| !self.is_layout(child)).collect();
                        items.extend(children[at + 1..].iter().copied().filter(|&child| !self.is_layout(child)));
                        if items.is_empty() {
                            "0".to_string()
                        } else {
                            self.merge_form(&items)
                        }
                    }
                    None => match children.into_iter().rev().find(|&child| !self.is_layout(child)) {
                        Some(child) => self.form(child),
                        None => "0".to_string(),
                    },
                }
            }
            "if_clause" | "comprehension_clause" => {
                // `if p(x)`: the value is the last child.
                match self.children(id).into_iter().rev().find(|&child| !self.is_layout(child)) {
                    Some(child) => self.form(child),
                    None => "0".to_string(),
                }
            }
            "dictionary" | "object" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                if items.is_empty() {
                    "0".to_string()
                } else {
                    self.merge_form(&items)
                }
            }
            "pair" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                // `{ [key]: 1 }`: the one-pass reader writes the computed key as
                // a list holding the name in it, and the pair there is the key,
                // not a key/value merge.
                if items.first().is_some_and(|&first| self.kind(first) == "computed_property_name") {
                    let inner = self
                        .children(items[0])
                        .into_iter()
                        .find(|&child| !self.is_layout(child));
                    let key = inner.map(|child| self.form(child)).unwrap_or_else(|| "0".to_string());
                    // `{ [TypeId]: { … } }` is a pair like any other: the key
                    // is a computed list and the value is the pair's last item.
                    // Returning the key alone dropped the value (14 js files).
                    let value = match items.last() {
                        Some(&last) if last != items[0] => self.form(last),
                        _ => "0".to_string(),
                    };
                    return format!("[\"m\",[[\"t\",[{key}]],{value}]]");
                }
                match items.len() {
                    // `{ async: async }`: key and value are the same keyword, so
                    // both children are layout and the pair held nothing at all —
                    // the one-pass reader writes the name once and the CST wrote
                    // the constant 0, dropping the property. A key and value that
                    // spell the same word are one name.
                    0 => {
                        let words: Vec<String> = self
                            .children(id)
                            .into_iter()
                            .map(|child| self.text_string(child))
                            .filter(|text| {
                                !text.is_empty()
                                    && text != ":"
                                    && (text.as_bytes()[0].is_ascii_alphabetic() || text.as_bytes()[0] == b'_')
                            })
                            .collect();
                        if let Some(first) = words.first().filter(|_| words.iter().all(|word| word == &words[0])) {
                            let mut out = "[\"m\",[[\"n\",".to_string();
                            push_string(&mut out, first);
                            out.push_str("]]]");
                            out
                        } else {
                            "0".to_string()
                        }
                    }
                    // `{ type: "boolean" }`: a key spelled like a keyword (`type`,
                    // `class`, `default`) has that keyword as its kind, so
                    // is_layout() drops it and the pair is left holding its value
                    // alone. The lexer reads the key as a name, so the CST must:
                    // the key is the first child whose text is a word.
                    1 => {
                        let key = self
                            .children(id)
                            .into_iter()
                            .find(|&child| {
                                let text = self.text_string(child);
                                !text.is_empty()
                                    && text != ":"
                                    && (text.as_bytes()[0].is_ascii_alphabetic() || text.as_bytes()[0] == b'_')
                            });
                        match key {
                            Some(key) => {
                                let mut out = "[\"m\",[[\"n\",".to_string();
                                push_string(&mut out, &self.text_string(key));
                                out.push_str("],");
                                out.push_str(&self.form(items[0]));
                                out.push_str("]]");
                                out
                            }
                            None => self.form(items[0]),
                        }
                    }
                    _ => {
                        let key = self.form(items[0]);
                        let value = self.form(items[items.len() - 1]);
                        format!("[\"m\",[{key},{value}]]")
                    }
                }
            }
            "keyword_argument" => {
                // `f(default=True)` is `name`, `=`, `value`: the separator is a
                // child, and taking the children as a pair read the value as
                // the `=` and every keyword argument as a constant.
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && self.kind(child) != "=")
                    .collect();
                // The grammar writes the `=` sometimes as a child and sometimes
                // not, so the name is the first item and the value the last.
                match (items.first().copied(), items.last().copied()) {
                    (Some(name), Some(value)) if items.len() >= 2 => {
                        let mut out = "[\"k\",".to_string();
                        push_string(&mut out, &self.text_string(name));
                        out.push(',');
                        out.push_str(&self.form(value));
                        out.push(']');
                        out
                    }
                    _ => "0".to_string(),
                }
            }
            // `f(*args, **kwargs)`: Python spells the double splat its own way,
            // and the one-pass lexer reads the name under it.
            "spread_element" | "spread_argument" | "rest_pattern" | "list_splat" | "list_splat_pattern" | "dictionary_splat" => match self.expression_child(id) {
                Some(child) => self.form(child),
                None => "0".to_string(),
            },
            "assignment" | "assignment_expression" | "augmented_assignment_expression" => {
                let children = self.children(id);
                let value = children.last().filter(|&&last| last != children[0]).map(|&last| self.form(last)).unwrap_or_else(|| "0".to_string());
                // `cache['SHA1'] = cache['sha1'] = _sha1.sha1` nests: the right
                // side is another assignment, and the one-pass lexer reads the
                // line left to right, so its value names the inner binder too.
                // Reading only the innermost value dropped `cache`.
                let chained = children
                    .last()
                    .filter(|&&last| last != children[0])
                    .is_some_and(|&last| matches!(self.kind(last), "assignment" | "assignment_expression"));
                if chained {
                    let binder = self.form(children[0]);
                    return format!("[\"m\",[{binder},{value}]]");
                }
                // `var _default = exports.default = (0, f)(x)`: the declaration's
                // value is an assignment of its own, and the one-pass lexer
                // reads that binder into the value. Transpiled CommonJS does
                // this on nearly every module, and the CST wrote the inner
                // value alone.
                if self.is_js() {
                    let binder = self.form(children[0]);
                    if binder != "0" {
                        return format!("[\"m\",[{binder},{value}]]");
                    }
                }
                value
            }
            // `f"{a(x)} b{c(y)}"` and `` `a${h(x)}b${g(y)}` ``: the text is not
            // a value, but each field's expression is, and the one-pass lexer's
            // string reader forms the same merge of them. So a call written in
            // a field is a fact on both paths, and a field with no call saves
            // nothing to compare.
            // Kotlin's `"$a ${f(b)}"` and Swift's `"\(f(b))"` are one
            // `string_literal` with an `interpolation` per hole, exactly as
            // Python's f-string and JavaScript's template are. Reading the
            // literal as text made every call in a hole invisible — the lexer's
            // string reader does see them, so the hole is the fact.
            "f_string" | "template_string" | "string_literal"
                if matches!(kind, "f_string" | "template_string")
                    || matches!(self.language.name, "kotlin" | "swift" | "scala") =>
            {
                // Python's replacement field is `interpolation`, JavaScript's
                // is `template_substitution`; everything else is the same
                // shape, and a template with no substitution is still `0`.
                let field = if matches!(kind, "f_string" | "string_literal") {
                    "interpolation"
                } else {
                    "template_substitution"
                };
                let mut parts = Vec::new();
                for child in self.children(id) {
                    // Kotlin's `$name` is a token, not a node: the grammar hands
                    // it over as `template_name`, and its text is the name
                    // between the `$`. Skipped, the hole became the constant 0
                    // and the lexer's `n:name` had nowhere to pair.
                    if self.kind(child) == "template_name" {
                        let text = self.text_string(child);
                        let name = text.trim_start_matches('$');
                        let name = if name == "this" { "self" } else { name };
                        if !name.is_empty() {
                            let mut out = "[\"n\",".to_string();
                            push_string(&mut out, name);
                            out.push(']');
                            parts.push(out);
                        }
                        continue;
                    }
                    if self.kind(child) != field {
                        continue;
                    }
                    // `{x!r:>{w}}` holds one expression; the conversion and the
                    // format spec are text, and their nested fields are
                    // `interpolation` nodes of their own, so they are reached
                    // from the spec, not from here. A template substitution's
                    // nested substitutions live inside a nested `template_string`
                    // node, which `form` reaches as the substitution's value.
                    // A leaf identifier is a value: requiring the part to have
                    // children of its own skipped `{a}` entirely and left the
                    // field as the constant 0.
                    let value = self.children(child).into_iter().find(|&part| {
                        !self.is_layout(part)
                            && !matches!(
                                self.kind(part),
                                "{" | "}" | ":" | "!" | "template_open" | "template_close"
                            )
                            && self.kind(part) != field
                    });
                    if let Some(value) = value {
                        parts.push(self.form(value));
                    }
                    for sub in self.children(child) {
                        if self.kind(sub) == field {
                            let inner = self.children(sub).into_iter().find(|&part| {
                                !self.is_layout(part)
                                    && !matches!(
                                        self.kind(part),
                                        "{" | "}" | ":" | "!" | "template_open" | "template_close"
                                    )
                                    && self.kind(part) != field
                            });
                            if let Some(inner) = inner {
                                parts.push(self.form(inner));
                            }
                        }
                    }
                }
                // Scala's `s"...${f(x)}"` names its interpolator: the lexer
                // reads the `s` as a name ahead of the fields, so the value is
                // `m:[n:s,...]`. Without it the one field was the whole value.
                if self.language.name == "scala" && !parts.is_empty() {
                    let prefix = self
                        .children(id)
                        .into_iter()
                        .find(|&child| self.kind(child) == "string_start")
                        .map(|child| self.text_string(child).trim_end_matches('"').to_string())
                        .unwrap_or_default();
                    if !prefix.is_empty() {
                        let mut out = "[\"n\",".to_string();
                        push_string(&mut out, &prefix);
                        out.push(']');
                        parts.insert(0, out);
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // `a to listOf(1)`: Kotlin's infix call is a value with two operands
            // and a name between them. Read through the default arm only the
            // first operand was formed, so a call written on the right of `to`,
            // `until`, `downTo` or any user infix function was never recorded -
            // every map entry built with `to` lost its value's call.
            "infix_expression" if self.is_kotlin() => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                if items.is_empty() {
                    "0".to_string()
                } else {
                    self.merge_form(&items)
                }
            }
            // `minBackoff * math.pow(2, n)`: Scala spells an infix expression as
            // a `binary` node. The default arm read only its first operand, so
            // a call on the right (`math.pow`, `a * f(b)`) was never recorded
            // and the binding lost the value the lexer wrote as a merge of both
            // operands. Form both sides like Kotlin's infix expression. Swift's
            // `a == f(b)` is the same node: an `if` on it lost its condition
            // and every call in it (`if lhs == Unicode.Scalar(x)`).
            "binary" if matches!(self.language.name, "scala" | "swift") => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
                if items.is_empty() {
                    "0".to_string()
                } else {
                    self.merge_form(&items)
                }
            }
            // `reportConnectFailure { … }`: Scala's argument block is a call to
            // the name before the brace, and the lexer files the line as that
            // name. The default arm read the node as nothing, so the event was
            // lost; the block's own statements are read by the walk.
            "block_argument" if self.language.name == "scala" => {
                match self.children(id).into_iter().find(|&child| !self.is_layout(child)) {
                    Some(callee) => self.form(callee),
                    None => "0".to_string(),
                }
            }
            // `ResettableValue<(Int, Int) -> Bool>`: a tuple type in a type
            // list is its element types, which the lexer reads as a tuple. The
            // generic arm below read the node as nothing, so `Int, Int` were
            // lost from the value.
            "tuple_type" if self.language.name == "swift" => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && !matches!(self.kind(child), "(" | ")" | ","))
                    .collect();
                if items.is_empty() {
                    "0".to_string()
                } else {
                    self.tuple_form(&items)
                }
            }
            // `UnsafeCell<State>`, `Vec<Entry>`: a generic type holds the base
            // and the type arguments, and both are names the lexer reads.
            "generic_type" | "generic_specialization" | "type_arguments" | "type_parameter_list" => {
                let mut parts = Vec::new();
                for child in self.children(id) {
                    if self.is_layout(child) || matches!(self.kind(child), "<" | ">" | "," | "::") {
                        continue;
                    }
                    let value = self.form(child);
                    if value != "0" {
                        parts.push(value);
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // `[u8; CLASSES]`: an array type holds a type name and a length, both
            // of them names the lexer reads.
            "array_type" | "slice_type" | "reference_type" | "pointer_type" => {
                let mut parts = Vec::new();
                for child in self.children(id) {
                    if self.is_layout(child) || matches!(self.kind(child), "[" | "]" | "&" | "*" | ";" ) {
                        continue;
                    }
                    let value = self.form(child);
                    if value != "0" {
                        parts.push(value);
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // Rust's and JavaScript's lexers have no keyword table for these:
            // `true` is a name, and an object literal `{ value: true }` records
            // one where the CST was writing a constant.
            "true" | "false" if self.is_rust() || self.is_js() => {
                let text = self.text_string(id);
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &text);
                out.push(']');
                out
            }
            // Kotlin's one-pass reader has no keyword table either, so
            // `enableSsl = false` files `n:false` and `x = null` files `n:null`
            // while the CST wrote the constant 0 - every boolean and null
            // initialiser on a line the lexer reads as a statement disagreed.
            "true" | "false" | "null" if self.is_kotlin() => {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &self.text_string(id));
                out.push(']');
                out
            }
            // JavaScript reads `null` and `undefined` as names too: there is no
            // keyword table on the lexer's side, so `obj.__proto__ = null`
            // records `n:null` while the CST wrote the constant 0.
            "null" | "undefined" if self.is_js() => {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &self.text_string(id));
                out.push(']');
                out
            }
            // C# spells its three constants as keywords and the one-pass
            // reader writes each of them as a name (`n:true`, `n:null`). A
            // *generic* method's body reaches this shared walk instead of
            // `c_function`, so `return true;` inside `T M<T>()` formed as the
            // constant 0 and lost its value on every member with a type
            // parameter list, while the same statement in a non-generic method
            // was right.
            "true" | "false" | "null" if self.language.name == "csharp" => {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &self.text_string(id));
                out.push(']');
                out
            }
            "string" | "string_literal" | "number" | "integer" | "float" | "true" | "false"
            | "none" | "null" | "undefined" | "regex" | "comment"
            | "arrow_function" | "function_expression" | "generator_function" => "0".to_string(),
            // `WindowGroup { ... }`: Swift's trailing closure call has the
            // callee as its first expression child; when the callee is a bare
            // identifier leaf (`WindowGroup`), the generic fallback skips it
            // because `first_child == NONE`.
            "trailing_closure" => self
                .expression_child(id)
                .map_or_else(|| "0".to_string(), |child| self.form(child)),
            // `.init({ _ in stringCtx })`: a Swift closure is its parameters and
            // the names its body reads, which is how the lexer writes it. Read
            // as the default arm the whole closure was the constant 0, so every
            // name a closure body read dropped out of the call that took it.
            // The body is walked as statements of its own, so its names are
            // read here without forming it again (a second form would record
            // its calls twice).
            "closure" if self.language.name == "swift" => {
                let mut names: Vec<String> = Vec::new();
                let mut stack: Vec<u32> = self.children(id);
                stack.reverse();
                while let Some(child) = stack.pop() {
                    if self.is_layout(child) {
                        continue;
                    }
                    if self.kind(child) == "identifier" {
                        let text = self.text_string(child);
                        let mut form = String::from("[\"n\",");
                        push_string(&mut form, &text);
                        form.push(']');
                        names.push(form);
                        continue;
                    }
                    let mut kids = self.children(child);
                    kids.reverse();
                    stack.extend(kids);
                }
                match names.len() {
                    0 => "0".to_string(),
                    1 => names.remove(0),
                    _ => format!("[\"m\",[{}]]", names.join(",")),
                }
            }
            // `class Action extends _AttributeHolder(Function) {}`: the body is
            // deferred, but the `extends` heritage expression is evaluated when
            // the class expression is evaluated.
            "class" => match self.children(id).into_iter().find(|&child| self.kind(child) == "class_heritage") {
                Some(heritage) => self
                    .expression_child(heritage)
                    .map_or_else(|| "0".to_string(), |child| self.form(child)),
                None => "0".to_string(),
            },
            // A region the grammar could not name. The tokens are still there —
            // recovery keeps every byte — so the values inside it are read like
            // any other: `("prefix " f"{call(x)}")` is an ERROR today because
            // implicit concatenation of a literal with an f-string is not a
            // rule, and skipping the region dropped the call inside it.
            "ERROR" => {
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| !self.is_layout(child) && self.node(child).first_child != NONE)
                    .collect();
                // The tokens inside a region the grammar could not name are
                // still there, and a call is visible in them: a name followed by
                // an argument list. Reading that is not inventing a fact — it is
                // reading what recovery left — and it is the only way a call
                // inside `("prefix " f"{call(x)}")` survives, since implicit
                // concatenation of a literal with an f-string is not a rule yet.
                self.recovered_calls(id);
                match items.len() {
                    0 => "0".to_string(),
                    1 => self.form(items[0]),
                    _ => self.merge_form(&items),
                }
            }
            // A keyword reused as an identifier (`match`, `type`, `case`)
            // carries that keyword as its kind; the spelling is the name.
            _ if self.leaf_name(id).is_some() => {
                let text = self.leaf_name(id).unwrap_or_default();
                let text = if text == "this" { "self".to_string() } else { text };
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &text);
                out.push(']');
                out
            }
            _ => {
                // A node with no mapping: its first value-bearing child, so a
                // wrapper the grammar added costs nothing.
                match self.expression_child(id) {
                    Some(child) if self.node(child).first_child != NONE => self.form(child),
                    _ => "0".to_string(),
                }
            }
        }
    }


    /// Calls visible in the raw tokens of an `ERROR` region: `name(` or
    /// `a.b(`. Nothing is invented — the name and the argument list are both in
    /// the source, and the region is only reached where the grammar failed to
    /// build a node for them.
    pub(crate) fn recovered_calls(&mut self, id: u32) {
        let tokens: Vec<u32> = self.children(id);
        let mut at = 0;
        while at < tokens.len() {
            if self.kind(tokens[at]) != "identifier" {
                at += 1;
                continue;
            }
            // `a.b(` walks the dotted chain forward before the paren, and the
            // name keeps every component: the resolver matches on the path.
            let mut end = at;
            let mut name = self.text_string(tokens[at]);
            while end + 2 < tokens.len()
                && matches!(self.kind(tokens[end + 1]), "." | "?.")
                && self.kind(tokens[end + 2]) == "identifier"
            {
                name.push('.');
                name.push_str(&self.text_string(tokens[end + 2]));
                end += 2;
            }
            if end + 1 < tokens.len() && self.kind(tokens[end + 1]) == "(" {
                let line = self.line(tokens[at]);
                self.push_call(name, line);
            }
            at = end + 1;
        }
    }

    pub(crate) fn tuple_form(&mut self, items: &[u32]) -> String {
        let mut out = "[\"t\",[".to_string();
        for (index, &item) in items.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&self.form(item));
        }
        out.push_str("]]");
        out
    }

    /// A Swift literal's elements in order, with the `,` and `:` separators
    /// kept: Swift wraps them in one `collection_body` node.
    fn swift_collection_children(&self, id: u32) -> Vec<u32> {
        self.children(id)
            .into_iter()
            .flat_map(|child| {
                if self.kind(child) == "collection_body" {
                    self.children(child)
                } else {
                    vec![child]
                }
            })
            .collect()
    }

    /// Whether a Swift literal holds a `key: value` pair, i.e. a `:` among its
    /// direct elements. `[:]` is an empty dictionary and has no key.
    fn swift_has_pair_colon(&self, id: u32) -> bool {
        self.swift_collection_children(id).into_iter().any(|child| self.kind(child) == ":")
    }

    pub(crate) fn merge_form(&mut self, items: &[u32]) -> String {
        let mut out = "[\"m\",[".to_string();
        for (index, &item) in items.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&self.form(item));
        }
        out.push_str("]]");
        out
    }

    /// A Kotlin call with a trailing lambda inside an expression body: the
    /// one-pass reader merges the callee (or parenthesized call) with the
    /// lambda body rather than dropping the trailing lambda.
    fn kotlin_expr_call_form(&mut self, id: u32) -> Option<String> {
        let children = self.children(id);
        let lam = children
            .iter()
            .copied()
            .find(|&child| matches!(self.kind(child), "lambda_literal" | "annotated_lambda" | "labeled_lambda"))?;
        let has_args = children
            .iter()
            .any(|&child| matches!(self.kind(child), "argument_list" | "arguments"));
        let head = if has_args {
            self.call_form(id)
        } else {
            let callee = children.iter().copied().find(|&child| {
                !matches!(
                    self.kind(child),
                    "argument_list" | "arguments" | "type_arguments" | "(" | ")" | "lambda_literal" | "annotated_lambda" | "labeled_lambda"
                )
            })?;
            let form = self.form(callee);
            let path = self.path_of(callee);
            if !path.is_empty() {
                self.push_call(path, self.call_line(callee, id));
            }
            form
        };
        let lam_form = self.form(lam);
        Some(format!("[\"m\",[{head},{lam_form}]]"))
    }

    /// The name of a Kotlin DSL call: an identifier followed by a single
    /// lambda argument, which the lexer reads as the name alone.
    fn kotlin_dsl_name(&self, id: u32) -> Option<String> {
        let children = self.children(id);
        let callee = children
            .iter()
            .copied()
            .find(|&child| !matches!(self.kind(child), "argument_list" | "arguments" | "type_arguments" | "(" | ")"))?;
        if !matches!(self.kind(callee), "identifier" | "simple_identifier") {
            return None;
        }
        // The grammar keeps a trailing lambda as a direct child of the call,
        // not inside an `arguments` node, which is why the check below has to
        // look at both.
        let args: Vec<u32> = children
            .into_iter()
            .filter(|&child| {
                matches!(
                    self.kind(child),
                    "lambda_literal" | "annotated_lambda" | "lambda_expression" | "argument_list" | "arguments"
                )
            })
            .flat_map(|child| {
                if matches!(self.kind(child), "argument_list" | "arguments") {
                    self.children(child)
                } else {
                    vec![child]
                }
            })
            .filter(|&arg| !self.is_layout(arg))
            .collect();
        if args.len() != 1 || !matches!(self.kind(args[0]), "lambda_literal" | "annotated_lambda") {
            return None;
        }
        Some(self.text_string(callee))
    }

    pub(crate) fn call_form(&mut self, id: u32) -> String {
        self.call_form_link(id).0
    }

    /// A call's form, and — when the callee is a name — the index it was
    /// recorded under together with the receiver it is called on, so the call
    /// above it can name its receiver without guessing: `A().run` links `.run`
    /// to `(0, "0")`, the call `A`.
    pub(crate) fn call_form_link(&mut self, id: u32) -> (String, Option<(u32, String)>) {
        let children = self.children(id);
        let is_new = self.kind(id) == "new_expression";
        let callee = children
            .iter()
            .copied()
            .find(|&child| !matches!(self.kind(child), "new" | "argument_list" | "arguments" | "type_arguments" | "(" | ")"))
            .unwrap_or(id);
        // `type("H", (), {...})()`: the callee is a value produced by another
        // call. That inner call is the fact and its arguments are where the
        // dictionary (and everything in it) lives; the outer call has no name
        // of its own, so it adds none. `callee != id` keeps a recovered call
        // whose own callee is itself from descending forever.
        // Scala's `new X(args)` is a `call` whose callee is a `new_expression`
        // holding only `new` and the type name; the arguments live on the outer
        // node. Unwrapping to the inner form dropped every argument, so each
        // construction read as `c(0;[])` against the lexer's `c(0;[n:arg])`.
        // Read it as a plain call named after the type instead.
        let nested_callee = matches!(self.kind(callee), "call" | "call_expression")
            || (self.kind(callee) == "new_expression" && self.language.name != "scala");
        if callee != id && nested_callee {
            let inner = self.form(callee);
            if let Some(list) = children.iter().copied().find(|&child| matches!(self.kind(child), "argument_list" | "arguments")) {
                self.arguments(list);
            }
            return (inner, None);
        }
        // `f().g()`: the receiver is a call, and it is a fact of its own. The
        // call's own index and receiver go in the slot above it, so the record
        // says which call produced the receiver: `A().run` records `A`, `.run`
        // and the link between them. The calls array alone cannot say it — in
        // `TaintAnalyzer(a, b(), c).run()` the argument `b()` is recorded
        // between the two — and the path keeps the name the source spells.
        let mut receiver_call = None;
        // Scala's `select` is `object.name`: `s"...$x".format(y)` has the
        // interpolated string as its receiver, and that receiver is a value.
        let scala_select = self.language.name == "scala" && self.kind(callee) == "select";
        if scala_select || matches!(self.kind(callee), "attribute" | "member_expression" | "member_access_expression" | "member_access" | "navigation_expression") {
            if let Some(&object) = self.children(callee).first() {
                if self.named_call(object) {
                    // `new URL(x).toString()`, `A().b()`, `A().b().c()`: a
                    // receiver that is a call the walk records, whose own
                    // receiver is carried along so a chain stays readable one
                    // call at a time.
                    // The receiver is the inner call's whole form, arguments
                    // included: rebuilding it from the link alone wrote `[]` for
                    // every argument and lost `Path(path)` in `Path(path).resolve()`.
                    let (form, _link) = self.call_form_link(object);
                    receiver_call = Some(form);
                } else {
                    // `[...xs].filter(f)`, `handlers[k](x)`: any expression,
                    // whose calls are facts of their own. Its value is also the
                    // receiver the lexer writes: `(a or b).split('-')` has
                    // `a or b` there, and dropping it lost every name the
                    // receiver held.
                    let value = self.form(object);
                    let object_path = self.path_of(object);
                    if object_path.is_empty() || object_path.starts_with('.') {
                        receiver_call = Some(value);
                    }
                }
            }
        } else if !matches!(self.kind(callee), "identifier" | "property_identifier" | "type_identifier") {
            // `(_SockFilter * len(filters))(*filters)`, `handlers[k](x)` and
            // `f()()`: a callee that is not a plain name is an expression, and
            // the calls inside it are facts. A name has none, so it is skipped.
            let value = self.form(callee);
            // `foo(K).map(f)`: a Scala field callee whose base is a call has
            // a path that starts with `.` (no name before it), so the base's
            // value is the receiver and its arguments must be kept too.
            let path = self.path_of(callee);
            if path.is_empty() || path.starts_with('.') {
                receiver_call = Some(value);
            }
        }
        let (path, mut receiver) = self.call_target(callee, receiver_call);
        // A function literal is not a name: `go func() { … }()` has no callee to
        // record, and writing a call with an empty path invented an entry the
        // lexer never had — the literal's own invocation is not an edge.
        if path.is_empty() && matches!(self.kind(callee), "func_literal") {
            return ("0".to_string(), None);
        }
        let line = self.call_line(callee, id);
        let args = children
            .iter()
            .copied()
            .find(|&child| matches!(self.kind(child), "argument_list" | "arguments" | "template_string"));
        let mut targ = None;
        if let Some(list) = args.filter(|&list| self.kind(list) == "template_string") {
            let inner = self.form(list);
            if path.is_empty() && inner != "0" {
                receiver = format!("[\"m\",[{receiver},{inner}]]");
            } else if inner != "0" {
                targ = Some(format!("[{inner}]"));
            }
        }
        // `new Product { ExpiryDate = new DateTime(2000, 9, 28) }`: an object
        // initialiser is not an argument list, so nothing walked its items and
        // every call written inside one was lost - `DateTime` alone cost 58
        // files. Each item's value is a value of its own, and a lambda among
        // them keeps its own reading, which `c_calls_in_pub` already honours.
        let mut nested = Vec::new();
        for child in children.iter().copied() {
            if matches!(self.kind(child), "object_initializer" | "array_initializer" | "collection_initializer") {
                self.c_calls_in_pub(child, &mut nested);
            }
        }
        for call in nested {
            self.form(call);
        }
        let index = self.push_call(path, line);
        let mut arg_list = match args {
            Some(list) if self.kind(list) == "template_string" => targ.unwrap_or_else(|| "[]".to_string()),
            Some(list) => self.arguments(list),
            None => "[]".to_string(),
        };
        if self.is_kotlin() && !self.in_kotlin_expr_body {
            let mut trailing = Vec::new();
            for &child in &children {
                if matches!(self.kind(child), "lambda_literal" | "annotated_lambda" | "labeled_lambda") {
                    trailing.push(self.form(child));
                }
            }
            if !trailing.is_empty() {
                if arg_list == "[]" {
                    arg_list = format!("[{}]", trailing.join(","));
                } else if arg_list.ends_with(']') {
                    arg_list.pop();
                    arg_list.push(',');
                    arg_list.push_str(&trailing.join(","));
                    arg_list.push(']');
                }
            }
        }
        // `ResettableValue<(Int, Int) -> Bool>({ $0 == $1 })`: a generic
        // specialization called with arguments is not a call to the lexer. It
        // writes the type's names and the arguments' values as one value, so
        // the form is that value and no `c`.
        if self.language.name == "swift" && self.kind(callee) == "generic_specialization" && arg_list.len() > 2 {
            let base = self.form(callee);
            let inner = arg_list[1..arg_list.len() - 1].to_string();
            let out = format!("[\"m\",[{base},{inner}]]");
            return (out, Some((index as u32, receiver)));
        }
        let out = format!("[\"c\",{index},{receiver},{arg_list}]");
        let _ = is_new;
        let link = (index as u32, receiver);
        (out, Some(link))
    }

    /// The called path and the receiver form: `f(x)` -> (`f`, `0`),
    /// `a.b.c(x)` -> (`a.b.c`, `["n","a.b"]`), `A().b()` -> (`.b`,
    /// `["c",0,0,[]]`) with the index of the `A` call the walk recorded.
    pub(crate) fn call_target(&self, callee: u32, receiver_call: Option<String>) -> (String, String) {
        match self.kind(callee) {
            "subscript" | "subscript_expression" | "index_expression" if !self.path_of(callee).is_empty() => {
                match self.children(callee).first().copied() {
                    Some(inner) => self.call_target(inner, receiver_call),
                    None => (String::new(), "0".to_string()),
                }
            }
            // `.macOS("10.10")`: an implicit member has no receiver, and the
            // call is named by the member alone. Read through the default arm
            // it had no name at all, so the call the lexer records was missing.
            "implicit_member_expression" => (self.member_name(callee), "0".to_string()),
            // `s.go()` / `s->go()` in the C family: the grammar writes a field
            // access as `field_expression`, and read through the default arm
            // the call had no name at all.
            "attribute" | "member_expression" | "member_access_expression" | "member_access"
            | "conditional_member_access" | "navigation_expression" | "field_expression" | "select" => {
                let children = self.children(callee);
                let name = self.member_name(callee);
                let object = children.first().copied();
                let receiver_path = object.map(|node| self.path_of(node)).unwrap_or_default();
                let path = if receiver_path.is_empty() {
                    format!(".{name}")
                } else {
                    format!("{receiver_path}.{name}")
                };
                let receiver = if !receiver_path.is_empty() && !receiver_path.starts_with('.') {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &receiver_path);
                    out.push(']');
                    out
                } else {
                    receiver_call.unwrap_or_else(|| "0".to_string())
                };
                (path, receiver)
            }
            // Scala spells a qualified type in a `new` as flat children
            // (`new` / `java` / `.` / `util` / `.` / `ArrayList` /
            // `type_arguments`). Reading only the first identifier named the
            // call `java`, so every `new java.util.X[T](..)` lost its call.
            "new_expression" if self.language.name == "scala" => {
                let parts: Vec<String> = self
                    .children(callee)
                    .into_iter()
                    .filter(|&child| matches!(self.kind(child), "identifier" | "type_identifier"))
                    .map(|child| self.text_string(child))
                    .collect();
                let path = parts.join(".");
                // The lexer writes a qualified constructor with the owner in
                // the receiver slot (`c(n:java.util;[..])`), so split the dotted
                // type the same way a `select` callee is split.
                let receiver = if parts.len() > 1 {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &parts[..parts.len() - 1].join("."));
                    out.push(']');
                    out
                } else {
                    "0".to_string()
                };
                (path, receiver)
            }
            "call" | "call_expression" | "new_expression" => {
                // `f()(...)`: the callee is an inner call, and the name is the
                // inner call's own callee. It has to be looked for *inside*
                // `callee`: a call node reaches this arm as its own callee when
                // the grammar recovers a call with no operand, and recursing on
                // the same index is an infinite descent that takes the process
                // stack out (a four kilobyte fuzz file is enough).
                let inner = self.children(callee).into_iter().find(|&child| {
                    !matches!(self.kind(child), "(" | ")" | "argument_list" | "arguments" | "type_arguments" | "new")
                });
                match inner {
                    Some(inner) => {
                        let (path, _) = self.call_target(inner, None);
                        (path, "0".to_string())
                    }
                    None => (String::new(), "0".to_string()),
                }
            }
            _ => {
                let path = self.path_of(callee);
                let receiver = if path.is_empty() {
                    receiver_call.unwrap_or_else(|| "0".to_string())
                } else {
                    "0".to_string()
                };
                (path, receiver)
            }
        }
    }

    /// The property name of an `a.b` node: the last leaf after the dot. It is
    /// read by position, not by kind, because a member called `get` carries the
    /// grammar's `get` kind and a private one carries `#`.
    pub(super) fn member_name(&self, id: u32) -> String {
        for &child in self.children(id).iter().rev() {
            if self.node(child).first_child != NONE {
                continue;
            }
            let text = self.text(child);
            if text.is_empty() {
                continue;
            }
            if text[0].is_ascii_alphanumeric() || text[0] == b'_' || text[0] == b'#' || text[0] == b'$' || text[0] == b'`' {
                return String::from_utf8_lossy(text).trim_start_matches('#').to_string();
            }
        }
        String::new()
    }

    /// A dotted path, recording the calls on the way: `urlparse(x).netloc` has
    /// a call under the attribute, and that call is a fact.
    pub(crate) fn form_path(&mut self, id: u32) -> String {
        let path = self.path_of(id);
        let kind = self.kind(id);
        let children = self.children(id);
        if !path.is_empty() && !path.starts_with('.') {
            if matches!(kind, "attribute" | "member_expression" | "subscript" | "subscript_expression" | "field_expression") {
                if let Some(&object) = children.first() {
                    // A receiver can be any expression; nothing is recorded for a
                    // plain name, so this costs a walk and never a wrong fact.
                    self.form(object);
                }
            }
        }
        // `name[len(prefix):]`: the index is an expression of its own.
        if matches!(kind, "subscript" | "subscript_expression") {
            for &child in children.iter().skip(1) {
                if !self.is_layout(child) {
                    self.form(child);
                }
            }
        }
        path
    }

    /// The line a call is written on. For `path\n  .replace(...)` that is the
    /// line of `replace`, which is what a reader points at; Tree-sitter's node
    /// start would say the line of `path`.
    pub(crate) fn call_line(&self, callee: u32, call: u32) -> u32 {
        match self.kind(callee) {
            // `ParametersBuilder().apply { … }.build()`: a chain broken over
            // lines is one navigation expression that starts on the first
            // line, and the lexer files the call on the line its member name
            // is written. Kotlin spells the member access `navigation_expression`
            // and C# spells it `member_access`: a chain written over several
            // lines - `RemoteExecutor.Invoke(x,\n  new Options { .. })
            // .DisposeAsync();` - was dated to the receiver's first line, and the
            // one-pass reader dates a call to its name (84 files).
            "attribute" | "member_expression" | "navigation_expression" | "member_access"
            | "selector_expression" | "field_access" => {
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

    /// Whether a child of a binary, ternary or slice node is an operand: not
    /// layout, or a word the layout filter took for a keyword (`type`, `class`,
    /// `default`) that cannot be one of the operators this arm merges.
    fn binary_operand(&self, child: u32) -> bool {
        if !self.is_layout(child) {
            return true;
        }
        let text = self.text_string(child);
        let starts_a_word = text.as_bytes().first().is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_');
        starts_a_word
            && !matches!(
                text.as_str(),
                "in" | "instanceof" | "of" | "as" | "satisfies" | "typeof" | "void" | "delete" | "new" | "await" | "yield"
            )
    }

    pub(crate) fn arguments(&mut self, list: u32) -> String {
        let mut out = "[".to_string();
        let mut first = true;
        for child in self.children(list) {
            let kind = self.kind(child);
            // Python's `...` is an anonymous token, so it reads as layout, but
            // the one-pass lexer reads it as one argument of value 0: `type(...)`
            // is `c(0;0;[0])` there and `c(0;0;[])` here. Every EllipsisType and
            // every stub default of `...` fell back on the difference.
            let ellipsis = matches!(kind, "ellipsis" | "Ellipsis" | "...");
            if self.is_layout(child) && !ellipsis {
                continue;
            }
            if !first {
                out.push(',');
            }
            first = false;
            // `ArgumentNotNull(value, nameof(value))`: `nameof(…)` is a call to
            // the lexer wherever it is written, and only an initialiser was
            // read for it - every guard clause in the C# corpus lost it.
            if kind == "nameof_expression" {
                let line = self.line(child);
                self.push_call("nameof".to_string(), line);
            }
            // Go and Java do not use `form`: an argument that is itself a call
            // (`path.Join(os.Getenv(..))`) would come back as `0`.
            let value = if ellipsis {
                "0".to_string()
            } else if kind == "labeled_argument" || (kind == "named_argument" && (self.is_kotlin() || self.language.name == "scala")) {
                // `Package(name: "x", platforms: [.macOS("10")])`: a Swift
                // labeled argument is a keyword argument to the lexer -
                // `[\"k\",name,value]` - and read as a plain form it came back
                // as `0`, which every manifest line failed on. Scala's
                // `copy(c = g(y))` is the same keyword form to the lexer, and
                // read as a plain form the call on the right of `=` was lost.
                let parts = self.children(child);
                let name = parts.first().map(|&node| self.path_of(node)).unwrap_or_default();
                let inner = parts.last().map(|&node| self.form(node)).unwrap_or_else(|| "0".to_string());
                let mut keyed = "[\"k\",".to_string();
                push_string(&mut keyed, &name);
                keyed.push(',');
                keyed.push_str(&inner);
                keyed.push(']');
                keyed
            } else if self.kinds.family == Family::Plain
                && (kind == "assignment"
                    // Kotlin's annotation writes its named argument as an
                    // `assignment_expression` (`@Deprecated("x", level = …)`),
                    // and the lexer files it as the keyword form there too, so
                    // read as a merge it lost the name off every annotation.
                    || (kind == "assignment_expression"
                        && (self.is_kotlin() || self.kind(list) == "annotation_arguments")))
            {
                // `@deprecated("old", since = "2")`: a named argument is the
                // lexer's keyword form, not the assignment's merge.
                // Kotlin's annotations write the same argument with an
                // `assignment_expression` (`@Deprecated(level = X)`) where its
                // call arguments use `named_argument`, and that shape formed as
                // the value alone - the key `level` left the record.
                let parts = self.children(child);
                let name = parts.first().map(|&node| self.path_of(node)).unwrap_or_default();
                let inner = parts.last().map(|&node| self.form(node)).unwrap_or_else(|| "0".to_string());
                let mut out = "[\"k\",".to_string();
                push_string(&mut out, &name);
                out.push(',');
                out.push_str(&inner);
                out.push(']');
                out
            } else {
                match self.kinds.family {
                    Family::Go | Family::Java => self.decl_value(child),
                    _ => self.form(child),
                }
            };
            out.push_str(&value);
        }
        out.push(']');
        out
    }

    /// A dotted name off the source: `a . b` and `a.b` are the same path, and
    /// `this` becomes `self` as the flow form spells it.
    pub(crate) fn path_of(&self, id: u32) -> String {
        if let Some(name) = self.soft_name(id) {
            return name;
        }
        if self.is_rust() {
            if let Some(path) = self.rust_path(id) {
                return path;
            }
        }
        match self.kind(id) {
            "identifier" | "property_identifier" | "private_property_identifier" | "type_identifier"
            | "shorthand_property_identifier" | "this" | "super" => {
                let text = self.text_string(id);
                if text == "this" {
                    "self".to_string()
                } else {
                    text.trim_start_matches('#').to_string()
                }
            }
            "labeled_this" if self.is_kotlin() => "self".to_string(),
            // `JObject.Parse`: the C# grammar spells a member access with two
            // name leaves, and read as a leaf the whole receiver vanished -
            // every call through a type in the corpus lost its dotted name.
            // `_reader?.Close()` - the null-conditional member access names the
            // same path as a plain one.
            "attribute" | "member_expression" | "member_access" | "conditional_member_access"
            // Scala spells `x.y` as `select`; without it a dotted call callee
            // path was empty, so every `a.b(c)` call lost its name.
            | "select"
            // Kotlin spells `x.y` as `navigation_expression`. Read through the
            // default arm it named only its first leaf, so `DeprecationLevel.ERROR`
            // in a value formed as `DeprecationLevel`.
            | "navigation_expression"
            | "qualified_type" => {
                let children = self.children(id);
                let name = self.member_name(id);
                let base = children.first().map(|&child| self.path_of(child)).unwrap_or_default();
                if base.is_empty() {
                    format!(".{name}")
                } else {
                    format!("{base}.{name}")
                }
            }
            // `.macOS("10.10")` — an implicit member: the name is the member,
            // with no receiver. Read as a leaf the call lost its name entirely.
            "implicit_member_expression" => self.leaf_name(id).unwrap_or_default(),
            "meta_property" => self.text_string(id).chars().filter(|ch| !ch.is_whitespace()).collect(),
            // `MainKtsScript::class.java`: a class literal as a navigation
            // receiver names `<base>.class`, not empty.
            "class_literal" if self.is_kotlin() => {
                let base = self.children(id).first().map(|&child| self.path_of(child)).unwrap_or_default();
                if base.is_empty() {
                    String::new()
                } else {
                    format!("{base}.class")
                }
            }
            // A subscript is transparent in a path: `elements[i].innerHTML`
            // names `elements.innerHTML`, and `x[0].y` names `x.y`. Rust spells
            // it `index_expression`, and reading it as a leaf dropped the
            // receiver of every indexed assignment target.
            "subscript" | "subscript_expression" | "index_expression" => {
                let base = self.children(id).first().map(|&child| self.path_of(child)).unwrap_or_default();
                base
            }
            // `AbstractList<String>()` in a supertype constructor: the path of
            // a generic type is its base type's path without type arguments.
            "generic_type" => self.children(id).first().map(|&child| self.path_of(child)).unwrap_or_default(),
            "parenthesized_expression" | "try_expression" | "await_expression" => {
                // `(a, b)[i]`: parens around a comma list hold a value, not a
                // name. Reading the first item as a path dropped the rest of the
                // tuple, so traceback's `(self.filename, self.lineno, ...)[0]`
                // and imaplib's `(time.timezone, time.altzone)[dst]` formed as
                // the first name alone.
                if self.kind(id) == "parenthesized_expression"
                    && self.children(id).into_iter().filter(|&child| !self.is_layout(child)).count() > 1
                {
                    return String::new();
                }
                self.expression_child(id).map(|child| self.path_of(child)).unwrap_or_default()
            }
            // `(this as Foo).bar()` / `(this as? Foo)?.bar`: Kotlin's cast
            // includes the target type in its value, so collapsing `as_expression`
            // to its operand path drops the target type from the receiver.
            "as_expression" if self.is_kotlin() => String::new(),
            "as_expression" | "satisfies_expression" | "type_assertion" | "non_null_expression" | "not_null_expression" => {
                self.expression_child(id).map(|child| self.path_of(child)).unwrap_or_default()
            }
            // A call has no name of its own: `f().x` names `.x`.
            "call" | "call_expression" | "new_expression" | "invocation_expression" => String::new(),
            // `match.group`, `type.name`: a keyword reused as an identifier.
            _ => self.leaf_name(id).map(|text| if text == "this" { "self".to_string() } else { text }).unwrap_or_default(),
        }
    }

    /// The path of a value used as an assignment target, or empty when the
    /// target is not a name (a literal index, a call).
    pub(crate) fn value_path(&self, id: u32) -> Option<String> {
        if let Some(name) = self.soft_name(id) {
            return Some(name);
        }
        match self.kind(id) {
            "identifier" | "attribute" | "member_expression" | "subscript" | "subscript_expression" | "this" => {
                let path = self.path_of(id);
                (!path.is_empty()).then_some(path)
            }
            "tuple_pattern" | "pattern_list" | "array_pattern" | "list_pattern" | "object_pattern" => None,
            _ => self.leaf_name(id).filter(|path| !path.is_empty()),
        }
    }
}
