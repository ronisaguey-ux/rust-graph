# Numbers, operators and precedence.
hex = 0xff
binary = 0b1010
big = 1_000_000
ratio = 3.14
exponent = 1.5e3
sum = hex + binary * 2
power = 2 ** 10
negated = -sum
shifted = hex << 2
masked = 0xff & 0x0f
comparison = sum > 10 && sum < 100
either = comparison || false
inverted = not either
choice = comparison ? "yes" : "no"
puts(big)
puts(ratio)
puts(exponent)
puts(power)
puts(negated)
puts(shifted)
puts(masked)
puts(inverted)
puts(choice)
