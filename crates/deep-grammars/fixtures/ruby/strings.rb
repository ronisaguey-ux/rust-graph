# Strings: quotes, interpolation, percent literals, regex and heredocs.
name = "world"
greeting = "hello #{name}"
single = 'no #{interpolation} here'
words = %w[alpha beta gamma]
raw = %q(a raw (quoted) string)
pattern = /ab+c/i
symbols = [:alpha, :beta, :gamma]

sql = <<~SQL
  SELECT * FROM users
  WHERE name = 'ada'
SQL

notice = <<-END
  This heredoc terminator may be indented.
END

message = greeting + " " + name
matcher = "abc" =~ pattern
puts(message)
puts(single)
puts(words)
puts(raw)
puts(symbols)
puts(sql)
puts(notice)
puts(matcher)
