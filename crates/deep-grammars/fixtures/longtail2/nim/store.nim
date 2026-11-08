import std/tables
import std/options

type
  Entry* = tuple
    key: string
    value: int

  Store* = ref object
    items*: Table[string, int]

template withStore(store: untyped, body: untyped): untyped =
  body

proc newStore(): Store =
  Store(items: initTable[string, int]())

proc put(store: Store, key: string, value: int) {.raises: [].} =
  store.items[key] = value

proc get(store: Store, key: string): Option[int] =
  if store.items.hasKey(key):
    some(store.items[key])
  else:
    none(int)

proc size(store: Store): int =
  store.items.len

block sized:
  var store = newStore()
  put(store, "alpha", 1)
  put(store, "beta", 2)
  let total = size(store)
  while total > 0:
    echo "entries: ", total
    break

try:
  let store = newStore()
  echo size(store)
except ValueError as error:
  echo "error: ", error.msg
finally:
  echo "done"
