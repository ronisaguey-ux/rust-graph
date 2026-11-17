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
                                    // `getter=isWindow` is a keyed argument:
                                    // `["k","getter",["n","isWindow"]]`, the
                                    // shape the lexer writes for `a = b`.
                                    let mut form = String::from("[\"k\",");
                                    push_string(&mut form, item[..at].trim());
                                    form.push_str(",[\"n\",");
                                    push_string(&mut form, item[at + 1..].trim());
                                    form.push_str("]]");
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
            // `typedef struct aeApiState { … } aeApiState;` whose field list the
            // grammar cannot read leaves the closing declarator in an ERROR node
            // at file scope, on a line that opens with `}` and closes with `;`;
            // the lexer writes that name, so it is read from the line (13 files).
            "ERROR" => {
                // A header fragment — `{ 0x80, 36 },` a line, or a bare run of
                // numbers — is the whole file and one ERROR region: the grammar
                // has no statement to build, while the one-pass reader still
                // reads a value per brace item, or one tuple for a flat list.
                // Those values are on the lines, so they are read from the text.
                if self.c_fragment_events(id) {
                    return true;
                }
                // The region itself carries only the `}` (the declarator after it
                // is lost outright), so the name is read from the line.
                let line = self.line(id);
                if let (Some(start), Some(end)) = (
                    self.line_starts.get(line.saturating_sub(1) as usize).copied(),
                    self.line_starts.get(line as usize).copied(),
                ) {
                    let text = String::from_utf8_lossy(&self.src[start as usize..end as usize]);
                    let line_text = text.trim();
                    if let Some(rest) = line_text.strip_prefix('}') {
                        let name: String = rest
                            .trim_start()
                            .chars()
                            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                            .collect();
                        let tail = rest.trim_start()[name.len()..].trim();
                        if !name.is_empty() && tail == ";" {
                            let mut value = String::from("[\"n\",");
                            push_string(&mut value, &name);
                            value.push(']');
                            self.lexer_event(b'x', line, &[], &value, false);
                        }
                    }
                }
            }
            // `robj *bitroarCreate(void);`: a pointer return whose type the
            // grammar does not know is a `user_pointer_declaration`, and only
            // the declaration path read it - it never filed the call, so every
            // redis-style prototype on an unknown type lost it (10 files).
            "user_pointer_declaration" => {
                if self.child_of_kind(id, "parameter_list").is_some() {
                    // `lua_State *(lua_newstate) (lua_Alloc f, void *ud);`: the
                    // declarator is parenthesised, so there is no identifier
                    // child and the prototype wrote nothing at all.
                    if let Some(name) = self
                        .children(id)
                        .into_iter()
                        .find(|&child| self.kind(child) == "identifier")
                        .map(|node| self.text_string(node))
                        .filter(|name| !name.is_empty())
                        .or_else(|| self.declarator_name(id))
                    {
                        if !name.is_empty() {
                            let line = self.line(id);
                            let index = self.push_call(name.clone(), line);
                            // The line is `m:[n:robj,c(0;[0])]` to the lexer.
                            let text = self.text_string(id);
                            let mut names: Vec<String> = Vec::new();
                            let mut rest = text.trim_start();
                            let mut named = false;
                            while let Some(word) = leading_word(rest) {
                                if word == name {
                                    named = true;
                                    break;
                                }
                                if word != "void" {
                                    names.push(word.to_string());
                                }
                                rest = rest[word.len()..].trim_start();
                            }
                            // A parenthesised declarator never spells the name
                            // among the leading words, so the declaration lists
                            // it beside its type the way the lexer reads it.
                            if !named && !names.iter().any(|word| word == &name) {
                                names.push(name.clone());
                            }
                            let mut args: Vec<String> = Vec::new();
                            if let Some(list) = self.child_of_kind(id, "parameter_list") {
                                let mut group: Vec<String> = Vec::new();
                                for child in self.children(list) {
                                    if self.kind(child) == "," {
                                        args.push(self.c_param_form(&group));
                                        group.clear();
                                        continue;
                                    }
                                    group.extend(self.c_words_of(child));
                                }
                                args.push(self.c_param_form(&group));
                            }
                            let args: Vec<String> =
                                args.into_iter().filter(|arg| !arg.is_empty()).collect();
                            let call = format!("[\"c\",{index},0,[{}]]", args.join(","));
                            let mut parts: Vec<String> = names
                                .iter()
                                .map(|name| {
                                    let mut part = String::from("[\"n\",");
                                    push_string(&mut part, name);
                                    part.push(']');
                                    part
                                })
                                .collect();
                            parts.push(call);
                            self.lexer_event(
                                b'x',
                                line,
                                &[],
                                &format!("[\"m\",[{}]]", parts.join(",")),
                                false,
                            );
                        }
                    }
                } else {
                    // `TString *g = NULL;`: a pointer declarator with an
                    // initialiser and no parameter list is an ordinary
                    // declaration, and this arm wrote nothing for it, so the
                    // binding the lexer reads was lost (c corpus, 4 files).
                    let mut inits = Vec::new();
                    self.c_user_initializer(id, &mut inits);
                    if let Some((name, node, line)) = inits.first().cloned() {
                        self.assignment_parts(vec![name], node, line, false);
                    } else {
                        let form = self.c_words_form_skipping(id, &["initializer_list"]);
                        if form != "0" {
                            let line = self.line(id);
                            self.event(&format!("[\"x\",{line},{form}]"));
                        }
                    }
                }
            }
            // `return (void *)a_cas((volatile int *)p, (int)t, (int)s);`: the
            // call sits behind a cast and nothing descended into that node, so
            // every cast-receiver call in musl lost its call (3 shapes, 8 rows).
            "cast_expression" | "user_cast_expression" => {
                let mut calls = Vec::new();
                self.c_calls_in_pub(id, &mut calls);
                for call in calls {
                    self.form(call);
                }
            }
            // `hidden void __vm_wait(void);`: the macro before the type is its
            // own node, and the lexer keeps it as the first name of the line.
            "macro_qualified_item" => {
                if let Some(first) = self
                    .children(id)
                    .into_iter()
                    .find(|&child| self.kind(child) == "identifier")
                {
                    self.macro_prefix = Some(self.text_string(first));
                }
            }
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
                // `typedef enum { A = 0, B } T;`: the C grammar flattens the
                // enumerators into the body itself, so the shared `enum_body`
                // arm (which reads `enum_member` nodes) writes nothing for a C
                // enum. The lexer's model is its own: the words before the
                // brace, then one assignment to the first member when any
                // member has an initialiser - or the whole list as one value
                // when none does - and the declarator's name on its own line.
                if let Some(body) = self.c_enum_body(id) {
                    self.c_enum_declaration(id, body);
                    return true;
                }
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
                // `hidden void __vm_wait(void);`: a leading macro before the
                // return type makes the lexer read the declaration as names
                // plus a call of the declared function - `m:[n:hidden,c(0;[0])]`
                // - and the reader wrote the words form, so every musl
                // visibility-decorated prototype lost its line (36 files).
                // `size_t n1 = CHUNK_SIZE(self);`: the initialiser is a call
                // inside a `bare_user_declaration`, which c_initializers never
                // looked into, so the binding the lexer makes (`n1` holds
                // `CHUNK_SIZE`) was never made (18 files).
                for child in self.children(id) {
                    if self.kind(child) != "bare_user_declaration" {
                        continue;
                    }
                    let inner = self.children(child);
                    if let Some(call) = inner
                        .iter()
                        .copied()
                        .find(|&node| self.kind(node) == "call_expression")
                    {
                        let name = inner
                            .iter()
                            .copied()
                            .find(|&node| self.kind(node) == "identifier")
                            .map(|node| self.text_string(node))
                            .unwrap_or_default();
                        let path = self.decl_path(call);
                        let last = path.rsplit('.').next().unwrap_or("").to_string();
                        if !name.is_empty() && !last.is_empty() {
                            self.facts.vars.push((name, last));
                        }
                    }
                }
                // `LUA_API lua_State *(lua_newstate) (lua_Alloc f, void *ud);`:
                // a macro in front of a parenthesised declarator is not part of
                // the value — the lexer drops it and reads the declaration as
                // the call of the parenthesised name alone — so the arm for that
                // declarator below owns the statement. The prefix is still taken
                // so it cannot leak into the next declaration.
                let prefix = self.macro_prefix.take();
                let forced = if self.c_own_parenthesized_declarator(id) { None } else { prefix };
                if let Some(word) = forced
                    .clone()
                    .or_else(|| leading_word(self.text_string(id).trim_start()).map(str::to_string))
                {
                    const KEYWORDS: &[&str] = &[
                        "void", "int", "char", "short", "long", "float", "double", "signed",
                        "unsigned", "const", "volatile", "static", "extern", "inline", "register",
                        "auto", "struct", "union", "enum", "typedef", "_Bool",
                    ];
                    if forced.is_some() || !KEYWORDS.contains(&word.as_str()) {
                        if let (Some(list), Some(name_node)) = (
                            self.c_descendant(id, "parameter_list"),
                            self.child_of_kind(id, "direct_declarator")
                                .and_then(|node| self.first_identifier(node))
                                // `hidden off_t __lseek(int, off_t, int);`: a
                                // return type the grammar does not know becomes
                                // a `bare_user_declaration`, whose name is a
                                // direct child of it, so the declarator lookup
                                // found nothing and the macro word in front of
                                // the type was dropped from a prototype the
                                // lexer keeps whole.
                                .or_else(|| {
                                    let bare = self.c_descendant(id, "bare_user_declaration")?;
                                    let kids = self.children(bare);
                                    let at = kids.iter().rposition(|&node| self.kind(node) == "identifier")?;
                                    Some(self.text_string(kids[at]))
                                }),
                        ) {
                            let name = name_node;
                            if !name.is_empty() {
                                let line = self.line(id);
                                let text = self.text_string(id);
                                let mut names: Vec<String> =
                                    forced.clone().map(|prefix| vec![prefix]).unwrap_or_default();
                                let mut rest = text.trim_start();
                                while let Some(word) = leading_word(rest) {
                                    if word == name {
                                        break;
                                    }
                                    if word != "void" {
                                        names.push(word.to_string());
                                    }
                                    rest = rest[word.len()..].trim_start();
                                }
                                let mut args: Vec<String> = Vec::new();
                                let mut group: Vec<String> = Vec::new();
                                for child in self.children(list) {
                                    if self.kind(child) == "," {
                                        args.push(self.c_param_form(&group));
                                        group.clear();
                                        continue;
                                    }
                                    group.extend(self.c_words_of(child));
                                }
                                args.push(self.c_param_form(&group));
                                let args: Vec<String> = args.into_iter().filter(|a| !a.is_empty()).collect();
                                let index = self.push_call(name.clone(), line);
                                let call = format!("[\"c\",{index},0,[{}]]", args.join(","));
                                let mut parts: Vec<String> = names
                                    .iter()
                                    .map(|name| {
                                        let mut part = String::from("[\"n\",");
                                        push_string(&mut part, name);
                                        part.push(']');
                                        part
                                    })
                                    .collect();
                                parts.push(call);
                                self.lexer_event(b'x', line, &[], &format!("[\"m\",[{}]]", parts.join(",")), false);
                                return true;
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
                        // An empty argument list is still a list: `["c",3,0,]` is not JSON,
                        // and a prototype with no readable declarator name has one.
                        if args.is_empty() {
                            args.push_str("[]");
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
                // `extern hidden volatile int *const __x_lockptr;` splits into
                // two declarations on one line when the type is not a keyword,
                // and the lexer reads the line as one run of names with `const`
                // dropped - so the event written per statement was missing the
                // first one's words (23 musl files, 52 lines).
                if inits.is_empty() {
                    let line = self.line(id);
                    if let Some(text) = self.line_text(line) {
                        let first = leading_word(text.trim_start()).unwrap_or("");
                        if matches!(first, "extern" | "hidden" | "weak") && !text.contains('(') {
                            let words: Vec<String> = text
                                .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                                .filter(|word| !word.is_empty() && *word != "const")
                                .map(str::to_string)
                                .collect();
                            if words.len() > 2 {
                                let parts: Vec<String> = words
                                    .iter()
                                    .map(|word| {
                                        let mut part = String::from("[\"n\",");
                                        push_string(&mut part, word);
                                        part.push(']');
                                        part
                                    })
                                    .collect();
                                let value = format!("[\"m\",[{}]]", parts.join(","));
                                self.lexer_event(b'x', line, &[], &value, false);
                                wrote_prototype = true;
                            }
                        }
                    }
                }
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
                        // `static const uint32_t psbox[8][64] = { { … }, … };`:
                        // a brace list of brace lists is a merge of merges, one
                        // inner merge per row - counting only the outer children
                        // flattened the whole table to eight zeros.
                        let parts = self.c_initializer_elements(node);
                        let value = format!("[\"m\",[{}]]", parts.join(","));
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
                // `CURLcode Curl_altsvc_load(struct altsvcinfo *asi);` and the
                // multi-line form: a return type the grammar does not know
                // becomes a `bare_user_declaration` wrapping the declarator and
                // its parameters, so the prototype arm above saw no parameter
                // list of its own and every such prototype lost both the call
                // and the line's event.
                if !self.c_has_kind(id, "compound_statement") {
                    for child in self.children(id) {
                        if self.kind(child) != "bare_user_declaration"
                            || !self.c_has_kind(child, "parameter_list")
                        {
                            continue;
                        }
                        let inner = self.children(child);
                        let Some(name_at) = inner.iter().position(|&node| self.kind(node) == "identifier")
                        else {
                            continue;
                        };
                        let name = self.text_string(inner[name_at]);
                        if name.is_empty() {
                            continue;
                        }
                        let line = self.line(id);
                        let index = self.push_call(name.clone(), line);
                        let name_form = |text: &str| {
                            let mut form = String::from("[\"n\",");
                            push_string(&mut form, text);
                            form.push(']');
                            form
                        };
                        let mut parts: Vec<String> = Vec::new();
                        for &node in &inner[..name_at] {
                            for word in self.c_words_of(node) {
                                if matches!(word.as_str(), "const" | "static" | "extern" | "inline") {
                                    continue;
                                }
                                parts.push(name_form(&word));
                            }
                        }
                        if let Some(list) = inner.iter().copied().find(|&node| self.kind(node) == "parameter_list") {
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
            }
            _ => return false,
        }
        true
    }


    /// `(socklen_t)ss_size`, `(char *)ai->ai_addr + ss_size`: the one-pass
    /// lexer reads a cast as the type's words followed by the operand's value,
    /// and nothing here read the node at all, so every initialiser or
    /// assignment whose value is a cast was written as the constant 0.
    pub(crate) fn c_cast_form(&mut self, id: u32) -> Option<String> {
        if !matches!(self.kind(id), "cast_expression" | "user_cast_expression") {
            return None;
        }
        let children = self.children(id);
        let ty = children
            .iter()
            .copied()
            .find(|&child| matches!(self.kind(child), "cast_type" | "user_cast_head"))?;
        let operand = children
            .iter()
            .copied()
            .rev()
            .find(|&child| !self.is_layout(child) && child != ty)?;
        let words = self.c_words_of(ty);
        if words.is_empty() {
            return None;
        }
        let name_form = |text: &str| {
            let mut out = String::from("[\"n\",");
            push_string(&mut out, text);
            out.push(']');
            out
        };
        let ty_form = if words.len() == 1 {
            name_form(&words[0])
        } else {
            let parts: Vec<String> = words.iter().map(|word| name_form(word)).collect();
            format!("[\"m\",[{}]]", parts.join(","))
        };
        let operand_form = self.form(operand);
        Some(format!("[\"m\",[{ty_form},{operand_form}]]"))
    }

    /// The enum body this declaration declares, when the declaration is an
    /// enum's own. A struct's fields are not descended into: their enums are
    /// their own declarations.
    fn c_enum_body(&self, id: u32) -> Option<u32> {
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            for child in self.children(node) {
                match self.kind(child) {
                    "enum_body" => return Some(child),
                    "field_declaration_list" | "struct_body" | "compound_statement" => {}
                    _ => stack.push(child),
                }
            }
        }
        None
    }

    /// `typedef enum { A = 0, B } T;`, written the way the one-pass lexer
    /// reads it.
    fn c_enum_declaration(&mut self, id: u32, body: u32) {
        let decl_line = self.line(id);
        let start = self.node(id).start as usize;
        let at = self.node(body).start as usize;
        let header = String::from_utf8_lossy(&self.src[start..at]).to_string();
        let words: Vec<String> = header
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect();
        let name_form = |text: &str| {
            let mut out = String::from("[\"n\",");
            push_string(&mut out, text);
            out.push(']');
            out
        };
        // `{ A = 0 , B , C }`: the separators are children too, and the node
        // after an `=` is that member's initialiser rather than a new member.
        let mut members: Vec<(String, u32, bool)> = Vec::new();
        let mut after_eq = false;
        for child in self.children(body) {
            let kind = self.kind(child);
            if matches!(kind, "{" | "}" | ",") {
                continue;
            }
            if kind == "=" {
                after_eq = true;
                continue;
            }
            if after_eq {
                if let Some(last) = members.last_mut() {
                    last.2 = true;
                }
                after_eq = false;
                continue;
            }
            if kind == "identifier" {
                let name = self.text_string(child);
                let line = self.line(child);
                members.push((name, line, false));
            }
        }
        let declarator = self
            .child_of_kind(id, "direct_declarator")
            .and_then(|node| self.declarator_of(node));
        if !members.iter().any(|member| member.2) {
            // `enum { A, B } e;`: one value, the words before the brace, the
            // enumerators as a list, and the declared name.
            let mut parts: Vec<String> = words.iter().map(|word| name_form(word)).collect();
            if !members.is_empty() {
                let names: Vec<String> = members.iter().map(|member| name_form(&member.0)).collect();
                parts.push(format!("[\"m\",[{}]]", names.join(",")));
            }
            if let Some(name) = &declarator {
                parts.push(name_form(name));
            }
            if !parts.is_empty() {
                let value = format!("[\"m\",[{}]]", parts.join(","));
                self.event(&format!("[\"x\",{decl_line},{value}]"));
            }
            return;
        }
        if !words.is_empty() {
            let parts: Vec<String> = words.iter().map(|word| name_form(word)).collect();
            let value = format!("[\"m\",[{}]]", parts.join(","));
            self.event(&format!("[\"x\",{decl_line},{value}]"));
        }
        if let Some((first, line, _)) = members.first() {
            let value = if members.len() == 1 {
                "0".to_string()
            } else {
                let mut parts = vec!["0".to_string()];
                for member in &members[1..] {
                    parts.push(name_form(&member.0));
                }
                format!("[\"t\",[{}]]", parts.join(","))
            };
            let target = first.clone();
            let line = *line;
            self.lexer_event(b'a', line, &[target], &value, false);
        }
        if let Some(name) = declarator {
            let line = self
                .child_of_kind(id, "direct_declarator")
                .map(|node| self.line(node))
                .unwrap_or(decl_line);
            let value = name_form(&name);
            self.event(&format!("[\"x\",{line},{value}]"));
        }
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
                // `size_t n = CHUNK_SIZE(self);` and `TString *g = NULL;` are
                // `(bare_user_declaration (user_type_name ..) identifier =
                // value)` and `(user_pointer_declaration ..)`: the declarator is
                // a bare identifier, not a `direct_declarator`, so the binding
                // and its value were never read and the declaration wrote only
                // the words form (c corpus, 40 rows).
                "bare_user_declaration" | "user_pointer_declaration" => {
                    self.c_user_initializer(child, out);
                    continue;
                }
                "direct_declarator" => {
                    pending = self.declarator_of(child);
                    continue;
                }
                // A pointer belongs to the declarator that follows it, not to
                // the previous one: in char *a, **b; the two pointers of b
                // were read as a's initialiser, so the first name was written
                // as an assignment holding a pointer and every later name was
                // dropped from the record.
                "pointer" => {
                    pending = None;
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

    /// `size_t n = CHUNK_SIZE(self);` is `(bare_user_declaration
    /// (user_type_name identifier) identifier = value ;)`: the declared name is
    /// the first identifier and the initialiser is the node after the `=`.
    /// A declarator written as a bare identifier is not a `direct_declarator`,
    /// so the binding was never read.
    fn c_user_initializer(&mut self, id: u32, out: &mut Vec<(String, u32, u32)>) {
        let kids = self.children(id);
        let Some(name_at) = kids.iter().position(|&child| self.kind(child) == "identifier") else { return };
        let name = self.text_string(kids[name_at]);
        let Some(equal) = kids.iter().skip(name_at).position(|&child| self.kind(child) == "=") else { return };
        let Some(&value) = kids.get(name_at + equal + 1) else { return };
        if self.is_layout(value) {
            return;
        }
        let line = self.line(value);
        out.push((name, value, line));
    }

    /// One event per line for a header fragment: a file whose whole body is a
    /// run of numbers, commas and braces. `{ 0x80, 36 },` is a tuple of two
    /// constants to the one-pass reader and a flat list of numbers is one tuple
    /// of all of them, so the region is read from the text, not from a tree the
    /// grammar could not build. Anything else in the region leaves it alone.
    fn c_fragment_events(&mut self, id: u32) -> bool {
        let text = self.text(id);
        if text.is_empty() || text.len() * 10 < self.src.len() * 9 {
            return false;
        }
        // (byte offset, brace depth, which top-level brace group it is in)
        let mut numbers: Vec<(u32, usize, usize)> = Vec::new();
        let mut depth = 0usize;
        let mut groups = 0usize;
        let mut at = 0usize;
        while at < text.len() {
            let byte = text[at];
            if byte.is_ascii_digit() {
                let begin = at as u32;
                while at < text.len() && (text[at].is_ascii_alphanumeric() || text[at] == b'.' || text[at] == b'_') {
                    at += 1;
                }
                numbers.push((begin, depth, groups));
                continue;
            }
            match byte {
                b'{' => {
                    if depth == 0 {
                        groups += 1;
                    }
                    depth += 1;
                    at += 1;
                }
                b'}' => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                    at += 1;
                }
                b',' | b' ' | b'\n' | b'\r' | b'\t' => at += 1,
                _ => return false,
            }
        }
        if depth != 0 || numbers.is_empty() {
            return false;
        }
        let braced = text.contains(&b'{');
        let mut items: Vec<(u32, usize)> = Vec::new();
        if braced {
            let mut group: Option<usize> = None;
            for &(offset, depth, which) in &numbers {
                if depth != 1 {
                    continue;
                }
                if group != Some(which) {
                    items.push((offset, 0));
                    group = Some(which);
                }
                if let Some(last) = items.last_mut() {
                    last.1 += 1;
                }
            }
        } else if let Some(&(offset, _, _)) = numbers.first() {
            items.push((offset, numbers.len()));
        }
        for (offset, count) in items {
            if count < 2 {
                continue;
            }
            let line = self.line_at(offset);
            let mut value = "[\"t\",[".to_string();
            for index in 0..count {
                if index > 0 {
                    value.push(',');
                }
                value.push('0');
            }
            value.push_str("]]");
            self.lexer_event(b'x', line, &[], &value, false);
        }
        true
    }

    /// The line a byte offset sits on.
    fn line_at(&self, offset: u32) -> u32 {
        match self.line_starts.partition_point(|&start| start <= offset) {
            0 => 1,
            found => found as u32,
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

    /// Whether the declaration's own declarator is parenthesised — `int
    /// (name)(args)` — rather than a parameter's, which every prototype with a
    /// function-pointer parameter also has. A macro in front of the first is not
    /// part of the value the lexer reads; in front of the second it is.
    fn c_own_parenthesized_declarator(&self, id: u32) -> bool {
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            for child in self.children(node) {
                match self.kind(child) {
                    "parenthesized_declarator" => return true,
                    "parameter_list" | "compound_statement" | "initializer_list" => {}
                    _ => stack.push(child),
                }
            }
        }
        false
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
        // `int Guard::level() const { … }`: the lexer reads the qualified name
        // as a call, and the definition's own node never wrote one, so every
        // out-of-line member definition lost the call (cpp: 108 call rows).
        if let Some(scoped) = self.child_of_kind(id, "scoped_type_name") {
            let text = self.text_string(scoped).replace("::", ".");
            let path = text.trim().trim_start_matches('.').to_string();
            if !path.is_empty() {
                self.push_call(path, self.line(scoped));
            }
        }
        let params = declarator
            .and_then(|node| self.child_of_kind(node, "parameter_list"))
            .or_else(|| children.iter().copied().find(|&child| self.kind(child) == "parameter_list"))
            .map(|list| self.c_params(list))
            .unwrap_or_default();
        let owner = if self.frame().container { self.frame().class.clone() } else { String::new() };
        let index = self.add_symbol(name.clone(), owner.clone(), KIND_FUNCTION, line, params);
        self.stack.push(Frame { node: id, symbol: index, class: owner, receiver: String::new(), container: false, defines: Some(index) });
    }

    /// A parameter group as the lexer reads it: a type with no name is the
    /// constant 0, `...` is the constant 0, and a name is written as it stands.
    pub(crate) fn c_param_form(&self, group: &[String]) -> String {
        let Some(last) = group.last() else { return String::new() };
        if last == "..." || (group.len() == 1 && last == "void") {
            return "0".to_string();
        }
        if group.len() > 1 {
            let parts: Vec<String> = group
                .iter()
                .map(|word| {
                    let mut part = String::from("[\"n\",");
                    push_string(&mut part, word);
                    part.push(']');
                    part
                })
                .collect();
            return format!("[\"m\",[{}]]", parts.join(","));
        }
        let mut form = String::from("[\"n\",");
        push_string(&mut form, last);
        form.push(']');
        form
    }

    /// One source line, without its break, when it is in range.
    pub(crate) fn line_text(&self, line: u32) -> Option<String> {
        let start = self.line_starts.get(line.saturating_sub(1) as usize).copied()?;
        let end = self
            .line_starts
            .get(line as usize)
            .copied()
            .unwrap_or(self.src.len() as u32);
        Some(String::from_utf8_lossy(&self.src[start as usize..end as usize]).trim_end().to_string())
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
        let mut macro_members: Vec<u32> = Vec::new();
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
            let index = self.push_call(name.clone(), line);
            // The line is the call itself: `BIT(header);` is `c(0;[n:header])`
            // to the lexer, and the member walk wrote `m:[n:BIT,n:header]`.
            let mut args: Vec<String> = Vec::new();
            if let Some(paren) = self
                .children(child)
                .into_iter()
                .find(|&node| self.kind(node) == "parenthesized_declarator")
            {
                for word in self.c_words_of(paren) {
                    if word == name {
                        continue;
                    }
                    let mut form = String::from("[\"n\",");
                    push_string(&mut form, &word);
                    form.push(']');
                    args.push(form);
                }
            }
            let value = format!("[\"c\",{index},0,[{}]]", args.join(","));
            self.lexer_event(b'x', line, &[], &value, false);
            macro_members.push(child);
        }
        // `int (*configure)(void *priv, int reconfigure);`: a field that is a
        // function pointer is the declared name applied, so the one-pass reader
        // writes a call with that name as its argument — `c(0;[n:configure])` —
        // and the member walk wrote the header's words instead. Every such
        // field in redis's `connection.h`-style vtables lost its line.
        let mut pointer_lines: Vec<u32> = Vec::new();
        for child in self.children(body) {
            // The declarator is a direct child of the body, or one level down
            // inside a `direct_declarator`: `int (*compress)(…)` and
            // `connection* (*conn_create)(…)` are spelled the two ways.
            let mut paren = self
                .children(child)
                .into_iter()
                .find(|&node| self.kind(node) == "parenthesized_declarator");
            if paren.is_none() {
                if let Some(declarator) = self
                    .children(child)
                    .into_iter()
                    .find(|&node| self.kind(node) == "direct_declarator")
                {
                    paren = self
                        .children(declarator)
                        .into_iter()
                        .find(|&node| self.kind(node) == "parenthesized_declarator");
                }
            }
            let Some(paren) = paren else { continue };
            let Some(name) = self.declarator_of(paren) else { continue };
            if name.is_empty() {
                continue;
            }
            let line = self.line(child);
            let index = self.push_call(name.clone(), line);
            let mut arg = String::from("[\"n\",");
            push_string(&mut arg, &name);
            arg.push(']');
            let value = format!("[\"c\",{index},0,[{}]]", arg);
            self.lexer_event(b'x', line, &[], &value, false);
            // The line is that call alone: the type words the header walk
            // would write are not part of the value the lexer reads.
            pointer_lines.push(line);
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
            if macro_members.contains(&node) {
                continue;
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
            if pointer_lines.contains(&line) {
                continue;
            }
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

    /// A brace initialiser's value: one part per element, each the element's
    /// own value. `int K[64] = { 0, 0, … }` is a row of constants and stays
    /// one; `ehooks_default_extent_hooks = { ehooks_default_alloc, … }` is a row
    /// of names, and writing the constant 0 for each element made every
    /// initialised table of function pointers disagree with the lexer, which
    /// reads the name (c corpus, 125 rows).
    pub(crate) fn c_initializer_elements(&mut self, node: u32) -> Vec<String> {
        // `{ 1, 2 }` is its elements alone: the braces are punctuation the
        // lexer never writes as a value, and reading them as two more elements
        // put a constant at each end of every brace list in the corpus
        // (`{ .a = x, .b = y, .c = 0 }` was five elements: three names the
        // readers wrote and two zeros neither of them did).
        let kids: Vec<u32> = self
            .children(node)
            .into_iter()
            .filter(|&child| !self.is_layout(child) && !matches!(self.kind(child), "," | "{" | "}"))
            .collect();
        kids.into_iter()
            .map(|child| {
                if self.kind(child) == "initializer_list" {
                    format!("[\"m\",[{}]]", self.c_initializer_elements(child).join(","))
                } else {
                    self.form(child)
                }
            })
            .collect()
    }

    /// `.field = value` inside a brace list. The one-pass reader writes the
    /// field name and, when the value is one it writes, the value beside it in
    /// one merge: `.getExpireMeta = getMyItemExpireMeta` reads as
    /// `m:[n:getExpireMeta,n:getMyItemExpireMeta]`. A value it writes nothing
    /// for — a plain number, or an operator over numbers — leaves the name
    /// alone, so `.itemsAddrAreOdd = 0` reads as `n:itemsAddrAreOdd`. A char, a
    /// string or a parenthesised expression is the constant `0` it does write,
    /// and keeps both halves. None of this was read here, so every designated
    /// initializer in the corpus was a run of constants.
    pub(crate) fn c_designated_form(&mut self, id: u32) -> Option<String> {
        if self.kind(id) == "initializer_list" {
            let parts = self.c_initializer_elements(id);
            return Some(if parts.is_empty() {
                "0".to_string()
            } else {
                format!("[\"m\",[{}]]", parts.join(","))
            });
        }
        if self.kind(id) != "designated_field" {
            return None;
        }
        let kids: Vec<u32> = self
            .children(id)
            .into_iter()
            .filter(|&child| !self.is_layout(child) && self.kind(child) != "=")
            .collect();
        let key_node = *kids.iter().find(|&&child| self.kind(child) == "identifier")?;
        let key = self.text_string(key_node);
        if key.is_empty() {
            return None;
        }
        let mut name = "[\"n\",".to_string();
        push_string(&mut name, &key);
        name.push(']');
        let Some(&value) = kids.iter().find(|&&child| child != key_node) else { return Some(name) };
        let form = self.form(value);
        if form == "0" && !self.c_writes_a_constant(value) {
            return Some(name);
        }
        Some(format!("[\"m\",[{},{}]]", name, form))
    }

    /// Whether the one-pass reader writes the constant `0` for this value: a
    /// char or string literal and a parenthesised expression do, while a
    /// number — or an operator over numbers — writes nothing at all.
    fn c_writes_a_constant(&self, id: u32) -> bool {
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            if matches!(
                self.kind(node),
                "string" | "char_literal" | "parenthesized_expression" | "initializer_list"
            ) {
                return true;
            }
            stack.extend(self.children(node));
        }
        false
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
    /// `global_State *g = G(L);` at statement level: an unknown type and a
    /// pointer parse as a product, so the declared name is the product's right
    /// operand and the value is what the assignment stores.
    pub(crate) fn c_product_declarator(&self, id: u32) -> Option<(String, u32)> {
        if self.kind(id) != "assignment_expression" {
            return None;
        }
        let children = self.children(id);
        let at = children.iter().position(|&child| self.kind(child) == "=")?;
        let target = *children[..at].iter().rev().find(|&&child| !self.is_layout(child))?;
        if self.kind(target) != "binary_expression" {
            return None;
        }
        let inner = self.children(target);
        if inner.iter().filter(|&&child| self.kind(child) == "*").count() != 1 {
            return None;
        }
        let name = *inner.last()?;
        if self.kind(name) != "identifier" {
            return None;
        }
        let name = self.text_string(name);
        if name.is_empty() {
            return None;
        }
        let value = *children.get(at + 1)?;
        Some((name, value))
    }

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
            // `mh->db[mh->num_dbs].dbid`: a subscript is transparent in a
            // path — the lexer cuts the bracket off the target it reads, so
            // `suite[0].children` names `suite.children` — and the receiver's
            // path continues through it. Read as a leaf it named `.dbid`.
            "subscript_expression" | "subscript" | "index_expression" => {
                let object = self.children(id).into_iter().next()?;
                self.c_path(object).or_else(|| {
                    let path = self.path_of(object);
                    (!path.is_empty()).then_some(path)
                })
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

#[cfg(test)]
mod designated_initializer_tests {
    use super::*;
    use crate::deep_integration::{self, Lang};

    /// The bindings the reader writes, as `name -> what it holds` pairs.
    fn module_vars(source: &str) -> Vec<(String, String)> {
        let tree = deep_integration::parse_file(source.as_bytes(), Lang::C).expect("a C tree");
        extract(source.as_bytes(), &tree, Lang::C).vars
    }

    fn module_events(source: &str) -> String {
        let tree = deep_integration::parse_file(source.as_bytes(), Lang::C).expect("a C tree");
        let facts = extract(source.as_bytes(), &tree, Lang::C);
        facts
            .symbols
            .iter()
            .find(|symbol| symbol.kind == KIND_MODULE)
            .map(|symbol| symbol.events.clone())
            .unwrap_or_default()
    }

    /// `.x = y` is the field and its value in one merge: the one-pass reader
    /// writes both, and reading the field as a constant lost every designated
    /// initializer in the corpus.
    #[test]
    fn a_designated_field_keeps_its_name_and_value() {
        let events = module_events("struct S { int x; };\nstruct S s = { .x = y };\n");
        assert!(events.contains("[\"m\",[[\"n\",\"x\"],[\"n\",\"y\"]]]"), "{events}");
    }

    /// A number is a value the one-pass reader writes nothing for, so the
    /// element is the field name alone — `.x = 0` may not become a pair with a
    /// constant the reader never wrote.
    #[test]
    fn a_constant_designated_field_is_the_name_alone() {
        let events = module_events("struct S { int x; };\nstruct S s = { .x = 0 };\n");
        assert!(events.contains("[\"n\",\"x\"]"), "{events}");
        assert!(!events.contains("[[\"n\",\"x\"],0]"), "{events}");
    }

    /// A header fragment is data the grammar cannot build a statement for, and
    /// the one-pass reader still writes a value per brace item. Those lines are
    /// the file's record, so both readers have to write them.
    #[test]
    fn a_header_fragment_writes_a_tuple_per_brace_item() {
        let events = module_events("{ 0x80, 36 },\n{ 0x81, 37 },\n");
        assert_eq!(events.matches("[\"t\",[0,0]]").count(), 2, "{events}");
    }

    /// `int (*configure)(void *priv, int reconfigure);`: a struct field that is
    /// a function pointer is the declared name applied to the one-pass reader,
    /// and the member walk wrote the header's words instead.
    #[test]
    fn a_function_pointer_member_is_a_call_of_its_name() {
        let events = module_events("struct T {\n    int (*configure)(int a);\n};\n");
        assert!(events.contains("[[\"n\",\"configure\"]]"), "{events}");
        assert!(!events.contains("[\"m\",[[\"n\",\"int\"],[\"n\",\"configure\"]]]"), "{events}");
    }

    /// `LUA_API int (lua_isuserdata) (lua_State *L, int idx);`: the macro in
    /// front of a parenthesised declarator is not part of the value — the
    /// one-pass reader drops it and reads the declaration as the call alone.
    #[test]
    fn a_macro_before_a_parenthesised_declarator_is_not_a_value() {
        let events = module_events("LUA_API int (lua_isuserdata) (int idx);\n");
        assert!(events.contains("[\"c\",0,0,[[\"n\",\"lua_isuserdata\"]]]"), "{events}");
        assert!(!events.contains("LUA_API"), "{events}");
    }

    /// The same macro in front of a prototype whose *parameter* is the
    /// parenthesised one stays: the one-pass reader keeps it as the line's
    /// first name.
    #[test]
    fn a_macro_before_a_pointer_parameter_stays_a_name() {
        let events = module_events("JEMALLOC_EXPORT int cb(void *(calloc_cb)(int, int));\n");
        assert!(events.contains("JEMALLOC_EXPORT"), "{events}");
    }

    /// The braces around a brace list are punctuation, not two more elements:
    /// counting them put a constant at each end of every table.
    #[test]
    fn a_brace_list_is_its_elements_without_the_braces() {
        let events = module_events("int t[2] = { 1, 2 };\n");
        assert!(events.contains("[\"m\",[0,0]]"), "{events}");
        assert!(!events.contains("[\"m\",[0,0,0,0]]"), "{events}");
    }

    /// `global_State *g = G(L);`: an unknown type and a pointer parse as a
    /// product, and the one-pass reader binds `g` to the capitalised call on
    /// the right. Nothing named the declared operand, so every macro
    /// initialiser on a pointer declaration in lua and the redis modules was a
    /// variable the tree did not have (15 files).
    #[test]
    fn a_macro_initialiser_on_a_pointer_declaration_binds_its_variable() {
        let vars = module_vars("void f(void) {\n  global_State *g = G(L);\n}\n");
        assert!(vars.contains(&("g".to_string(), "G".to_string())), "{vars:?}");
    }
}
