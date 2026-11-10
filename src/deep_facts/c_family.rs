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
        matches!(self.language.name, "c" | "cpp")
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
        if self.child_of_kind(id, "class_body").is_none() && self.child_of_kind(id, "struct_body").is_none() {
            return;
        }
        let Some(name_node) = self.child_of_kind(id, "identifier") else { return };
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
                let name = self.member_name(id);
                let object = self.children(id).into_iter().next()?;
                let base = self.path_of(object);
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
