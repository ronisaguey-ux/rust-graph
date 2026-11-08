//! Tree inspection helpers for the tests. Every walk is iterative, so even the
//! million-deep tree can be rendered.

#![allow(dead_code)]

use deep_rt::{Action, Node, NodeId, StateId, SymbolId, NONE};

use super::{Tables, ASSIGN, BLOCK, COLON, DEF, DEFN, EQ, FILE, ITEM, ITEMS, LBRACE, LP, NAME, NUM,
            RBRACE, RP, SEMI, BAD};

pub fn children(nodes: &[Node], id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut at = nodes[id as usize].first_child;
    while at != NONE {
        out.push(at);
        at = nodes[at as usize].next_sibling;
    }
    out
}

pub fn kind_name(symbol: SymbolId) -> &'static str {
    match symbol {
        DEF => "DEF",
        NAME => "NAME",
        NUM => "NUM",
        EQ => "=",
        SEMI => ";",
        LP => "(",
        RP => ")",
        COLON => ":",
        LBRACE => "{",
        RBRACE => "}",
        BAD => "BAD",
        FILE => "FILE",
        ITEMS => "ITEMS",
        ITEM => "ITEM",
        DEFN => "DEF",
        BLOCK => "BLOCK",
        ASSIGN => "ASSIGN",
        _ => "?",
    }
}

/// The shape of the tree as text, rendered without recursion.
pub fn shape(nodes: &[Node], root: NodeId) -> String {
    let mut out = String::new();
    // (node, closing?) work items.
    let mut work: Vec<(NodeId, bool)> = vec![(root, false)];
    while let Some((id, closing)) = work.pop() {
        let node = nodes[id as usize];
        if closing {
            out.push(')');
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push('(');
        out.push_str(kind_name(node.kind));
        let kids = children(nodes, id);
        work.push((id, true));
        for &child in kids.iter().rev() {
            work.push((child, false));
        }
    }
    out
}

/// Every node in the tree, in document order.
pub fn all(nodes: &[Node], root: NodeId) -> Vec<NodeId> {
    deep_rt::preorder(nodes, root)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

pub fn action_cell(tables: &Tables, state: usize, symbol: SymbolId) -> Action {
    let grammar = tables.grammar();
    deep_rt::table::action(&grammar, state as StateId, symbol)
}
