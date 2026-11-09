"""Shapes lifted from felicity/scanners/legacy_source.py (422 lines).

`match` is a variable here, not a statement. The lexer reads `match.start()`
as a call on the name `match`; the CST carries the soft-keyword kind, and the
mapping has to spell the name from the leaf. The slice bounds hold two calls
per expression and both are facts.
"""

import re

PATTERN = re.compile(r"(?m)^\s*(?:eval|exec)\s*\(")


def first_use(source):
    match = PATTERN.search(source)
    if match is None:
        return None
    line = source[:match.start()].count("\n") + 1
    argument = source[match.start():match.start() + 120]
    return line, argument.strip()


def every_use(source):
    for match in PATTERN.finditer(source):
        before = source[max(0, match.start() - 20):match.start()]
        after = source[match.end():match.end() + 3].lstrip()
        column = match.start() - (source.rfind("\n", 0, match.start()) + 1)
        yield before, after, column


def body_of(source):
    match = PATTERN.search(source)
    end = match.end()
    start = match.start()
    return source[start:end]
