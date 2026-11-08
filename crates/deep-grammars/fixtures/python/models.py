from __future__ import annotations

from datetime import datetime
from decimal import Decimal
from enum import Enum, auto


class Status(Enum):
    DRAFT = auto()
    OPEN = auto()
    CLOSED = auto()


class Money:
    __slots__ = ("amount", "currency")

    def __init__(self, amount: Decimal, currency: str = "EUR") -> None:
        self.amount = amount
        self.currency = currency

    def __add__(self, other: Money) -> Money:
        if other.currency != self.currency:
            raise ValueError(f"cannot add {other.currency} to {self.currency}")
        return Money(self.amount + other.amount, self.currency)

    def __repr__(self) -> str:
        return f"Money({self.amount!r}, {self.currency!r})"


class Invoice:
    def __init__(self, number: str, issued: datetime) -> None:
        self.number = number
        self.issued = issued
        self.lines: list[tuple[str, Money]] = []
        self.status = Status.DRAFT

    def add_line(self, description: str, price: Money) -> None:
        self.lines.append((description, price))

    @property
    def subtotal(self) -> Money:
        total = Money(Decimal("0"))
        for _, price in self.lines:
            total = total + price
        return total

    def close(self) -> None:
        if not self.lines:
            raise ValueError("an invoice without lines cannot be closed")
        self.status = Status.CLOSED
