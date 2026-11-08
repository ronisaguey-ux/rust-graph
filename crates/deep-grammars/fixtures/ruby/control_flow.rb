# Control flow: if/elsif/else, unless, loops, ranges and case.
def classify(value)
  if value < 0
    "negative"
  elsif value == 0
    "zero"
  else
    "positive"
  end
end

def warn_unless_ready(ready)
  unless ready
    puts("not ready")
  end
end

count = 0
while count < 3
  count += 1
end

until count == 0
  count -= 1
end

for i in 0...3
  puts(i)
end

case count
when 0 then puts("empty")
when 1, 2 then puts("small")
else puts("large")
end

puts(classify(-2))
warn_unless_ready(count > 5)
