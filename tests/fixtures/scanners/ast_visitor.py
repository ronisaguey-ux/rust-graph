"""Shapes lifted from felicity/scanners/ast_visitor.py (573 lines).

The one lexer-side fact this file lost is a call in the alternative of an
`if`/`else` expression; the rest of the differences are lexer omissions (the
one-pass lexer stops after the first call of a boolean chain), which the deep
path reads correctly.
"""

import ast

DECISION_NAMES = frozenset({"is_admin", "is_authorized", "has_permission"})
AUTHORITY_ATTRIBUTES = frozenset({"role", "permissions"})


def decides(child):
    if isinstance(child, ast.Name) and child.id.lower() in DECISION_NAMES:
        return True
    if isinstance(child, ast.Attribute) and child.attr.lower() in AUTHORITY_ATTRIBUTES:
        return True
    return False


def literal(v, literal_names):
    if isinstance(v, ast.Name) and (v.id.isupper() or v.id in literal_names):
        return v.id
    if isinstance(v, ast.Attribute) and isinstance(v.value, ast.Name):
        return v.value.id
    return None


def severity(self, file_path):
    return Severity.LOW if _looks_like_test_file(self.file_path) else Severity.HIGH


def host(host_node):
    if isinstance(host_node, ast.Constant) and host_node.value in ("0.0.0.0", ""):
        return True
    return False
