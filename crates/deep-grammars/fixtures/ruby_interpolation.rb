# frozen_string_literal: true
# Interpolation: a `#{...}` field is code, not text.
name = "world"
greeting = "hello #{name}"
total = "sum: #{1 + 2}"
nested = "a #{ "b #{name} c" } d"
hash = "#{ {a: 1}[:a] }"
call = "result: #{fetch(name)}"
chain = "list: #{ [1, 2].map { |x| x * 2 }.join(", ") }"
methodarg = "size #{count(names)} and #{pick(1, 2)}"
puts(greeting, total, nested, hash, call, chain, methodarg)
