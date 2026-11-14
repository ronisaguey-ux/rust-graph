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
            "call_expression" | "defer_statement" | "go_statement" | "inc_statement" | "send_statement" => {
                self.decl_statement(id)
            }
            _ => return false,
        }
        true
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
        let target = parts.first().and_then(|&name| self.decl_target(name)).into_iter().collect::<Vec<_>>();
        let value = self.decl_values(&parts[equal + 1..]);
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
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &String::from_utf8_lossy(text));
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
        if matches!(kind, "call_expression" | "method_invocation" | "object_creation_expression") {
            let index = self.decl_call(id);
            let args = self
                .children(id)
                .into_iter()
                .find(|&child| matches!(self.kind(child), "arguments" | "argument_list"))
                .map(|list| self.arguments(list))
                .unwrap_or_else(|| "[]".to_string());
            let callee = self.decl_callee(id);
            // `ts.Listener.Addr().String()`: the receiver is a call of its own,
            // and the lexer records it as one. Reading only the outer call lost
            // every chained call in Go (57 of the 96 remaining fallbacks on
            // net/http were exactly this shape).
            self.record_receiver_calls(callee);
            let path = self.decl_path(callee);
            let receiver = match path.rsplit_once('.') {
                Some((head, _)) if !head.is_empty() => {
                    let mut out = "[\"n\",".to_string();
                    push_string(&mut out, head);
                    out.push(']');
                    out
                }
                _ => "0".to_string(),
            };
            return format!("[\"c\",{index},{receiver},{args}]");
        }
        let mut parts = Vec::new();
        if matches!(
            kind,
            "selector_expression" | "qualified_type" | "field_access" | "dotted_name" | "scoped_identifier"
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

    /// One call, at the line its callee is written on, named the way the source
    /// spells it. Returns its index in the symbol's call table, which is what
    /// the flow form's `["c",id,...]` names.
    pub(crate) fn decl_call(&mut self, id: u32) -> usize {
        let callee = self.decl_callee(id);
        let path = self.decl_path(callee);
        let line = self.line(callee);
        self.push_call(path, line)
    }


    /// Record every call that sits in a callee's receiver chain.
    pub(crate) fn record_receiver_calls(&mut self, callee: u32) {
        let mut node = callee;
        for _ in 0..8 {
            if !matches!(self.kind(node), "selector_expression" | "member_expression" | "field_access") {
                return;
            }
            let Some(object) = self.children(node).into_iter().find(|&child| !self.is_layout(child)) else {
                return;
            };
            if matches!(self.kind(object), "call_expression" | "method_invocation") {
                self.decl_call(object);
                node = object;
                continue;
            }
            node = object;
        }
    }

    /// The callee of a call node: the expression before the argument list.
    pub(crate) fn decl_callee(&self, id: u32) -> u32 {
        let children = self.children(id);
        let kind = self.kind(id);
        if kind == "object_creation_expression" {
            return children
                .iter()
                .copied()
                .find(|&child| !matches!(self.kind(child), "new" | "arguments" | "type_arguments"))
                .unwrap_or(id);
        }
        let args = children
            .iter()
            .position(|&child| matches!(self.kind(child), "arguments" | "argument_list" | "type_arguments"));
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
            | "scoped_type_identifier" | "nested_type_identifier" | "class_literal" | "method_reference" => {
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
            "method_invocation" => {
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
            "index_expression" => self.children(id).first().map(|&base| self.decl_path(base)).unwrap_or_default(),
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
            "index_expression" => {
                let path = self.children(id).first().map(|&base| self.decl_path(base)).unwrap_or_default();
                (!path.is_empty()).then_some(path)
            }
            "parenthesized_expression" => self.expression_child(id).and_then(|child| self.decl_target(child)),
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
                "pair" | "pair_pattern" | "object_assignment_pattern" => {
                    if let Some(last) = self.children(node).into_iter().last() {
                        walk.push(last);
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
                "is" | "as" | "satisfies" | "get" | "set" | "static" | "async" | "readonly" | "declare" => {
                    out.push(self.text_string(node))
                }
                _ => {
                    let mut child = self.node(node).first_child;
                    while child != NONE {
                        walk.push(child);
                        child = self.node(child).next_sibling;
                    }
                }
            }
        }
        out
    }

    // -- statements ---------------------------------------------------------

    pub(crate) fn expression_statement(&mut self, id: u32) {
        let Some(child) = self.expression_child(id) else { return };
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
            return;
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
                        if value != "0" {
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
        let value = match self.expression_child(id) {
            Some(child) => {
                if let Some(module) = self.require_module(child) {
                    self.facts.imports.push(Import { line, module, kind: IMPORT_STAR, ..Import::default() });
                }
                self.form(child)
            }
            None => "0".to_string(),
        };
        self.event(&format!("[\"r\",{line},{value}]"));
    }

    /// `const x = ...;` without an initializer is an expression, as the lexer
    /// has it; with one it is an assignment.
    /// `require('x')` is an import, not a call: the lexer path agrees, and a
    /// resolver treats the module string, not a callee named `require`.
    pub(crate) fn require_module(&self, root: u32) -> Option<String> {
        let mut stack = vec![(root, false)];
        while let Some((node, in_args)) = stack.pop() {
            let kind = self.kind(node);
            // A `require` nested in an argument list still counts: the one-pass
            // lexer records `_interopRequireWildcard(require("react"))` as an
            // import of `react`, so refusing the nested form cost 276 JS files.
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
            let next_in_args = in_args || kind == "arguments";
            for child in self.children(node).into_iter().rev() {
                let ck = self.kind(child);
                if !next_in_args
                    && (matches!(ck, "statement_block" | "block" | "class_body")
                        || self.kinds.definitions.iter().any(|def| def.kind == ck))
                {
                    continue;
                }
                stack.push((child, next_in_args));
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
            !self.is_layout(child) && !matches!(self.kind(child), "decorator" | "decorator_call_expression")
        }) else {
            return
        };
        let Some(equal) = children.iter().position(|&child| self.kind(child) == "=") else { return };
        let Some(&value) = children.get(equal + 1) else { return };
        let targets = self.pattern_names(name_node);
        self.assignment_parts(targets, value, self.line(id), false);
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
            "pattern_list" | "tuple_pattern" | "array_pattern" | "list_pattern" | "tuple" | "list"
            | "expression_list" => {
                self.children(id).into_iter().filter_map(|child| self.value_path(child)).collect()
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
            for target in targets {
                self.facts.vars.push((target, class.clone()));
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
            "attribute" | "member_expression" => {
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
                let line = self.line(child);
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
            let targets = inner.get(at + 1).map(|&child| self.pattern_names(child)).unwrap_or_default();
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
            let value = *children[..at].iter().rev().find(|&&child| !self.is_layout(child))?;
            let targets: Vec<String> = children.get(at + 1).map(|&child| self.pattern_names(child)).unwrap_or_default();
            return Some((targets, value));
        }
        let targets = children[..at]
            .iter()
            .filter(|&&child| !matches!(self.kind(child), "for" | "(" | ")"))
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
        if self.kinds.calls.contains(&kind) {
            return self.call_form(id);
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
        if let Some(name) = self.soft_name(id) {
            let mut out = "[\"n\",".to_string();
            push_string(&mut out, &name);
            out.push(']');
            return out;
        }
        match kind {
            "identifier" | "this" | "super" | "self" | "shorthand_property_identifier" | "_" => {
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
            // `field_expression` is Rust's spelling of the same path; without
            // this arm `self.skipped_groups == 0` read both sides as the
            // constant and the name left the record.
            "attribute" | "member_expression" | "subscript" | "subscript_expression" | "field_expression"
            // `Ordering::Acquire` in an argument list is a path, not a constant.
            | "scoped_identifier" | "scoped_type_identifier" => {
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
            // `a if c else b` holds three values and every one is read; the
            // condition is where `x if p(v) else y` calls `p`. `(n := f())` is
            // the same: the value is on the right, the name on the left.
            | "conditional_expression" | "named_expression"
            // `xs[lo():hi():st()]`: a slice and a subscript list are lists of
            // values, not wrappers, so nothing may be dropped.
            | "slice" | "subscript_list" => {
                // `type + 'x'`: `type` is a keyword in this grammar, so the
                // layout filter drops it and the binary expression keeps only
                // its right operand — the lexer reads `m:[n:type,0]` and the
                // CST wrote `0`. A word that is not one of this arm's operators
                // is an operand.
                let items: Vec<u32> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| self.binary_operand(child))
                    .collect();
                if items.len() == 1 {
                    return self.form(items[0]);
                }
                self.merge_form(&items)
            }
            "not_operator" | "unary_operator" | "unary_expression" | "await_expression" | "yield_expression"
            | "typeof" | "void" | "non_null_expression" | "as_expression" | "type_assertion" | "delete_expression" => {
                match self.expression_child(id) {
                    Some(child) => {
                        let inner = self.form(child);
                        format!("[\"m\",[{inner}]]")
                    }
                    None => "0".to_string(),
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
            // `lambda x: f(x)`: the parameters and body form the lambda value.
            "lambda" | "lambda_parameter" | "lambda_star_parameter" | "lambda_parameters" => {
                let mut parts = Vec::new();
                for child in self.children(id) {
                    if self.is_layout(child) || matches!(self.kind(child), "lambda" | ":" | "," | "=" | "*" | "**" | "/") {
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
            "tuple" | "list" | "set" | "pattern_list" | "array" | "tuple_pattern" | "list_pattern" | "array_pattern" => {
                let items: Vec<u32> = self.children(id).into_iter().filter(|&child| !self.is_layout(child)).collect();
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
                    let value = inner.map(|child| self.form(child)).unwrap_or_else(|| "0".to_string());
                    return format!("[\"t\",[{value}]]");
                }
                match items.len() {
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
                    0 => "0".to_string(),
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
            "spread_element" | "rest_pattern" | "list_splat" | "list_splat_pattern" | "dictionary_splat" => match self.expression_child(id) {
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
            "f_string" | "template_string" => {
                // Python's replacement field is `interpolation`, JavaScript's
                // is `template_substitution`; everything else is the same
                // shape, and a template with no substitution is still `0`.
                let field = if kind == "f_string" { "interpolation" } else { "template_substitution" };
                let mut parts = Vec::new();
                for child in self.children(id) {
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
                        !self.is_layout(part) && !matches!(self.kind(part), "{" | "}" | ":" | "!") && self.kind(part) != field
                    });
                    if let Some(value) = value {
                        parts.push(self.form(value));
                    }
                }
                match parts.len() {
                    0 => "0".to_string(),
                    1 => parts.remove(0),
                    _ => format!("[\"m\",[{}]]", parts.join(",")),
                }
            }
            // `UnsafeCell<State>`, `Vec<Entry>`: a generic type holds the base
            // and the type arguments, and both are names the lexer reads.
            "generic_type" | "type_arguments" | "type_parameter_list" => {
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
            // JavaScript reads `null` and `undefined` as names too: there is no
            // keyword table on the lexer's side, so `obj.__proto__ = null`
            // records `n:null` while the CST wrote the constant 0.
            "null" | "undefined" if self.is_js() => {
                let mut out = "[\"n\",".to_string();
                push_string(&mut out, &self.text_string(id));
                out.push(']');
                out
            }
            "string" | "string_literal" | "number" | "integer" | "float" | "true" | "false"
            | "none" | "null" | "undefined" | "regex" | "comment" => "0".to_string(),
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
        if callee != id && matches!(self.kind(callee), "call" | "call_expression" | "new_expression") {
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
        if matches!(self.kind(callee), "attribute" | "member_expression") {
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
            if self.path_of(callee).is_empty() {
                receiver_call = Some(value);
            }
        }
        let (path, receiver) = self.call_target(callee, receiver_call);
        let line = self.call_line(callee, id);
        let index = self.push_call(path, line);
        let args = children
            .iter()
            .copied()
            .find(|&child| matches!(self.kind(child), "argument_list" | "arguments"));
        let mut out = format!("[\"c\",{index},{receiver},");
        match args {
            Some(list) => out.push_str(&self.arguments(list)),
            None => out.push_str("[]"),
        }
        out.push(']');
        let _ = is_new;
        let link = (index as u32, receiver);
        (out, Some(link))
    }

    /// The called path and the receiver form: `f(x)` -> (`f`, `0`),
    /// `a.b.c(x)` -> (`a.b.c`, `["n","a.b"]`), `A().b()` -> (`.b`,
    /// `["c",0,0,[]]`) with the index of the `A` call the walk recorded.
    pub(crate) fn call_target(&self, callee: u32, receiver_call: Option<String>) -> (String, String) {
        match self.kind(callee) {
            "attribute" | "member_expression" => {
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
            if text[0].is_ascii_alphanumeric() || text[0] == b'_' || text[0] == b'#' || text[0] == b'$' {
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
            "attribute" | "member_expression" => {
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
            // Go and Java do not use `form`: an argument that is itself a call
            // (`path.Join(os.Getenv(..))`) would come back as `0`.
            let value = if ellipsis {
                "0".to_string()
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
            "attribute" | "member_expression" => {
                let children = self.children(id);
                let name = self.member_name(id);
                let base = children.first().map(|&child| self.path_of(child)).unwrap_or_default();
                if base.is_empty() {
                    format!(".{name}")
                } else {
                    format!("{base}.{name}")
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
            "parenthesized_expression" => self.expression_child(id).map(|child| self.path_of(child)).unwrap_or_default(),
            "as_expression" | "type_assertion" | "non_null_expression" => {
                self.expression_child(id).map(|child| self.path_of(child)).unwrap_or_default()
            }
            // A call has no name of its own: `f().x` names `.x`.
            "call" | "call_expression" | "new_expression" => String::new(),
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
