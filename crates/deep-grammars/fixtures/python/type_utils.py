from typing import Callable, Iterable, Protocol, TypeVar

T = TypeVar("T")
K = TypeVar("K")
V = TypeVar("V")
Number = int | float


class Comparable(Protocol):
    def __lt__(self, other: object) -> bool: ...


def unique(items: Iterable[T], key: Callable[[T], object] | None = None) -> list[T]:
    seen: set[object] = set()
    result: list[T] = []
    for item in items:
        marker = key(item) if key is not None else item
        if marker in seen:
            continue
        seen.add(marker)
        result.append(item)
    return result


def group_by(items: Iterable[T], key: Callable[[T], K]) -> dict[K, list[T]]:
    grouped: dict[K, list[T]] = {}
    for item in items:
        grouped.setdefault(key(item), []).append(item)
    return grouped


def smallest(items: Iterable[Comparable]) -> Comparable:
    iterator = iter(items)
    best = next(iterator)
    for item in iterator:
        if item < best:
            best = item
    return best


def index_of(items: Iterable[T], predicate: Callable[[T], bool]) -> int:
    for position, item in enumerate(items):
        if predicate(item):
            return position
    return -1


type Pair[K, V] = tuple[K, V]
type MappingView[K, V] = Iterable[Pair[K, V]]
