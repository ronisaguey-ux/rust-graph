"""Shapes lifted from felicity/scanners/interprocedural.py (1217 lines).

Every construct here lost a fact to the mapping layer on the real file: the
parenthesised multi-line import, calls inside f-string fields, calls in an
`if`/`else` expression, calls in a lambda body, and calls in a slice.
"""

from pathlib import Path
from felicity.scanners.graph_markers import (
    ARGV_CALLS, ASSIGNMENT_SINKS, CWE, EXECUTION_CLASSES, LOCAL_SOURCES, SINK_CLASSES, class_of,
    match_sanitizer, match_sink, match_source_call, match_source_path)

_WORKSPACE = Path(__file__).resolve().parent.parent.parent.parent
_PACKAGE = Path(__file__).resolve().parents[2]


def _grade(hit):
    order = {"low": 1, "medium": 2, "high": 3, "critical": 4}
    return order.get(hit.severity, 0)


def ranked(hits):
    return sorted(hits, key=lambda hit: (order.get(_grade(hit), 4), len(hit.chain), hit.entry, hit.chain))


def describe(hit):
    description = f" The same sink has {len(hit.entry)} entry points: {', '.join(map(short_name, hit.chain))}."
    description += f" This entry reaches {len(hit.sinks)} sinks of this class: {', '.join(map(short_name, hit.sinks))}."
    title = f"Request Data Reaches {hit.marker} Across {len(hit.chain) - 1} Call(s)"
    return description, title


def language_of(symbol):
    return LANGUAGE_TAGS.get(Path(str(symbol.get("file", ""))).suffix.lower(), "GEN")


def payload_summary(payload_path):
    payload = json.loads(Path(payload_path).read_text(encoding="utf-8"))
    return f"{len(payload.get('symbols', []))} symbols, {payload.get('edges')} edges"


def keep(findings, resolved):
    return [f for f in findings if Path(f.file_path).resolve() == resolved]


def load(payload):
    joined = payload if all(value is payload[0] for value in payload) else _merge(*payload)
    return joined
