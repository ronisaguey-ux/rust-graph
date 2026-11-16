//! C, C++ and Objective-C: the declared name lives inside a declarator.

use super::*;

impl<'a> Walker<'a> {
    // -- C and C++ ----------------------------------------------------------
    //
    // C and C++ are read here, not through the Python/JS arms above. The
    // declarations are specifier runs over a declarator (the name is inside
    // `direct_declarator`, not a child the shared `Def` can name), a class is
    // `class_specifier`/`struct_or_union_specifier` because the grammar shares
    // one rule between `class`, `struct` and `union`, and a method's owner is
    // the enclosing class or namespace. Everything below is reached only when
    // [`Self::is_c`] is true, so no other language can see it.

    /// The runtime's language name for a C-family grammar. The grammar, not the
    /// lexer tag, is what decides: `CPP` is C++ here and nothing else.
    pub(crate) fn is_c(&self) -> bool {
        matches!(self.language.name, "c" | "cpp" | "objc")
    }

    /// The C-family walk arm: the nodes whose name is not a child of a `Def`
    /// table (a declarator holds it) and the containers a member's owner comes
    /// from. `false` hands the node to the shared walk below.
    pub(crate) fn enter_c(&mut self, id: u32) -> bool {
        match self.kind(id) {
            // Every shape the C and C++ grammars give a function whose return
            // type is a typedef name or a scoped path: `list *listCreate(void)`
            // is `bare_user_function_definition`, not `function_definition`,
            // and reading only the latter left every redis function unnamed.
            "function_definition"
            | "user_function_definition"
            | "bare_user_function_definition"
            | "plain_user_function"
            | "specified_user_function"
            | "scoped_user_function"
            | "specified_scoped_function" => self.c_function(id),
            // C spells it `struct_or_union_specifier`, C++ `class_specifier`;
            // both carry a body and a name, and the lexer files both as a class.
            "class_specifier" | "struct_or_union_specifier" | "struct_specifier" | "enum_specifier" => {
                self.c_class(id)
            }
            // `__asm__("bsr %1,%0" : …)`: the grammar gives it its own node and
            // nothing read it, so every inline asm the lexer files as a call was
            // lost (161 of the C corpus's losses, the largest single cluster).
            // The lexer names the call after the token as written, `__asm__` or
            // `asm`, and a leading `__volatile__` is a second call to it.
            "asm_statement" | "gnu_asm_expression" => {
                let line = self.line(id);
                let text = self.text_string(id);
                let mut rest = text.trim_start();
                let mut first = true;
                for _ in 0..3 {
                    let Some(word) = leading_word(rest) else { break };
                    if first || word == "volatile" || word == "__volatile__" {
                        self.push_call(word.to_string(), line);
                    }
                    first = false;
                    rest = rest[word.len()..].trim_start();
                    if rest.starts_with('(') {
                        break;
                    }
                }
            }
            // `weak_alias(a, b)` at file scope: the grammar gives the statement
            // its own node, and nothing read it, so every macro invocation the
            // lexer files as a call was lost (185 of the C corpus's losses).
            "macro_call_statement" => {
                if let Some(node) = self.child_of_kind(id, "identifier") {
                    let name = self.text_string(node);
                    let line = self.line(id);
                    let index = self.push_call(name, line);
                    // The lexer files the statement twice: the call, and an
                    // event whose value is that call's form. Writing only the
                    // call left one loss on every alias line in musl.
                    if let Some(list) = self.child_of_kind(id, "arguments") {
                        let args = self.arguments(list);
                        self.event(&format!("[\"x\",{line},[\"c\",{index},0,{args}]]"));
                    }
                }
            }
            // `__attribute__((__may_alias__))`, `__attribute__((visibility("hidden")))`:
            // the lexer reads the attribute as a call of the token and nothing
            // read the node, so every attributed typedef and prototype in musl
            // lost a call.
            "attribute" => {
                let line = self.line(id);
                self.push_call("__attribute__".to_string(), line);
            }
            // `@interface A : NSObject`: the header line is a statement to the
            // lexer and nothing wrote it, so every Objective-C interface lost
            // its first line.
            "objc_interface" => {
                let line = self.line(id);
                let mut parts: Vec<String> = Vec::new();
                let mut form = String::from("[\"n\",");
                push_string(&mut form, "interface");
                form.push(']');
                parts.push(form);
                for child in self.children(id) {
                    for name in self.c_words_of(child) {
                        let mut form = String::from("[\"n\",");
                        push_string(&mut form, &name);
                        form.push(']');
                        parts.push(form);
                    }
                }
                let value = format!("[\"m\",[{}]]", parts.join(","));
                self.lexer_event(b'x', line, &[], &value, false);
            }
            // `ShaderDesignerWindowController *windowController;` in an ivar
            // block is a declaration statement to the lexer.
            "ivar_declaration" => {
                let form = self.c_words_form_skipping(id, &["initializer_list"]);
                if form != "0" {
                    let line = self.line(id);
                    self.event(&format!("[\"x\",{line},{form}]"));
                }
            }
            // `@synthesize window = _window;` files the binding, and nothing
            // read it: every synthesized property lost its line.
            "objc_synthesize" => {
                let names: Vec<String> = self
                    .children(id)
                    .into_iter()
                    .filter(|&child| self.kind(child) == "identifier")
                    .map(|child| self.text_string(child))
                    .collect();
                if names.len() >= 2 {
                    let line = self.line(id);
                    let mut form = |text: &str| {
                        let mut out = String::from("[\"n\",");
                        push_string(&mut out, text);
                        out.push(']');
                        out
                    };
                    // The lexer keeps the first binding's value and the later
                    // *names* as `[name, value]` merges: `@synthesize a = _a,
                    // b = _b;` is `a = t:[n:_a, m:[n:b,n:_b]]`.
                    let pairs: Vec<(String, String)> = names
                        .chunks(2)
                        .map(|pair| (pair[0].clone(), pair.get(1).cloned().unwrap_or_default()))
                        .collect();
                    let value = if pairs.len() == 1 {
                        form(&pairs[0].1)
                    } else {
                        let mut parts = vec![form(&pairs[0].1)];
                        for (key, value) in &pairs[1..] {
                            parts.push(format!("[\"m\",[{},{}]]", form(key), form(value)));
                        }
                        format!("[\"t\",[{}]]", parts.join(","))
                    };
                    self.lexer_event(b'a', line, &[names[0].clone()], &value, false);
                }
            }
            // `@selector(run:)`: the lexer files a call of `selector`, and the
            // node was never read.
            "selector_expression" => {
                let line = self.line(id);
                self.push_call("selector".to_string(), line);
            }
            // `@property (nonatomic, strong) UILabel *label;`: the lexer reads
            // `property` as a call of that name, and the node was never read -
            // 155 one-loss files on the Objective-C corpus.
            "objc_property_declaration" => {
                let line = self.line(id);
                let index = self.push_call("property".to_string(), line);
                // The line itself is an `x` event whose first element is that
                // call, with the attribute words as its arguments, followed by
                // the type and the declared name - exactly the merge the lexer
                // writes for a statement.
                let name_form = |text: &str| {
                    let mut form = String::from("[\"n\",");
                    push_string(&mut form, text);
                    form.push(']');
                    form
                };
                let mut attributes: Vec<String> = Vec::new();
                let mut rest: Vec<String> = Vec::new();
                for child in self.children(id) {
                    if self.kind(child) == "property_attributes" {
                        // `(nonatomic, readonly, getter=isWindow)`: an
                        // assignment attribute is a keyword argument to the
                        // call, not two names in a row.
                        let text = self.text_string(child);
                        let inside = text.trim().trim_start_matches('(').trim_end_matches(')').to_string();
                        for item in inside.split(',') {
                            let item = item.trim();
                            if item.is_empty() {
                                continue;
                            }
                            match item.find('=') {
                                Some(at) => {
                                    let mut form = String::from("[\"k\",");
                                    push_string(&mut form, item[..at].trim());
                                    form.push_str("]:[\"n\",");
                                    push_string(&mut form, item[at + 1..].trim());
                                    form.push(']');
                                    attributes.push(form);
                                }
                                None => {
                                    let mut form = String::from("[\"n\",");
                                    push_string(&mut form, item);
                                    form.push(']');
                                    attributes.push(form);
                                }
                            }
                        }
                    } else {
                        rest.extend(self.c_words_of(child));
                    }
                }
                let args: Vec<String> = attributes.clone();
                let call = format!("[\"c\",{index},0,[{}]]", args.join(","));
                let mut parts = vec![call];
                parts.extend(rest.iter().map(|name| name_form(name)));
                let value = format!("[\"m\",[{}]]", parts.join(","));
                self.lexer_event(b'x', line, &[], &value, false);
            }
            // `[NSIndexSet indexSetWithIndexesInRange:NSMakeRange(200, 100)]`:
            // a call written as a message argument was never recorded - the
            // message node is not a call to this reader, so nothing descended
            // into its arguments. 9 one-loss files on the corpus.
            "message" => {
                let mut calls = Vec::new();
                self.c_calls_in(id, &mut calls);
                for call in calls {
                    self.form(call);
                }
                return false;
            }
            "namespace_definition" => self.c_namespace(id),
            // `static_assert(sizeof(T) == 4, "…")`: the lexer reads a call, the
            // grammar gives it a declaration node, and nothing read it.
            "static_assert_declaration" | "static_assert" => {
                let line = self.line(id);
                self.push_call("static_assert".to_string(), line);
            }
            // `char buf[BUF];` / `int x = 0;`: the lexer reads the declaration
            // as the run of names on its line (plus the initializer's value when
            // there is one), so the CST writes the same event instead of
            // dropping the statement as layout.
            "declaration_statement" | "declaration" => {
                // `typedef struct { … } Foo;` and `typedef struct aeApiState {
                // … } aeApiState;`: the lexer writes the declarator's name on
                // the closing line (`n:Foo`) and no reader wrote it for either
                // form.
                let mut has_body_type = false;
                let mut pending = vec![id];
                while let Some(node) = pending.pop() {
                    match self.kind(node) {
                        "struct_or_union_specifier" | "struct_specifier" | "enum_specifier" => {
                            let body = self.child_of_kind(node, "struct_body").is_some()
                                || self.child_of_kind(node, "field_declaration_list").is_some()
                                || self.child_of_kind(node, "enumerator_list").is_some();
                            if body {
                                has_body_type = true;
                            }
                        }
                        _ => pending.extend(self.children(node)),
                    }
                }
                if has_body_type {
                if let Some(declarator) = self
                    .children(id)
                    .into_iter()
                    .find(|&node| self.kind(node) == "direct_declarator")
                {
                    if let Some(name_node) = self.child_of_kind(declarator, "identifier") {
                        let name = self.text_string(name_node);
                        if !name.is_empty() {
                            let line = self.line(name_node);
                            let mut value = String::from("[\"n\",");
                            push_string(&mut value, &name);
                            value.push(']');
                            self.lexer_event(b'x', line, &[], &value, false);
                        }
                    }
                }
                }
                let form = self.c_words_form_skipping(
                    id,
                    &["class_body", "struct_body", "compound_statement", "function_definition"],
                );
                // `static const double huge = 0x1p1023;`: a declaration with
                // an initializer is an assignment to the lexer, and the words
                // form wrote an `x` it never wrote, so every initialised
                // constant in musl lost its line.
                let mut inits = Vec::new();
                self.c_initializers(id, &mut inits);
                // `lua_pushnil(L);` - one argument, and C's most vexing parse
                // makes the grammar call it a declaration: a plain type name
                // with a parenthesised declarator. The lexer reads a call, so
                // every one-argument call statement in the corpus lost it.
                if !self.c_has_kind(id, "compound_statement")
                    && self.c_has_kind(id, "parenthesized_declarator")
                    && !self.c_has_kind(id, "parameter_list")
                    && !self.c_has_kind(id, "initializer_list")
                    && inits.is_empty()
                {
                    let text = self.text_string(id);
                    if let Some(word) = leading_word(text.trim_start()) {
                        let line = self.line(id);
                        self.push_call(word.to_string(), line);
                    }
                    return true;
                }
                // `int (*close)(FILE *);` - a function pointer. The lexer reads
                // the type word and the parenthesised list as a call of that
                // type, with the pointer's own name as its argument, and keeps
                // only a storage class in front of it as a name (`const` is not
                // one to it). Written as the words form the value never had a
                // call in it, so every function pointer in the corpus lost both
                // its call and its event.
                if !self.c_has_kind(id, "compound_statement")
                    && self.c_has_kind(id, "parenthesized_declarator")
                    && self.c_has_kind(id, "parameter_list")
                    // `typedef int (*cmpfun)(…);` is a type name to the lexer,
                    // not a call: the words form is what pairs there.
                    && leading_word(self.text_string(id).trim_start()) != Some("typedef")
                    // With an initialiser the lexer reads the value, not a call.
                    && inits.is_empty()
                {
                    let line = self.line(id);
                    let text = self.text_string(id);
                    let mut rest = text.trim_start();
                    let mut names: Vec<String> = Vec::new();
                    loop {
                        let Some(word) = leading_word(rest) else { break };
                        match word {
                            "static" | "extern" | "register" | "inline" => {
                                names.push(word.to_string());
                                rest = rest[word.len()..].trim_start();
                            }
                            "const" | "volatile" | "typedef" => {
                                rest = rest[word.len()..].trim_start();
                            }
                            _ => break,
                        }
                    }
                    if let Some(word) = leading_word(rest) {
                        let index = self.push_call(word.to_string(), line);
                        let name = self.c_descendant(id, "parenthesized_declarator")
                            .and_then(|node| self.declarator_of(node))
                            .unwrap_or_default();
                        let mut args = String::new();
                        if !name.is_empty() {
                            args = "[[\"n\",".to_string();
                            push_string(&mut args, &name);
                            args.push_str("]]");
                        }
                        let call = format!("[\"c\",{index},0,{args}]");
                        let value = if names.is_empty() {
                            call
                        } else {
                            let mut parts: Vec<String> = names
                                .iter()
                                .map(|name| {
                                    let mut form = "[\"n\",".to_string();
                                    push_string(&mut form, name);
                                    form.push(']');
                                    form
                                })
                                .collect();
                            parts.push(call);
                            format!("[\"m\",[{}]]", parts.join(","))
                        };
                        self.lexer_event(b'x', line, &[], &value, false);
                    }
                    return true;
                }
                let mut wrote_prototype = false;
                if !inits.is_empty() {
                    // `char *a=0, *b=0, *c=0;`: the lexer unpacks the declarator
                    // list and keeps the later names as elements of the first
                    // binding's value - `t:[0,n:b,n:c]` - so writing one
                    // assignment per declarator left the first one unequal on
                    // every multi-declarator statement in the corpus.
                    let first = inits[0].clone();
                    if inits.len() == 1 && self.kind(first.1) == "initializer_list" {
                        // `static const uint32_t K[64] = { 0, 0, … };`: the
                        // lexer reads the brace list as a merge of its
                        // elements, each a constant, and the value written here
                        // was the constant alone - every constant table in the
                        // corpus disagreed (97 rows).
                        let (name, node, line) = first;
                        let elements = self
                            .children(node)
                            .into_iter()
                            .filter(|&child| self.kind(child) != ",")
                            .count();
                        let value = format!("[\"m\",[{}]]", vec!["0"; elements].join(","));
                        self.lexer_event(b'a', line, &[name], &value, false);
                    } else if inits.len() == 1 {
                        let (name, node, line) = first;
                        self.assignment_parts(vec![name], node, line, false);
                    } else {
                        let (name, node, line) = first;
                        let mut parts = vec![self.form(node)];
                        for (other, _, _) in &inits[1..] {
                            let mut form = "[\"n\",".to_string();
                            push_string(&mut form, other);
                            form.push(']');
                            parts.push(form);
                        }
                        let value = format!("[\"t\",[{}]]", parts.join(","));
                        self.lexer_event(b'a', line, &[name], &value, false);
                    }
                } else if form != "0" && !wrote_prototype {
                    let line = self.line(id);
                    self.event(&format!("[\"x\",{line},{form}]"));
                }
                // `size_t n = strlen(s);` — the words form carries the names,
                // and the initializer's calls are facts of their own. Reading
                // only the words lost every call written in a declaration,
                // which in C is most of them.
                let mut calls = Vec::new();
                self.c_calls_in(id, &mut calls);
                for call in calls {
                    self.form(call);
                }
                // `struct althost { … };` sits inside the declaration statement,
                // which this arm claims, so the walk never descends to it and
                // the struct had no symbol at all.
                let mut bodies = Vec::new();
                self.c_specifiers_in(id, &mut bodies);
                for body in bodies {
                    self.c_class(body);
                }
                // `int g(void);` — the lexer reads a declared name followed by
                // its parameter list as a call, and a header is mostly
                // prototypes, so every one of them was a loss.
                if !self.c_has_kind(id, "compound_statement")
                    && self.c_has_kind(id, "parameter_list")
                {
                    if let Some(name) = self.declarator_name(id) {
                        let line = self.line(id);
                        let index = self.push_call(name, line);
                        // The lexer reads a prototype as one statement: the
                        // return type's words, then the call with each
                        // parameter as a merge of its own words - `const` and
                        // the pointer are dropped, a lone `void` is `0`.
                        // Writing only the declared name left every prototype
                        // in every C and C++ header unequal.
                        let name_form = |text: &str| {
                            let mut form = String::from("[\"n\",");
                            push_string(&mut form, text);
                            form.push(']');
                            form
                        };
                        let mut parts: Vec<String> = Vec::new();
                        for child in self.children(id) {
                            if self.kind(child) == "direct_declarator" {
                                break;
                            }
                            for word in self.c_words_of(child) {
                                if matches!(word.as_str(), "const" | "static" | "extern" | "inline") {
                                    continue;
                                }
                                parts.push(name_form(&word));
                            }
                        }
                        if let Some(list) = self.c_descendant(id, "parameter_list") {
                            let mut args: Vec<String> = Vec::new();
                            for parameter in self.children(list) {
                                let words: Vec<String> = self
                                    .c_words_of(parameter)
                                    .into_iter()
                                    .filter(|word| !matches!(word.as_str(), "const" | "static" | "extern" | "inline"))
                                    .collect();
                                if words.is_empty() || (words.len() == 1 && words[0] == "void") {
                                    args.push("0".to_string());
                                    continue;
                                }
                                let forms: Vec<String> = words.iter().map(|word| name_form(word)).collect();
                                args.push(format!("[\"m\",[{}]]", forms.join(",")));
                            }
                            parts.push(format!("[\"c\",{index},0,[{}]]", args.join(",")));
                            let value = format!("[\"m\",[{}]]", parts.join(","));
                            self.lexer_event(b'x', line, &[], &value, false);
                            wrote_prototype = true;
                        }
                    }
                }
                // `CURLcode Curl_altsvc_load(struct altsvcinfo *asi);`: a return
                // type the grammar does not know becomes a `bare_user_declaration`
                // wrapping the declarator and its parameters, so the prototype
                // arm above saw no parameter list of its own and every such
                // prototype in a C++ header lost the call the lexer reads.
                if !self.c_has_kind(id, "compound_statement") {
                    for child in self.children(id) {
                        if self.kind(child) != "bare_user_declaration"
                            || !self.c_has_kind(child, "parameter_list")
                        {
                            continue;
                        }
                        // The declarator here is a bare identifier before the
                        // parameter list - there is no `direct_declarator` node
                        // for `declarator_name` to follow.
                        let name = self
                            .children(child)
                            .into_iter()
                            .take_while(|&node| self.kind(node) != "parameter_list")
                            .find(|&node| self.kind(node) == "identifier")
                            .map(|node| self.text_string(node));
                        if let Some(name) = name {
                            let line = self.line(id);
                            self.push_call(name, line);
                        }
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// `(name, value node, line)` for each declarator of a declaration that has
    /// an initializer, in source order.
    fn c_initializers(&mut self, id: u32, out: &mut Vec<(String, u32, u32)>) {
        let mut pending: Option<String> = None;
        for child in self.children(id) {
            match self.kind(child) {
                "declaration_specifiers" => continue,
                // `static const uint32_t k = 235;` with a type the grammar does
                // not know becomes a `specified_user_declaration` inside the
                // statement, so the declarator and its initialiser were never
                // seen and the line lost its assignment.
                "specified_user_declaration" => {
                    self.c_initializers(child, out);
                    continue;
                }
                "direct_declarator" => {
                    pending = self.declarator_of(child);
                    continue;
                }
                _ => {}
            }
            if self.is_layout(child) {
                continue;
            }
            if let Some(name) = pending.take() {
                // The lexer files the assignment on the line the initialiser is
                // written on: `static const uint32_t\n  k2 = 235;` is line 2
                // there, and the declaration's own first line wrote an event
                // nothing could pair (38 rows of the C corpus).
                let line = self.line(child);
                out.push((name, child, line));
            }
        }
    }

    /// The first descendant of `id` of `kind`, in source order.
    fn c_descendant(&self, id: u32, kind: &str) -> Option<u32> {
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            for child in self.children(node).into_iter().rev() {
                if self.kind(child) == kind {
                    return Some(child);
                }
                stack.push(child);
            }
        }
        None
    }

    /// Whether any descendant of `id` is of `kind`.
    fn c_has_kind(&self, id: u32, kind: &str) -> bool {
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            for child in self.children(node) {
                if self.kind(child) == kind {
                    return true;
                }
                stack.push(child);
            }
        }
        false
    }

    /// The name inside a `direct_declarator` node itself.
    fn declarator_of(&self, id: u32) -> Option<String> {
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            for child in self.children(node).into_iter().rev() {
                match self.kind(child) {
                    "parameter_list" | "compound_statement" | "initializer_list" => {}
                    "identifier" => return Some(self.text_string(child)),
                    _ => stack.push(child),
                }
            }
        }
        None
    }

    /// The declared name of a declaration: the first identifier under a
    /// `direct_declarator`, never one inside its parameter list.
    fn declarator_name(&self, id: u32) -> Option<String> {
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            for child in self.children(node).into_iter().rev() {
                match self.kind(child) {
                    "parameter_list" | "compound_statement" | "initializer_list" => {}
                    // Only the declarator's own subtree names it: the type
                    // before it is a `plain_type_name` with an identifier of
                    // its own, and returning the first identifier seen after
                    // the declarator walked the type too named the call
                    // `CURLcode` where the lexer names it `Curl_amiga_init`.
                    "direct_declarator" => {
                        let mut own = vec![child];
                        while let Some(node) = own.pop() {
                            for inner in self.children(node) {
                                match self.kind(inner) {
                                    "parameter_list" | "compound_statement" | "initializer_list" => {}
                                    "identifier" => return Some(self.text_string(inner)),
                                    _ => own.push(inner),
                                }
                            }
                        }
                    }
                    _ => stack.push(child),
                }
            }
        }
        None
    }

    /// Every `struct`/`union`/`enum` specifier inside `id`.
    fn c_specifiers_in(&self, id: u32, out: &mut Vec<u32>) {
        for child in self.children(id) {
            match self.kind(child) {
                "struct_or_union_specifier" | "struct_specifier" | "enum_specifier" | "class_specifier" => {
                    out.push(child)
                }
                "compound_statement" | "function_definition" => {}
                _ => self.c_specifiers_in(child, out),
            }
        }
    }

    /// Every `call_expression` inside `id`, in source order, not descending
    /// into a nested function or class body (those are walked on their own).
    fn c_calls_in(&self, id: u32, out: &mut Vec<u32>) {
        for child in self.children(id) {
            match self.kind(child) {
                "call_expression" => out.push(child),
                "lambda_expression" | "class_specifier" | "function_definition" | "compound_statement" => {}
                _ => self.c_calls_in(child, out),
            }
        }
    }

    /// `#include "settings.h"` and `#include <stdio.h>`: the grammar drops the
    /// directive line as trivia — which is what keeps include guards and
    /// conditional blocks from costing a statement — so the module is read off
    /// the source, on the line it sits on, exactly as the lexer reads it.
    pub(crate) fn c_includes(&mut self) {
        let text = String::from_utf8_lossy(self.src);
        for (index, line) in text.lines().enumerate() {
            // Objective-C imports with `#import`, which is the same directive
            // to the lexer under another word: nothing read it, so every
            // `#import "Local.h"` in the corpus lost its import (464 rows on
            // the Objective-C corpus alone).
            let trimmed = line.trim_start();
            let Some(rest) = trimmed
                .strip_prefix("#include")
                .or_else(|| trimmed.strip_prefix("#import"))
            else {
                continue;
            };
            let rest = rest.trim_start();
            // A quoted include names a file of this project and is the import
            // the lexer files (a star import: `<stdio.h>` is the system's).
            let Some(inner) = rest.strip_prefix('"') else { continue };
            let Some(module) = inner.split('"').next().filter(|module| !module.is_empty()) else { continue };
            self.facts.imports.push(Import { line: index as u32 + 1, module: module.to_string(), kind: IMPORT_STAR, ..Import::default() });
        }
    }

    /// Nodes C and C++ own. `false` hands the node to the shared walk.
    pub(crate) fn c_function(&mut self, id: u32) {
        let children = self.children(id);
        let declarator = children.iter().copied().find(|&child| self.kind(child) == "direct_declarator");
        let name = declarator
            .and_then(|node| self.first_identifier(node))
            .or_else(|| self.child_of_kind(id, "identifier").map(|node| self.text_string(node)))
            .or_else(|| {
                self.child_of_kind(id, "scoped_type_name")
                    .and_then(|node| self.c_path(node))
                    .and_then(|path| path.rsplit('.').next().map(str::to_string))
            });
        let Some(name) = name else { return };
        let line = self.line(id);
        let params = declarator
            .and_then(|node| self.child_of_kind(node, "parameter_list"))
            .or_else(|| children.iter().copied().find(|&child| self.kind(child) == "parameter_list"))
            .map(|list| self.c_params(list))
            .unwrap_or_default();
        let owner = if self.frame().container { self.frame().class.clone() } else { String::new() };
        let index = self.add_symbol(name.clone(), owner.clone(), KIND_FUNCTION, line, params);
        self.stack.push(Frame { node: id, symbol: index, class: owner, receiver: String::new(), container: false, defines: Some(index) });
    }

    /// A parameter list as the lexer reads it: the last name of each
    /// comma-separated group, with `void` standing for no parameter.
    pub(crate) fn c_params(&self, list: u32) -> Vec<String> {
        let mut out = Vec::new();
        let mut group: Vec<String> = Vec::new();
        for child in self.children(list) {
            if self.kind(child) == "," {
                if let Some(name) = group.pop() {
                    if name != "void" {
                        out.push(name);
                    }
                }
                group.clear();
                continue;
            }
            group.extend(self.c_words_of(child));
        }
        if let Some(name) = group.pop() {
            if name != "void" {
                out.push(name);
            }
        }
        out
    }

    /// `class A : public B {`, `struct s {`, `union u {`: a container whose
    /// members take its name as their owner. A declaration with no body is a
    /// use of the type, not a definition, and the lexer opens no symbol for it.
    pub(crate) fn c_class(&mut self, id: u32) {
        // `struct althost { char *host; };` in C has a field list, not a
        // `struct_body`; without it every C struct and enum was unnamed, and
        // the lexer's symbol for it had no counterpart.
        if self.child_of_kind(id, "class_body").is_none()
            && self.child_of_kind(id, "struct_body").is_none()
            && self.child_of_kind(id, "field_declaration_list").is_none()
            && self.child_of_kind(id, "enumerator_list").is_none()
        {
            return;
        }
        let Some(name_node) = self.child_of_kind(id, "identifier") else {
            // `typedef struct { uint32_t size; } Foo;`: the specifier carries no
            // name - the declarator after it does - so the body was never read
            // and every member line lost its event. The members are still
            // statements to the lexer, whatever the type is called.
            if let Some(body) = self
                .child_of_kind(id, "field_declaration_list")
                .or_else(|| self.child_of_kind(id, "struct_body"))
            {
                self.c_field_events(body);
            }
            return;
        };
        let name = self.text_string(name_node);
        if name.is_empty() {
            return;
        }
        let line = self.line(id);
        let mut bases = Vec::new();
        if let Some(clause) = self.child_of_kind(id, "base_clause") {
            for child in self.children(clause) {
                if matches!(self.kind(child), "plain_type_name" | "scoped_type_name" | "user_type_name") {
                    let path = self.c_path(child).unwrap_or_default();
                    if let Some(last) = path.rsplit('.').next().filter(|last| !last.is_empty()) {
                        bases.push(last.to_string());
                    }
                }
            }
        }
        let index = self.add_symbol(name.clone(), String::new(), KIND_CLASS, line, Vec::new());
        self.facts.symbols[index].bases = bases;
        let symbol = self.frame().symbol;
        self.stack.push(Frame { node: id, symbol, class: name, receiver: String::new(), container: true, defines: Some(index) });
        // A member's line is a statement to the lexer: `char *host;` is an
        // event with the names on that line. The body's declarations are flat
        // siblings, so they are read by line rather than by node.
        if let Some(body) = self
            .child_of_kind(id, "field_declaration_list")
            .or_else(|| self.child_of_kind(id, "struct_body"))
        {
            self.c_field_events(body);
        }
    }

    /// One `x` event per member line inside an aggregate body.
    fn c_field_events(&mut self, body: u32) {
        // `BIT(persist);`: the grammar reads a function-like macro member as a
        // type name followed by a declarator, and the lexer reads the same
        // tokens as a call to that name. curl's bit fields are 48 C++ files.
        for child in self.children(body) {
            if self.kind(child) != "user_struct_declaration" || !self.c_has_kind(child, "parenthesized_declarator") {
                continue;
            }
            let Some(first) = self
                .children(child)
                .into_iter()
                .find(|&node| self.kind(node) == "user_type_name")
            else {
                continue;
            };
            let text = self.text_string(first);
            let Some(name) = leading_word(text.trim_start()).map(str::to_string) else { continue };
            let line = self.line(child);
            self.push_call(name, line);
        }
        let mut by_line: std::collections::BTreeMap<u32, Vec<String>> = std::collections::BTreeMap::new();
        let mut stack = vec![body];
        while let Some(node) = stack.pop() {
            // `BIT(persist);` in a struct body: a member the lexer reads as a
            // call, because a macro member is a call to anyone reading tokens.
            // 48 files of the C++ corpus are curl's bit fields alone.
            if self.kind(node) == "call_expression" {
                if let Some(callee) = self.children(node).first().copied() {
                    let path = self.path_of(callee);
                    if !path.is_empty() {
                        let line = self.line(node);
                        self.push_call(path, line);
                    }
                }
            }
            let item = self.node(node);
            if item.first_child == NONE {
                if let Some(name) = self.leaf_name(node) {
                    let line = self.line(node);
                    by_line.entry(line).or_default().push(name);
                }
                continue;
            }
            for child in self.children(node).into_iter().rev() {
                stack.push(child);
            }
        }
        for (line, names) in by_line {
            let mut value = "[\"m\",[".to_string();
            for (index, name) in names.iter().enumerate() {
                if index > 0 {
                    value.push(',');
                }
                value.push_str("[\"n\",");
                push_string(&mut value, name);
                value.push(']');
            }
            value.push_str("]]");
            self.lexer_event(b'x', line, &[], &value, false);
        }
    }

    /// `namespace app { ... }`: the lexer files a namespace as a class, and its
    /// members take the namespace as their owner.
    pub(crate) fn c_namespace(&mut self, id: u32) {
        let Some(name_node) = self.child_of_kind(id, "identifier") else { return };
        let name = self.text_string(name_node);
        let line = self.line(id);
        let index = self.add_symbol(name.clone(), String::new(), KIND_CLASS, line, Vec::new());
        let symbol = self.frame().symbol;
        self.stack.push(Frame { node: id, symbol, class: name, receiver: String::new(), container: true, defines: Some(index) });
    }

    /// `int x = 0;`, `Square box = Square(4);`, `std::string name_;`: the
    /// lexer reads the statement, so a declarator with an initializer is an
    /// assignment event and a declarator without one is the words of the
    /// declaration. `typedef struct {...} T;` is the same shape.
    pub(crate) fn c_words_form(&self, id: u32) -> String {
        self.c_words_form_skipping(id, &[])
    }

    /// The same, without descending into the named subtrees: a struct's body is
    /// its members' statements, not part of the declaration that holds it.
    pub(crate) fn c_words_form_skipping(&self, id: u32, skip: &[&str]) -> String {
        let words = self.c_words_of_skipping(id, skip);
        if words.is_empty() {
            return "0".to_string();
        }
        let mut out = "[\"m\",[".to_string();
        for (index, word) in words.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("[\"n\",");
            push_string(&mut out, word);
            out.push(']');
        }
        out.push_str("]]");
        out
    }

    pub(crate) fn c_words_of(&self, id: u32) -> Vec<String> {
        self.c_words_of_skipping(id, &[])
    }

    pub(crate) fn c_words_of_skipping(&self, id: u32, skip: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            if skip.contains(&self.kind(node)) {
                continue;
            }
            let item = self.node(node);
            if item.first_child == NONE {
                if let Some(name) = self.leaf_name(node) {
                    out.push(name);
                }
                continue;
            }
            let kids = self.children(node);
            for child in kids.into_iter().rev() {
                stack.push(child);
            }
        }
        out
    }

    /// The merge of every value-bearing child, in order.
    pub(crate) fn c_path(&self, id: u32) -> Option<String> {
        match self.kind(id) {
            "field_expression" | "qualified_expression" => {
                // `s->fn(…)`: the grammar gives the arrow form the same node as
                // a dot, but the member's own leaf node is empty, so the path
                // came out as `s.` and the call had no name at all. The source
                // between the operator is the name.
                let mut name = self.member_name(id);
                if name.is_empty() {
                    let text = self.text_string(id);
                    let tail: String = text
                        .chars()
                        .rev()
                        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                        .collect();
                    name = tail.chars().rev().collect();
                }
                let object = self.children(id).into_iter().next()?;
                // `cf->next->cft->do_connect(…)`: the receiver is itself a
                // field expression, and `path_of` reads a name, not a chain, so
                // the call came out as `.do_connect` and matched nothing.
                let base = self
                    .c_path(object)
                    .or_else(|| {
                        let path = self.path_of(object);
                        (!path.is_empty()).then_some(path)
                    })
                    .unwrap_or_default();
                Some(if base.is_empty() { format!(".{name}") } else { format!("{base}.{name}") })
            }
            "plain_type_name" | "user_type_name" => {
                let child = self.children(id).into_iter().next()?;
                let path = self.path_of(child);
                (!path.is_empty()).then_some(path)
            }
            "scoped_type_name" => {
                let mut out = String::new();
                for child in self.children(id) {
                    if self.kind(child) == "template_arguments" {
                        continue;
                    }
                    let part = if matches!(self.kind(child), "::") {
                        ".".to_string()
                    } else {
                        self.path_of(child)
                    };
                    if part.is_empty() {
                        continue;
                    }
                    out.push_str(&part);
                }
                (!out.is_empty()).then_some(out)
            }
            _ => None,
        }
    }
}

/// Byte offset of the start of each line, so a node's line is a binary search.
fn line_starts(src: &[u8]) -> Vec<u32> {
    let mut out = vec![0u32];
    for (index, byte) in src.iter().enumerate() {
        if *byte == b'\n' {
            out.push(index as u32 + 1);
        }
    }
    out
}

/// The identifier characters at the start of `text`, if any.
fn leading_word(text: &str) -> Option<&str> {
    let end = text
        .char_indices()
        .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '_'))
        .map(|(at, _)| at)
        .unwrap_or(text.len());
    if end == 0 {
        None
    } else {
        Some(&text[..end])
    }
}
