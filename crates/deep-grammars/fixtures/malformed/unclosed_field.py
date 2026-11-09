"""Replacement fields that cannot close, and two that do.

`{` at a line end, an empty field and a field cut off by the string's own
quote are errors; `f"{{"` is one escaped brace followed by the closing quote,
and `f"{'}'}"` holds a string literal, so both are valid Python and must stay
clean.
"""

a = f"{
b = f"{}"
c = f"{a"
d = f"{{"
e = f"{'}'}"
