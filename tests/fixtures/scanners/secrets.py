"""Shapes lifted from felicity/scanners/secrets.py (524 lines).

The parenthesised `from felicity.config import (...)` is the second of the two
import losses on the real corpus, and `type(candidate)` / `match.group` are the
soft-keyword shapes: this grammar lexes `match`, `case` and `type` as keyword
kinds wherever they appear, so the mapping has to read them as names.
"""

import re
from pathlib import Path
from felicity.core.finding import Finding, Severity
from felicity.config import (
    SHANNON_ENTROPY_THRESHOLD,
    MIN_SECRET_LENGTH,
    MAX_SECRET_LENGTH,
)

UNQUOTED_ASSIGNMENT_PATTERN = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(\S+)")


def scan_line(line, findings, line_idx):
    ent_match = UNQUOTED_ASSIGNMENT_PATTERN.search(line.rstrip())
    if ent_match:
        candidate = ent_match.group(1)
        already_caught = any(f.line_number == line_idx for f in findings)
        if not already_caught and not _is_placeholder_value(candidate):
            entropy = calculate_shannon_entropy(candidate)
            strong = len(candidate) >= MIN_SECRET_LENGTH and entropy >= SHANNON_ENTROPY_THRESHOLD
            if strong or looks_random(candidate):
                return type(candidate), candidate
    for match in UNQUOTED_ASSIGNMENT_PATTERN.finditer(line):
        if match.group(2):
            yield match.group(2)
