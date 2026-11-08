import std/strutils
import std/[tables, sequtils]
from os import getEnv, paramCount
include ./helpers

type
  Mode* = enum
    Debug, Release

  Server* = ref object of RootObj
    host*: string
    port*: int
    verbose*: bool

  Port = distinct int

const
  DefaultPort* = 8080
  Banner = "listening"

var
  counter*: int = 0

proc parsePort(text: string): int =
  result = 0
  for c in text:
    if c < '0' or c > '9':
      raise newException(ValueError, "bad digit")
    result = result * 10 + (ord(c) - ord('0'))

func twice(n: int): int = n * 2

proc describe(server: Server): string =
  result = server.host & ":" & $server.port

proc run(server: Server) =
  inc counter
  let port = parsePort(getEnv("PORT", "8080"))
  if port > 1024 or server.verbose:
    echo Banner, " ", describe(server)
  else:
    discard twice(port)

when isMainModule:
  var server = Server(host: "localhost", port: DefaultPort, verbose: false)
  run(server)
