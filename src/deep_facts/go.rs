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
        {
            format!(".{path}")
        } else {
            path
        };
        let line = self.call_line(callee, id);
        self.push_call(path, line)
    }


    /// Whether a dotted callee names only its last segment: the base is no
    /// name at all (`"content".getBytes()`), so nothing qualifies the member.
    fn decl_base_unnamed(&self, id: u32) -> bool {
        if !matches!(
            self.kind(id),
            "field_access"
                | "member_access"
                | "conditional_member_access"
                | "selector_expression"
                | "navigation_expression"
        ) {
            return false;
        }
        let Some(&first) = self.children(id).first() else { return false };
        if self.kind(first) == "." {
            return false;
        }
        self.decl_path_seg(first, true).is_empty()
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
                // `((Map) actual).get("foo1")`, `((Mono<?>) responsePublisher).block(t)`:
                // a parenthesised cast for a receiver holds no call, and the
                // one-pass reader reads it as the merge of the cast's type and
                // its operand. Left out, the receiver was the constant 0 and the
                // cast's names were nowhere in the record (spring's
                // JacksonJsonViewTests and HttpServiceMethod, commons-lang's
                // EqualsBuilderTest: 6 rows).
                let cast = matches!(self.kind(object), "parenthesized_expression" | "cast_expression");
                (object != call && (cast || self.decl_has_call(object))).then_some(object)
            }
            // `Ordering.from(k).<K>onKeys()`: Java's `method_invocation` with
            // `type_arguments` keeps the receiver as a direct child of `call`
            // rather than wrapping it in `field_access`.
            _ if self.kind(call) == "method_invocation" => {
                let object = self.children(call).into_iter().find(|&child| !self.is_layout(child))?;
                // A cast receiver, for the invocations this grammar keeps the
                // object as a direct child of: same reading as the arm above.
                let cast = matches!(self.kind(object), "parenthesized_expression" | "cast_expression");
                (object != callee && (cast || self.decl_has_call(object))).then_some(object)
            }
            _ => None,
        }
    }

    /// The receiver value of a call: the object expression's own value when it
    /// is a call, and otherwise the dotted path the one-pass reader writes.
    pub(crate) fn decl_call_receiver(&mut self, call: u32, callee: u32) -> String {
        if let Some(expr) = self.decl_receiver_expr(call, callee) {
            return self.decl_value(expr);
        }
        // `Collections.<Class<?>>emptySet()`: Java keeps the type-argument list
        // between the receiver and the method name, inside the invocation's own
        // children. The one-pass reader merges the receiver's name and the type
        // argument into the call's receiver (`c(0;m:[n:Collections,n:Class];[])`),
        // and read from the callee's path alone the receiver was the constant 0
        // with the type argument nowhere in the record (mockito's
        // TypeCachingMockBytecodeGeneratorTest: 2 rows).
        if self.language.name == "java"
            && self.kind(call) == "method_invocation"
            && self.child_of_kind(call, "type_arguments").is_some()
        {
            let mut parts: Vec<String> = Vec::new();
            for child in self.children(call) {
                if child == callee || matches!(self.kind(child), "arguments" | ".") {
                    continue;
                }
                let value = self.decl_value(child);
                if !value.is_empty() && value != "0" {
                    parts.push(value);
                }
            }
            return match parts.len() {
                0 => "0".to_string(),
                1 => parts.pop().unwrap_or_else(|| "0".to_string()),
                _ => format!("[\"m\",[{}]]", parts.join(",")),
            };
        }
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
        self.decl_path_seg(id, true)
    }

    /// The path a dotted name writes, remembering whether `id` is the leftmost
    /// segment of it. Only that segment can be the receiver: renaming every
    /// leaf whose text matches the receiver spelled `cc.cc.mu` as
    /// `self.self.mu`, a value the lexer never writes (transport.go:2959).
    fn decl_path_seg(&self, id: u32, root: bool) -> String {
        match self.kind(id) {
            "identifier" | "type_identifier" | "field_identifier" | "this" | "super" => {
                let text = self.text_string(id);
                let receiver = self.frame().receiver.as_str();
                if text == "this" || (root && !receiver.is_empty() && text == receiver) {
                    "self".to_string()
                } else {
                    text
                }
            }
            // `Sup.super.isLongLived()`: the qualified `super` of an overriding
            // class, spelled like `qualified_this` and dropped the same way, so
            // the call's receiver was the constant 0 (java: Task.java's
            // `SchedulingAwareRunnable.super.isLongLived()`).
            "qualified_super" => {
                let mut parts: Vec<String> = Vec::new();
                for child in self.children(id) {
                    let part = match self.kind(child) {
                        "identifier" | "type_identifier" | "super" => self.text_string(child),
                        _ => continue,
                    };
                    parts.push(part);
                }
                parts.join(".")
            }
            // `Outer.this.x`: an inner class's qualified receiver, and the type
            // prefix is part of the name the one-pass reader builds. It spells
            // the `this` verbatim in a value (`n:Outer.this.f`) and as `self` in
            // a target list (`Outer.self.f = v`), which `decl_target` rewrites.
            // With no arm the node named nothing at all, so the receiver of
            // every call through it was dropped and a target list lost its
            // prefix (java: Task.java's three `Task.this.lastExecutionOutcome`
            // writes, FrameworkServlet's `FrameworkServlet.this.onApplicationEvent(event)`,
            // GroovyMarkupConfigurer's `GroovyMarkupConfigurer.this.resolveTemplate(…)`).
            "qualified_this" => {
                let mut parts: Vec<String> = Vec::new();
                for child in self.children(id) {
                    let part = match self.kind(child) {
                        "identifier" | "type_identifier" | "this" => self.text_string(child),
                        _ => continue,
                    };
                    parts.push(part);
                }
                parts.join(".")
            }
            "selector_expression" | "qualified_type" | "field_access" | "scoped_identifier" | "dotted_name"
            | "scoped_type_identifier" | "nested_type_identifier" | "class_literal" | "method_reference"
            // Kotlin's `user.title` in a template hole is a navigation; the
            // path is the receiver and the member, like every other dotted name.
            | "navigation_expression"
            // `JObject.Parse`: the C# member access carries two name leaves and
            // no dotted kind at all, so every call through a type lost its
            // receiver - 592 rows of one C# slice.
            | "member_access"
            // `button.Tag?.ToString()`: a null-conditional member access names
            // the same dotted path as a plain one when reached via a field or
            // local initializer.
            | "conditional_member_access" => {
                let children = self.children(id);
                // The last segment is read by its text: `Mockito.when` names a
                // member called `when`, which this grammar gives a keyword kind.
                let name = children
                    .iter()
                    .copied()
                    .rev()
                    .find(|&child| self.kind(child) != "." && self.leaf_name(child).is_some())
                    .map(|child| self.decl_path_seg(child, false))
                    .unwrap_or_default();
                let base = children
                    .first()
                    .copied()
                    .filter(|&first| self.kind(first) != ".")
                    .map(|first| self.decl_path_seg(first, root))
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
                self.decl_path_seg(callee, root)
            }
            "call_expression" => {
                let callee = self.decl_callee(id);
                if callee == id {
                    return String::new();
                }
                self.decl_path_seg(callee, root)
            }
            "index_expression" | "array_access" => {
                self.children(id).first().map(|&base| self.decl_path_seg(base, root)).unwrap_or_default()
            }
            // `[]byte(x)`, `*T`, `map[K]V`: a conversion names its type, and the
            // lexer reads the type's own name as the callee.
            "array_type" | "pointer_type" | "map_type" | "slice_type" | "channel_type" => self
                .last_name(id)
                .or_else(|| self.leaf_name(id))
                .unwrap_or_default(),
            "parenthesized_expression" | "unary_expression" | "type_assertion" => self
                .expression_child(id)
                .map(|child| self.decl_path_seg(child, root))
                .unwrap_or_default(),
            // `((InheritMe) from).privateInherited = "foo"`: the one-pass reader
            // spells a cast as its type and its value with nothing between them
            // (`InheritMefrom.privateInherited`). Read through the leaf fallback
            // the cast named nothing at all, so the target lost its receiver and
            // the assignment left the record (mockito's LenientCopyToolTest, 2
            // rows).
            "cast_expression" => {
                let mut out = String::new();
                for child in self.children(id) {
                    if self.is_layout(child) {
                        continue;
                    }
                    out.push_str(&self.decl_path_seg(child, root));
                }
                out
            }
            // `(InheritMe) from`: the type half of a cast.
            "cast_type" => self.last_name(id).or_else(|| self.leaf_name(id)).unwrap_or_default(),
            "generic_type" => {
                self.children(id).first().map(|&base| self.decl_path_seg(base, root)).unwrap_or_default()
            }
            _ => self.leaf_name(id).unwrap_or_default(),
        }
    }

    /// The name an assignment target binds: `x`, `this.index`, `xs[0]` -> `xs`
    /// (`x, y := ...` splits into two children before this is called).
    /// A selector whose receiver chain runs through a call, `flags.Lookup("h").Hidden`.
    /// The call has no name of its own, so the target is the member after the
    /// call: `.Hidden`, as the lexer writes it. `decl_path` named the callee's
    /// chain instead and read the methods `PersistentFlags` and `Lookup` as
    /// fields, which made every such assignment a target list the lexer never wrote.
    fn go_chain_has_call(&self, id: u32) -> bool {
        let mut at = id;
        loop {
            match self.kind(at) {
                "selector_expression" | "field_access" | "member_access" => match self.children(at).first() {
                    Some(&base) => at = base,
                    None => return false,
                },
                // Java and C# spell a call with their own node kinds, so
                // `readLabel(offset, labels).flags = v` read as a path through
                // the method's name instead of the member after the call
                // (java: ClassReader).
                "call_expression"
                | "method_invocation"
                | "invocation_expression"
                | "object_creation_expression"
                | "object_creation" => return true,
                _ => return false,
            }
        }
    }

    /// The name a selector writes after the last call in its receiver chain.
    /// `a.B().C.D` names `.C.D`; `f().x` names `.x`.
    fn go_call_tail(&self, id: u32) -> String {
        let children = self.children(id);
        let name = children
            .iter()
            .copied()
            .rev()
            .find(|&child| self.kind(child) != "." && self.leaf_name(child).is_some())
            .map(|child| self.decl_path(child))
            .unwrap_or_default();
        let head = match children.first().copied() {
            Some(base)
                if matches!(
                    self.kind(base),
                    "call_expression"
                        | "method_invocation"
                        | "invocation_expression"
                        | "object_creation_expression"
                        | "object_creation"
                ) =>
            {
                String::new()
            }
            Some(base) if matches!(self.kind(base), "selector_expression" | "field_access" | "member_access") && self.go_chain_has_call(base) => {
                self.go_call_tail(base)
            }
            Some(base) => self.decl_path(base),
            None => String::new(),
        };
        if head.is_empty() {
            format!(".{name}")
        } else {
            format!("{head}.{name}")
        }
    }

    pub(crate) fn decl_target(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "selector_expression" | "field_access" if self.go_chain_has_call(id) => {
                let path = self.go_call_tail(id);
                (!path.is_empty()).then_some(path)
            }
            "identifier" | "field_identifier" | "this" | "selector_expression" | "field_access" => {
                let path = self.decl_path(id);
                // `Outer.this.f = v` binds `Outer.self.f`: the lexer writes a
                // qualified receiver as `this` in a value and as `self` in a
                // target list, and `decl_path` writes the value spelling.
                let path = if path.contains(".this") {
                    path.split('.').map(|segment| if segment == "this" { "self" } else { segment }).collect::<Vec<_>>().join(".")
                } else {
                    path
                };
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
            // A contextual keyword bound as a name: `to = high - 1` in a
            // quickselect. Its kind is its own spelling, so the arms above never
            // matched it and the target list came back empty.
            kind if self.kinds.soft.contains(&kind) => {
                let path = self.decl_path(id);
                (!path.is_empty()).then_some(path)
            }
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
                "member_expression" | "attribute" | "member" | "safe_member" | "member_access_expression" | "nullsafe_member_access_expression" | "scoped_property_access_expression" | "instance_variable" | "class_variable" | "global_variable" | "absolute_constant" => {
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
        let Some(child) = self.expression_child(id) else { return };
        self.statement_node(id, child);
    }

    /// One expression of a statement list, dispatched by its kind. A grammar
    /// that keeps several expressions in one node (Erlang's `body`) calls this
    /// once per child; everything else reaches it through `expression_statement`.
    pub(crate) fn statement_node(&mut self, id: u32, mut child: u32) {
        // Swift `var x: T { get set }`: the grammar reads `get` as a statement
        // and leaves `set` beside it as an ERROR leaf; the lexer files both
        // accessors as one event, so the pair is written here.
        if self.language.name == "swift" && matches!(self.kind(child), "identifier") {
            let next = self.node(id).next_sibling;
            if next != NONE
                && self.kind(next) == "ERROR"
                && self.children(next).len() == 1
            {
                let word = self.children(next)[0];
                let first = self.text_string(child);
                let second = self.text_string(word);
                if matches!((first.as_str(), second.as_str()), ("get", "set") | ("set", "get")) {
                    let line = self.line(id);
                    let mut atoms = String::new();
                    for text in [&first, &second] {
                        if !atoms.is_empty() {
                            atoms.push(',');
                        }
                        atoms.push_str("[\"n\",");
                        push_string(&mut atoms, text);
                        atoms.push(']');
                    }
                    self.event(&format!("[\"x\",{line},[\"m\",[{atoms}]]]"));
                    return;
                }
            }
        }
        // `unsafe x = y` (Swift 6.2): the marker wraps the whole assignment, so
        // the statement is the assignment inside it, as `try` would be.
        while self.language.name == "swift" && self.kind(child) == "unsafe_expression" {
            let Some(inner) = self.expression_child(child) else { return };
            child = inner;
        }
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
        if self.language.name == "csharp" && self.kind(child) == "lambda_expression" {
            self.cs_lambda_assignment_symbol(child);
        }
        match self.kind(child) {
            // Erlang spells a binding `match_expression`: `X = f(Y)`. Read
            // through the default arm it formed the variable and dropped the
            // call on its right, so every binding whose value is a call lost it.
            "assignment" | "assignment_expression" => self.assignment(child, false),
            "match_expression" if self.facts.tag == "ERL" => self.assignment(child, false),
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
        self.assignment_at(id, augmented, self.line(id));
    }

    pub(crate) fn assignment_at(&mut self, id: u32, augmented: bool, line: u32) {
        let children = self.children(id);
        let Some(&first) = children.first() else { return };
        let Some(&last) = children.last() else { return };
        if first == last {
            return;
        }
        // `expression_statement` dispatches `assignment_expression` here
        // directly, bypassing the walk arm in `mod.rs`, so a statement-level
        // lambda assignment (`_handler = x => ...;` or `cfg.P = x => ...;`)
        // must also declare its function symbol here.
        if !augmented && self.language.name == "csharp" {
            self.cs_lambda_symbol(id);
        }
        if std::env::var("RG_ASSIGN_DEBUG").is_ok() {
            let dump: Vec<String> = children.iter().map(|&c| format!("{}:{:?}", self.kind(c), self.text_string(c).chars().take(8).collect::<String>())).collect();
            eprintln!("assign: {:?}", dump);
        }
        // The left-hand side can itself carry calls (e.g. `c[f() ? 0 : 1] = v`);
        // `assignment_targets` only extracts the target name, so walk `first`
        // as an expression to record those calls in source order.
        let _ = self.form(first);
        let targets = self.assignment_targets(first);
        // C# spells a subscripted target `element_access`.
        let weak = augmented || matches!(self.kind(first), "subscript" | "subscript_expression" | "index_expression" | "element_access");
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
            "subscript" | "subscript_expression" | "index_expression" | "element_access" | "element_reference" | "index" | "dereference" => {
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
                // A C# declarator names one identifier; a dotted target is a
                // member being assigned, not a variable being declared.
                if !(self.language.name == "csharp" && target.contains('.')) {
                    self.facts.vars.push((target.clone(), class.clone()));
                }
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
        if matches!(self.kind(call), "new_expression" | "const_object_expression" | "object_creation" | "object_creation_expression") {
            let path = if matches!(self.kind(call), "new_expression" | "const_object_expression") {
                self.children(call).into_iter().find_map(|child| {
                    match self.kind(child) {
                        "identifier" | "member_expression" | "attribute" | "name" => Some(self.path_of(child)),
                        "qualified_name" => Some(self.text_string(child)),
                        _ => None,
                    }
                })?
            } else {
                self.decl_path(self.decl_callee(call))
            };
            let last = path.rsplit(|c| c == '.' || c == '\\').next().unwrap_or(&path);
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
            // C# spells calls as `invocation_expression` / `generic_invocation`
            // and `new T()` as `object_creation` / `object_creation_expression`.
            "call" | "call_expression" | "new_expression" | "const_object_expression" | "invocation_expression" | "object_creation"
            | "object_creation_expression" | "generic_invocation" => Some(id),
            "attribute" | "member_expression" | "member_access" | "null_aware_member" | "field_access" | "subscript"
            | "subscript_expression" | "parenthesized_expression" => {
                self.children(id).first().and_then(|&object| self.value_root_call(object))
            }
            "await_expression" if self.facts.tag == "DART" => {
                self.children(id).into_iter().rev().find(|&c| !self.is_layout(c)).and_then(|c| self.value_root_call(c))
            }
            // `candidate = Path(x).parent / candidate`: a binary value starts
            // with its left operand, which is what the lexer's first token is.
            "binary_operator" | "binary_expression" | "boolean_operator" | "conditional_expression"
            | "ternary_expression" => self.children(id).first().and_then(|&left| self.value_root_call(left)),
            // Swift spells the same node `binary` (and `a ?? b` as
            // `nil_coalescing`). Without them the CST reader gave up on a whole
            // file whose only unusual node was one of these: Double.swift had no
            // CST record at all, and the audit could only report a single generic
            // loss for it. Reading them costs per-line rows on the files that now
            // parse, and that is the honest trade - a parsed file with named
            // disagreements beats an unparsed one.
            "binary" | "nil_coalescing" if self.language.name == "swift" => {
                self.children(id).first().and_then(|&left| self.value_root_call(left))
            }
            // `new Foo() as any`, `new Foo() satisfies T`, `new Foo()!`: a type
            // assertion or non-null postfix does not change the constructed class.
            "as_expression" | "satisfies_expression" | "non_null_expression" => {
                self.expression_child(id).and_then(|expr| self.value_root_call(expr))
            }
            // Swift spells the same shape `cast`/`force_unwrap`. Guarded on the
            // language so no other grammar's node of that name is misread.
            "cast" | "force_unwrap" if self.language.name == "swift" => {
                self.expression_child(id).and_then(|expr| self.value_root_call(expr))
            }
            _ => None,
        }
    }

    /// The callee at the root of a call chain: `a.b().c()` -> `a.b`, `f()()` ->
    /// `f`, `a.b()` -> `a.b`.
    pub(crate) fn root_callee(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "call" | "call_expression" | "new_expression" | "invocation_expression" | "generic_invocation" => {
                let callee = self.children(id).into_iter().find(|&child| {
                    !matches!(
                        self.kind(child),
                        "new" | "argument_list" | "arguments" | "type_arguments" | "type_parameter_list" | "(" | ")"
                    )
                })?;
                (callee != id).then(|| self.root_callee(callee)).flatten()
            }
            "attribute" | "member_expression" | "navigation_expression" | "member_access" | "field_access" => {
                let object = self.children(id).first().copied()?;
                let path = self.path_of(id);
                // `Config.Array(...).pipe` starts with a call: unwrap to that
                // call so the class is the one the lexer's first tokens name.
                // Otherwise the whole path is the answer: `Context.Reference`
                // constructs Reference, not Context, and returning the object
                // root reported every namespaced constructor as a lost var.
                if path.contains("()")
                    || matches!(
                        self.kind(object),
                        "call" | "call_expression" | "new_expression" | "invocation_expression"
                            | "object_creation" | "object_creation_expression"
                    )
                {
                    return self.root_callee(object);
                }
                let path = if self.facts.tag == "PAS" { path.to_ascii_lowercase() } else { path };
                (!path.is_empty()).then_some(path)
            }
            _ => {
                let path = self.path_of(id);
                let path = if self.facts.tag == "PAS" { path.to_ascii_lowercase() } else { path };
                (!path.is_empty()).then_some(path)
            }
        }
    }

    // -- control flow -------------------------------------------------------

    fn swift_condition_item(&mut self, item: u32) -> String {
        if self.kind(item) == "binding_condition" {
            if let Some(binding) = self.child_of_kind(item, "condition_binding") {
                let parts = self.children(binding);
                if let Some(at) = parts.iter().position(|&part| self.kind(part) == "=") {
                    if let (Some(&first), Some(&value)) = (parts.first(), parts.get(at + 1)) {
                        let targets = self.pattern_names(first);
                        if targets.len() == 1 && self.kind(value) != "type_annotation" {
                            let bound = self.form(value);
                            let mut name = "[\"n\",".to_string();
                            push_string(&mut name, &targets[0]);
                            name.push(']');
                            return format!("[\"m\",[{name},{bound}]]");
                        }
                    }
                }
            }
            return "0".to_string();
        }
        self.form(item)
    }

    pub(crate) fn condition_before(&mut self, id: u32) {
        // `if a, b, c {`: Swift condition lists have multiple comma-separated
        // clauses before `block`, and the lexer records all of them as a tuple.
        // Reading only the first child dropped every clause after the first.
        if self.language.name == "swift" {
            let items: Vec<u32> = self
                .children(id)
                .into_iter()
                .take_while(|&child| !matches!(self.kind(child), "block" | "else_clause"))
                .filter(|&child| !self.is_layout(child) && self.kind(child) != ",")
                .collect();
            if items.len() > 1 {
                let parts: Vec<String> = items.iter().map(|&item| self.swift_condition_item(item)).collect();
                if parts.iter().any(|part| part != "0") {
                    let mut expr = items[0];
                    while self.kind(expr) == "parenthesized_expression" {
                        match self.expression_child(expr) {
                            Some(inner) => expr = inner,
                            None => break,
                        }
                    }
                    let line = self.line(expr);
                    self.event(&format!("[\"x\",{line},[\"t\",[{}]]]", parts.join(",")));
                }
                return;
            }
        }
        // `if let credentials = store.credentials(for: space) {` with the body on the following
        // lines: the condition is a `binding_condition`, and `form` on that kind writes the
        // constant 0, so the whole binding was silently dropped. Written inline the loss hid
        // behind the body's own event on the same line, which is why it needed a two-line
        // difference to show.
        if self.language.name == "swift" {
            if let Some(condition) = self.child_of_kind(id, "binding_condition") {
                if let Some(binding) = self.child_of_kind(condition, "condition_binding") {
                    let parts = self.children(binding);
                    if let Some(at) = parts.iter().position(|&part| self.kind(part) == "=") {
                        if let (Some(&first), Some(&value)) = (parts.first(), parts.get(at + 1)) {
                            let targets = self.pattern_names(first);
                            if targets.len() == 1 && self.kind(value) != "type_annotation" {
                                // A conditional binding is the assignment a
                                // plain `let` writes, and the lexer reads it as
                                // one (`a result = value`); `guard let … else`
                                // already writes the same event here. Written as
                                // a pair of its own the binding matched nothing
                                // the lexer wrote, so its line was reported as a
                                // loss (Diffing.swift:418).
                                let line = self.line(binding);
                                self.assignment_event(targets, value, line, false);
                                return;
                            }
                        }
                    }
                    // `guard let hProcess else {` (Swift's shorthand optional
                    // binding): no `=` and no initialiser, so the arm above
                    // found no value and the line kept nothing. The condition
                    // still reads the binding it shadows, and the one-pass
                    // reader files that name on the guard line. `if let x {` is
                    // silent there, so only a guard writes.
                    if self.kind(id) == "guard_statement" {
                        if let Some(&first) = parts.first() {
                            let targets = self.pattern_names(first);
                            if targets.len() == 1 {
                                let mut name = "[\"n\",".to_string();
                                push_string(&mut name, &targets[0]);
                                name.push(']');
                                let line = self.line(binding);
                                self.event(&format!("[\"x\",{line},{name}]"));
                                return;
                            }
                        }
                    }
                }
            }
        }
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

    /// `guard let x = value else { … }`: the name the condition binds and its
    /// initialiser. A guard with no initialiser binds nothing to a value.
    pub(crate) fn swift_guard_binding(&mut self, id: u32) -> Option<(Vec<String>, u32)> {
        let condition = self.child_of_kind(id, "binding_condition")?;
        let binding = self.child_of_kind(condition, "condition_binding")?;
        let children = self.children(binding);
        let at = children.iter().position(|&child| self.kind(child) == "=")?;
        let value = *children.get(at + 1)?;
        let targets = self.pattern_names(*children.first()?);
        (!targets.is_empty()).then_some((targets, value))
    }

    /// `guard let code = a ?? b else {`: the lexer reads the binding and both
    /// operands as one name list on the guard line; the CST wrote nothing.
    pub(crate) fn swift_guard_coalesce(&mut self, id: u32) -> bool {
        let Some(binding) = self.child_of_kind(id, "binding_condition") else { return false };
        let Some(condition) = self.child_of_kind(binding, "condition_binding") else { return false };
        let parts = self.children(condition);
        let Some(at) = parts.iter().position(|&part| self.kind(part) == "=") else { return false };
        let Some(&value) = parts.get(at + 1) else { return false };
        if self.kind(value) != "nil_coalescing" {
            return false;
        }
        let targets = self.pattern_names(parts[0]);
        if targets.len() != 1 {
            return false;
        }
        let bound = self.form(value);
        let Some(inner) = bound.strip_prefix("[\"m\",[").and_then(|rest| rest.strip_suffix("]]")) else { return false };
        let mut name = "[\"n\",".to_string();
        push_string(&mut name, &targets[0]);
        name.push(']');
        let line = self.line(binding);
        self.event(&format!("[\"x\",{line},[\"m\",[{name},{inner}]]]"));
        true
    }

    /// `if let x = value, cond(x) {`: the binding and the call that follows it
    /// in the same condition list. `condition_before` reads only the first
    /// condition, and a binding forms as nothing, so the call was lost.
    pub(crate) fn swift_binding_then_call(&mut self, id: u32) -> bool {
        let items: Vec<u32> = self
            .children(id)
            .into_iter()
            .take_while(|&child| !matches!(self.kind(child), "block" | "else_clause"))
            .filter(|&child| !self.is_layout(child) && self.kind(child) != ",")
            .collect();
        let [binding, call] = items[..] else { return false };
        if self.kind(binding) != "binding_condition" || self.kind(call) != "call_expression" {
            return false;
        }
        let Some(condition) = self.child_of_kind(binding, "condition_binding") else { return false };
        let parts = self.children(condition);
        let Some(at) = parts.iter().position(|&part| self.kind(part) == "=") else { return false };
        let Some(&value) = parts.get(at + 1) else { return false };
        let targets = self.pattern_names(parts[0]);
        if targets.len() != 1 {
            return false;
        }
        let bound = self.form(value);
        let mut name = "[\"n\",".to_string();
        push_string(&mut name, &targets[0]);
        name.push(']');
        let test = self.form(call);
        let line = self.line(binding);
        self.event(&format!("[\"x\",{line},[\"t\",[[\"m\",[{name},{bound}]],{test}]]]"));
        true
    }

    /// `if cond, var x = value {`: the plain condition and the binding that
    /// follows it. `condition_before` reads only the first condition, so the
    /// call in the binding's initialiser was lost.
    pub(crate) fn swift_call_then_binding(&mut self, id: u32) -> bool {
        let items: Vec<u32> = self
            .children(id)
            .into_iter()
            .take_while(|&child| !matches!(self.kind(child), "block" | "else_clause"))
            .filter(|&child| !self.is_layout(child) && self.kind(child) != ",")
            .collect();
        let [test, binding] = items[..] else { return false };
        if self.kind(binding) != "binding_condition" || self.kind(test) == "binding_condition" {
            return false;
        }
        let Some(condition) = self.child_of_kind(binding, "condition_binding") else { return false };
        let parts = self.children(condition);
        let Some(at) = parts.iter().position(|&part| self.kind(part) == "=") else { return false };
        let Some(&value) = parts.get(at + 1) else { return false };
        let targets = self.pattern_names(parts[0]);
        if targets.len() != 1 {
            return false;
        }
        let bound = self.form(value);
        let mut name = "[\"n\",".to_string();
        push_string(&mut name, &targets[0]);
        name.push(']');
        let first = self.form(test);
        let line = self.line(test);
        self.event(&format!("[\"x\",{line},[\"t\",[{first},[\"m\",[{name},{bound}]]]]]"));
        true
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
