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
            }
            // `var emptyJSON = JSONText("{}")` binds the name to the class it
            // converts to, as `emptyJSON := JSONText("{}")` does in `go_assign`.
            // The value is the spec's one expression, possibly in a list.
            let values: Vec<u32> = parts[equal + 1..]
                .iter()
                .flat_map(|&part| match self.kind(part) {
                    "expression_list" => self.children(part).into_iter().filter(|&child| !self.is_layout(child)).collect(),
                    _ => vec![part],
                })
                .collect();
            if let [value] = values[..] {
                if let Some(class) = self.constructed(value) {
                    self.facts.vars.push((declared[0].clone(), class));
                }
            }
        }
        for name in declared {
            self.lexer_event(b'a', line, std::slice::from_ref(&name), &value, false);
        }
    }

    fn go_find_func_literal(&self, nodes: &[u32]) -> Option<u32> {
        for &node in nodes {
            match self.kind(node) {
                "func_literal" => return Some(node),
                "call_expression" => {
                    if let Some(callee) = self.children(node).into_iter().find(|&c| !self.is_layout(c)) {
                        if self.kind(callee) == "func_literal" {
                            return Some(callee);
                        }
                    }
                }
                "expression_list" => {
                    let non_layout: Vec<u32> = self.children(node).into_iter().filter(|&c| !self.is_layout(c)).collect();
                    if non_layout.len() == 1 {
                        if let Some(found) = self.go_find_func_literal(&non_layout) {
                            return Some(found);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn go_bind_func_literal(&mut self, target: &str, literal: u32, line: u32) {
        if target.is_empty() || target == "self" {
            return;
        }
        let params = self
            .child_of_kind(literal, "parameter_list")
            .map(|list| self.params(literal, list))
            .unwrap_or_default();
        let index = self.add_symbol(target.to_string(), String::new(), KIND_FUNCTION, line, params);
        self.go_literal_frames.insert(literal, index);
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
            let text = if self.language.name == "csharp" && text.starts_with(b"@") && text.len() > 1 && text[1] != b'"' {
                &text[1..]
            } else {
                text
            };
            let first = text[0];
            // Java's float has no digit before the dot (`.005`), so read by its
            // first character alone it was punctuation and returned nothing —
            // `assertEquals(1.0, x, .005, "msg")` lost the argument. A dot that
            // is followed by a digit is the numeric literal the lexer writes as
            // the constant 0.
            let leading_dot_number = first == b'.' && text.len() > 1 && text[1].is_ascii_digit();
            if first.is_ascii_digit() || first == b'"' || first == b'\'' || first == b'`' || leading_dot_number {
                return "0".to_string();
            }
            // Go identifiers may be Unicode too: `stream{ƒ: ƒ}` names `ƒ`.
            if first.is_ascii_alphabetic() || first == b'_' || (matches!(self.language.name, "csharp" | "go") && first >= 0x80) {
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
        // The same flat, numeric-free merge `form` writes for java, for the
        // values that arrive through the declaration path (`int x = a + 1`,
        // an `if` condition, a `return a + b`).
        if kind == "binary_expression" && self.language.name == "java" {
            let mut items = Vec::new();
            self.java_binary(id, &mut items);
            return match items.len() {
                0 => "0".to_string(),
                1 => items.pop().unwrap_or_else(|| "0".to_string()),
                _ => format!("[\"m\",[{}]]", items.join(",")),
            };
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
                if child_kind == "new" {
                    continue;
                }
                // `new int[] { foo() }`: an array initializer can contain calls
                // and expressions that must be walked, just like `dimensions_expr`.
                if matches!(child_kind, "dimensions_expr" | "array_initializer") {
                    let value = self.decl_value(child);
                    if !value.is_empty() && value != "0" {
                        parts.push(value);
                    }
                    continue;
                }
                if let Some(name) = self.leaf_name(child) {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &name);
                    out.push(']');
                    parts.push(out);
                } else {
                    let value = self.decl_value(child);
                    if !value.is_empty() && value != "0" {
                        parts.push(value);
                    }
                }
            }
            return match parts.len() {
                0 => "0".to_string(),
                1 => parts.pop().unwrap_or_else(|| "0".to_string()),
                _ => format!("[\"m\",[{}]]", parts.join(",")),
            };
        }
        // `{ 'a' }`, `{ 1, 2 }`, `{ { 1 }, { 2 } }`: an array initialiser is the
        // list of its elements, and the one-pass reader writes it as one merge
        // whatever the count (`m:[0]`, `m:[0,0]`, `m:[[m,[0]],[m,[0]]]`). Read by
        // the generic arm below, a one-element initialiser collapsed to the
        // element and came back as the literal `0` — the same value a plain
        // `= 1` writes — and a nested one lost its own list (java: `= { 1 }`,
        // 11 corpus rows across the guava array fields, protobuf's byte tables
        // and commons-lang's DiffBuilderTest).
        if self.language.name == "java" && kind == "array_initializer" {
            let mut parts = Vec::new();
            for child in self.children(id) {
                let value = self.decl_value(child);
                if !value.is_empty() {
                    parts.push(value);
                }
            }
            return match parts.len() {
                0 => "0".to_string(),
                _ => format!("[\"m\",[{}]]", parts.join(",")),
            };
        }
        if matches!(kind, "call_expression" | "method_invocation" | "object_creation_expression" | "object_creation" | "invocation_expression") {
            let callee = self.decl_callee(id);
            if self.language.name == "csharp" && kind == "object_creation" && matches!(self.kind(callee), "array_type" | "generic_type") {
                let mut parts = Vec::new();
                for child in self.children(id) {
                    if self.kind(child) == "new" {
                        continue;
                    }
                    let v = self.decl_value(child);
                    if !v.is_empty() && v != "0" {
                        parts.push(v);
                    }
                }
                return match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.pop().unwrap_or_else(|| "0".to_string()),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                };
            }
            if self.language.name == "csharp"
                && kind == "object_creation"
                && !self.children(id).iter().any(|&c| matches!(self.kind(c), "arguments" | "argument_list"))
                && self.children(id).iter().any(|&c| matches!(self.kind(c), "object_initializer" | "array_initializer" | "collection_initializer"))
            {
                let mut parts = Vec::new();
                for child in self.children(id) {
                    if self.kind(child) == "new" {
                        continue;
                    }
                    let v = self.decl_value(child);
                    if !v.is_empty() && v != "0" {
                        parts.push(v);
                    }
                }
                return match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.pop().unwrap_or_else(|| "0".to_string()),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                };
            }
            // `NewBuilder(foo).WithCount(count(baz)).Build()`, `makeFn(prep(a))(b)`,
            // `getItems(makeKey(x))[pickIdx(x)].Render()`: when the receiver chain
            // carries calls of its own, form that receiver expression directly so
            // every intermediate call, its arguments, and its names reach the record.
            let receiver = self.decl_call_receiver(id, callee);
            let index = self.decl_call(id);
            let args = self
                .children(id)
                .into_iter()
                .find(|&child| matches!(self.kind(child), "arguments" | "argument_list"))
                .map(|list| self.arguments(list))
                .unwrap_or_else(|| "[]".to_string());
            let call = format!("[\"c\",{index},{receiver},{args}]");
            // `gitRepo{}.gitWithinDir(x)`: a method called on a composite literal.
            // The receiver is not a call, so `decl_receiver_expr` leaves it out
            // and the record had no `gitRepo` at all. The lexer reads the literal
            // as its own name beside the call (`m:[n:gitRepo, c(...)]`), so the
            // literal is written the same way here.
            if self.language.name == "go" && kind == "call_expression" && self.kind(callee) == "selector_expression" {
                if let Some(object) = self.children(callee).into_iter().find(|&child| !self.is_layout(child)) {
                    if self.kind(object) == "composite_literal" {
                        let literal = self.decl_value(object);
                        return format!("[\"m\",[{literal},{call}]]");
                    }
                }
            }
            // `() -> new BeanE() { @Override public void afterPropertiesSet() { … } }`:
            // a body that builds an anonymous class keeps the class's members —
            // the one-pass reader merges the creation with the body it opens
            // (`m:[c(0;[]), m:[m:[n:Override,n:public,c(0;[]),m:[c(0;[0])]]]]`) —
            // while the call alone dropped them wherever the creation was read
            // as a value (`context.registerBean(name, BeanE.class, () -> new
            // BeanE() { … })`: 2 rows of the java corpus, spring-context and
            // junit-platform). `java_argument_value` already wrote the merge for
            // an argument; this is the same merge for every other value path.
            if self.kinds.family == Family::Java && kind == "object_creation_expression" {
                if let Some(body) = self.child_of_kind(id, "class_body") {
                    let members = self.java_anon_body(body);
                    if !members.is_empty() && members != "0" {
                        return format!("[\"m\",[{call},{members}]]");
                    }
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
        // `Car.class`: the node is its dotted path and nothing else. Pushed by
        // the member list below and then read child by child as well, the
        // value came out `m:[Car.class,Car,class]` - three names for one.
        if kind == "class_literal" {
            let path = self.decl_path(id);
            if !path.is_empty() {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &path);
                out.push(']');
                return out;
            }
        }
        // `catch (IOException e) { … }` read as a value: the one-pass reader
        // reads the declaration as a call of the keyword with the exception as
        // its argument (see `java_catch_clause`).
        if self.language.name == "java" && kind == "catch_clause" {
            return self.java_catch_clause(id);
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
                // A Java dotted name whose base names something and whose
                // leaves spell the path is one name to the one-pass reader
                // (`n:Label.FLAG_DEBUG_ONLY`, `n:BigInteger.class`). Descending
                // into its children as well wrote the path and each segment
                // beside it
                // (`m:[n:Label.FLAG_DEBUG_ONLY,n:Label,n:FLAG_DEBUG_ONLY]`), a
                // value the lexer never writes (java: ClassReader,
                // JdbcClientQueryTests). A cast receiver keeps the descent: the
                // lexer writes the cast's own names beside the member, and Go's
                // selectors are its own shape.
                if self.language.name == "java" && !self.decl_has_call(id) {
                    let leaves: Vec<String> =
                        self.children(id).iter().filter_map(|&child| self.leaf_name(child)).collect();
                    let base_named = self
                        .children(id)
                        .first()
                        .is_some_and(|&first| !self.decl_path_seg(first, true).is_empty());
                    if base_named && !leaves.is_empty() && leaves.join(".") == path {
                        return parts.pop().unwrap_or_else(|| "0".to_string());
                    }
                }
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
        let mut callee_form: Option<String> = None;
        let mut type_args: Vec<String> = Vec::new();
        let mut args_form: Option<String> = None;
        let mut trailing: Vec<String> = Vec::new();
        for (index, &child) in children.iter().enumerate() {
            let kind = self.kind(child);
            if index == 0 {
                // `new Action(() => Map(x)).Go<Ex>(y)`: the callee is a member
                // access whose receiver is itself a call. The name alone dropped
                // that receiver, so `Action` lost its row. Form the receiver first,
                // so its call is recorded before the outer call's arguments, as
                // the source reads.
                if self.language.name == "csharp" && self.kind(child) == "member_access" {
                    let object = self.children(child).into_iter().find(|&part| !self.is_layout(part) && self.kind(part) != ".");
                    if let Some(object) = object.filter(|&object| self.value_root_call(object).is_some()) {
                        let value = self.form(object);
                        if !value.is_empty() && value != "0" {
                            trailing.push(value);
                        }
                    }
                }
                let path = self.path_of(child);
                if !path.is_empty() && !path.starts_with('.') {
                    if matches!(self.kind(child), "member_access" | "conditional_member_access" | "element_access") {
                        if let Some(&object) = self.children(child).first() {
                            self.form(object);
                        }
                    }
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, &path);
                    out.push(']');
                    callee_form = Some(out);
                } else {
                    let mut out = self.form(child);
                    if path.starts_with('.') {
                        let name = path.trim_start_matches('.');
                        if !name.is_empty() {
                            let mut n = "[\"n\",".to_string();
                            push_string(&mut n, name);
                            n.push(']');
                            out = if out == "0" || out.is_empty() {
                                n
                            } else {
                                format!("[\"m\",[{out},{n}]]")
                            };
                        }
                    }
                    if !out.is_empty() && out != "0" {
                        callee_form = Some(out);
                    }
                }
                continue;
            }
            if kind == "type_arguments" {
                for arg in self.children(child) {
                    if matches!(self.kind(arg), "<" | ">" | ">>" | ",")
                        || (self.is_layout(arg) && self.leaf_name(arg).is_none())
                    {
                        continue;
                    }
                    let form = self.form(arg);
                    if !form.is_empty() && form != "0" {
                        type_args.push(form);
                    } else if let Some(word) = self.leaf_name(arg) {
                        let mut out = "[\"n\",".to_string();
                        push_string(&mut out, &word);
                        out.push(']');
                        type_args.push(out);
                    }
                }
                continue;
            }
            if matches!(kind, "arguments" | "argument_list") {
                self.delim_depth += 1;
                let mut items: Vec<String> = Vec::new();
                for arg in self.children(child) {
                    if self.is_layout(arg) {
                        continue;
                    }
                    items.push(self.form(arg));
                }
                self.delim_depth -= 1;
                match items.len() {
                    0 => {}
                    1 => args_form = Some(items.pop().unwrap_or_else(|| "0".to_string())),
                    _ => args_form = Some(format!("[\"t\",[{}]]", items.join(","))),
                }
                continue;
            }
            if self.is_layout(child) {
                continue;
            }
            let value = self.form(child);
            if !value.is_empty() && value != "0" {
                trailing.push(value);
            }
        }
        // Merge helper that flattens a nested `["m",[...]]` (e.g. from a generic
        // type argument like `IEnumerable<Color>`) so it matches the one-pass
        // reader's flat merge.
        let push_flat = |out: &mut Vec<String>, form: String| {
            if let Some(inner) = form.strip_prefix("[\"m\",[").and_then(|s| s.strip_suffix("]]")) {
                out.push(inner.to_string());
            } else {
                out.push(form);
            }
        };
        let merge_vec = |items: Vec<String>| -> String {
            match items.len() {
                0 => "0".to_string(),
                1 => items.into_iter().next().unwrap_or_else(|| "0".to_string()),
                _ => format!("[\"m\",[{}]]", items.join(",")),
            }
        };
        // With two or more type arguments (`Foo<A, B>(x)`), the one-pass reader
        // splits on `,` outside parentheses into a tuple: `m:[Foo, A]` then
        // `m:[B, x]`.
        if type_args.len() >= 2 {
            let mut tuple = Vec::with_capacity(type_args.len());
            let last_idx = type_args.len() - 1;
            for (i, targ) in type_args.into_iter().enumerate() {
                let mut elem = Vec::new();
                if i == 0 {
                    if let Some(c) = callee_form.take() {
                        push_flat(&mut elem, c);
                    }
                }
                push_flat(&mut elem, targ);
                if i == last_idx {
                    let a = args_form.take().unwrap_or_else(|| "0".to_string());
                    elem.push(a);
                    elem.extend(std::mem::take(&mut trailing));
                }
                tuple.push(merge_vec(elem));
            }
            return format!("[\"t\",[{}]]", tuple.join(","));
        }
        let mut parts = Vec::new();
        if let Some(c) = callee_form {
            push_flat(&mut parts, c);
        }
        for targ in type_args {
            push_flat(&mut parts, targ);
        }
        let a = args_form.unwrap_or_else(|| "0".to_string());
        parts.push(a);
        parts.extend(trailing);
        merge_vec(parts)
    }

    /// One call, at the line its callee is written on, named the way the source
    /// spells it. Returns its index in the symbol's call table, which is what
    /// the flow form's `["c",id,...]` names.
    pub(crate) fn decl_call(&mut self, id: u32) -> usize {
        let callee = self.decl_callee(id);
        let path = self.decl_path(callee);
        // `"content".getBytes()`, `"/echo".length()`: a Java member call whose
        // base names nothing has no qualified path, and the one-pass reader
        // spells that `.getBytes` - the marker the value check reads as "the
        // receiver is not a name". Written bare, the name claimed a free
        // function and the receiver value rendered as a call the lexer never
        // wrote (java: FileCopyUtilsTests, UtcDateTypeAdapter, SockJsServiceTests,
        // HeaderMethodArgumentResolver).
        let path = if self.language.name == "java"
            && !path.is_empty()
            && !path.starts_with('.')
            && self.decl_base_unnamed(callee)
