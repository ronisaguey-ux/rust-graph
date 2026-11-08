import std/strutils
import std/tables

type
  Kind = enum
    Password, Secret, Token

  Rule = object
    pattern: string
    kind: Kind
    enabled: bool

  Report* = object
    hits*: int
    paths*: seq[string]

const Patterns = ["password", "secret", "token"]

proc makeRule(pattern: string, kind: Kind): Rule =
  Rule(pattern: pattern, kind: kind, enabled: true)

proc match(rule: Rule, text: string): bool =
  rule.enabled and text.contains(rule.pattern)

proc scan(rules: seq[Rule], text: string): Report =
  var hits = 0
  var paths: seq[string] = @[]
  for rule in rules:
    case rule.kind
    of Password:
      if match(rule, text):
        inc hits
        paths.add(rule.pattern)
    of Secret, Token:
      hits = hits + 2
    else:
      discard
  Report(hits: hits, paths: paths)

iterator eachRule(report: Report): string =
  for path in report.paths:
    yield path
