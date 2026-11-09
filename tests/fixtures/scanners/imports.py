"""Import shapes this package writes, beyond the parenthesised list.

Each clause is a different `python_import` branch: a relative module, a
wildcard, a multi-clause `import ... as`, a dotted relative module and a
comma-separated `from` list. They are pinned together so one regression in the
mapping surfaces as one failing fixture rather than a silent count.
"""

from . import sibling
from ..package.module import Name, other as renamed
from .local import (
    alpha,
    beta as gamma,
)
from felicity.core import *
import os, sys as system, pathlib
import felicity.core.engine


def use(value):
    return sibling(value), Name(value), alpha(value), gamma(value), system, os
