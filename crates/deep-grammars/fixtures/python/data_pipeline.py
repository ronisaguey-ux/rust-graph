"""Load, clean and aggregate a CSV of orders."""

from __future__ import annotations

import csv
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable


@dataclass(frozen=True)
class Order:
    """One row of the input file."""

    sku: str
    quantity: int
    unit_price: float
    tags: list[str] = field(default_factory=list)

    @property
    def total(self) -> float:
        return self.quantity * self.unit_price


def load(path: Path) -> Iterable[Order]:
    with path.open(newline="") as handle:
        for row in csv.DictReader(handle):
            tags = [tag.strip() for tag in row["tags"].split("|") if tag]
            yield Order(row["sku"], int(row["quantity"]), float(row["unit_price"]), tags)


def by_sku(orders: Iterable[Order]) -> dict[str, float]:
    totals: dict[str, float] = {}
    for order in orders:
        totals[order.sku] = totals.get(order.sku, 0.0) + order.total
    return totals


if __name__ == "__main__":
    import sys

    summary = by_sku(load(Path(sys.argv[1])))
    for sku, amount in sorted(summary.items(), key=lambda pair: pair[1], reverse=True):
        print(f"{sku:>12} {amount:10.2f}")
