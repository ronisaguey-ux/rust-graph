# Collections: arrays, hashes, ranges, indexing and blocks.
numbers = [1, 2, 3, 4, 5]

squares = numbers.map { |n| n * n }

odds = numbers.select do |n|
  n % 2 == 1
end

total = numbers.reduce(0) { |sum, n| sum + n }

config = { host: "localhost", port: 8080, "debug" => true }

window = numbers[1..3]
first = numbers[0]
label = config[:host]

puts(squares)
puts(odds)
puts(total)
puts(window)
puts(first)
puts(label)
