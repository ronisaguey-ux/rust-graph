"""Fields that hold their own braces, strings and calls."""

mapping = f"{ {'a': 1}['a'] }"
nested = f"{f'{len(inner)}'}"
spaced = f"{ left + right }"
called = f"{', '.join(map(str, items))}"
debug = f"{total=}"
spec = f"{value:{width}.{precision}f}"
triple = f"""
total {sum(values)}
"""
