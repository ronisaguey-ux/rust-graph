import std/strutils

type
  Good = object
    value: int

proc compute(a: int, b: int): int =
  result = a + b

proc broken(a: int): int =
  if a > 0:
    result = a
# the string below never closes and the indentation after it does not line up
  result = "unterminated
