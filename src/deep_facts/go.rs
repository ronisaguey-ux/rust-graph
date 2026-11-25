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
                    }
                }
            }
        }
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
        let all = self.children(id);
        let mut found_eq = false;
        for (idx, &child) in all.iter().enumerate() {
            if self.kind(child) == "=" {
                found_eq = true;
                let Some(&rhs) = all[idx + 1..].iter().find(|&&c| !self.is_layout(c) && self.kind(c) != "ref") else {
                    continue;
                };
                if let Some(target) = self.cs_decl_target(&all[..idx]) {
                    self.cs_bind_constructed(target, rhs);
                }
            }
        }
        if found_eq {
            return;
        }
        let children: Vec<u32> = all.into_iter().filter(|&child| !self.is_layout(child)).collect();
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
        let Some(target) = self.cs_decl_target(&children[..children.len() - 1]) else {
            return;
        };
        self.cs_bind_constructed(target, initializer);
    }

    /// Extract the target name from the prefix before `=` in a C# declaration.
    /// Skips trailing `[...]` when an element-access assignment (`bounds[i] = ...`)
    /// was parsed as a declaration, accepts C# contextual keywords (`and`,
    /// `file`, `set`, ...), and strips the verbatim `@` prefix (`@event`).
    pub(crate) fn cs_decl_target(&self, prefix: &[u32]) -> Option<String> {
        let mut i = prefix.len();
        let mut bracket_depth = 0usize;
        while i > 0 {
            i -= 1;
            let child = prefix[i];
            let kind = self.kind(child);
            if matches!(kind, "whitespace" | "comment") {
                continue;
            }
            if kind == "]" {
                bracket_depth += 1;
                continue;
            }
            if kind == "[" {
                bracket_depth = bracket_depth.saturating_sub(1);
                continue;
            }
            if bracket_depth > 0 {
                continue;
            }
            let is_name = kind == "identifier"
                || matches!(kind, "member_access" | "field_access")
                || (self.language.name == "csharp"
                    && (self.kinds.soft.contains(&kind)
                        || matches!(
                            kind,
                            "file"
                                | "set"
                                | "remove"
                                | "get"
                                | "init"
                                | "add"
                                | "value"
                                | "and"
                                | "or"
                                | "not"
                                | "when"
                                | "where"
                                | "with"
                                | "from"
                                | "into"
                                | "let"
                                | "join"
                                | "on"
                                | "equals"
                                | "by"
                                | "ascending"
                                | "descending"
                                | "group"
                                | "select"
                                | "order"
                                | "alias"
                                | "async"
                                | "await"
                                | "dynamic"
                                | "nameof"
                                | "notnull"
                                | "unmanaged"
                                | "managed"
                                | "required"
                                | "scoped"
                                | "args"
                        )));
            if is_name {
                let raw = if matches!(kind, "member_access" | "field_access") {
                    self.path_of(child)
                } else {
                    self.text_string(child)
                };
                let clean = if self.language.name == "csharp" {
                    raw.strip_prefix('@').unwrap_or(&raw).to_string()
                } else {
                    raw
                };
                return (!clean.is_empty()).then_some(clean);
            }
        }
        None
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
                && matches!(self.text_string(child).as_str(), "=" | ":=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "&^=")
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
            // The binder must be a plain name. `indexers[nodeIndex] = func(obj any) …`
            // binds through an index expression, and the target read as the map's
            // name filed the literal under `indexers`: a function symbol the
            // lexer never opens, so every statement of that body was orphaned
            // (kubernetes.go newIndexedPodsInformer: 10 rows).
            let binder = children[..at].iter().copied().find(|&child| !self.is_layout(child));
            let plain_name = binder.is_some_and(|child| {
                let kind = self.kind(child);
                kind == "identifier"
                    || self.kinds.soft.contains(&kind)
                    // `db.head.memTruncationCallBack = func() {`: a selector is
                    // a name the lexer opens a symbol for too, so only targets
                    // that are not names (`m["a"]`) are left out.
                    || matches!(kind, "selector_expression" | "field_access" | "this")
            });
            if plain_name {
                if let Some(literal) = self.go_find_func_literal(&children[at + 1..]) {
                    let line = self.line(id);
                    self.go_bind_func_literal(target, literal, line);
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
        if declared.len() == 1 {
            if let Some(literal) = self.go_find_func_literal(&parts[equal + 1..]) {
                self.go_bind_func_literal(&declared[0], literal, line);
