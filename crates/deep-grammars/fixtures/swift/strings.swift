let name = "world"
let greeting = "hello, \(name)"
let math = "sum: \(1 + 2), nested: \("inner")"
let multiline = """
    first line
    second line
    """
let raw = #"a raw \n string with \(not interpolated)"#
let escaped = "quote \" and backslash \\"
let combined = greeting + " " + name
print(combined)
print(multiline)
print(raw)
print(escaped)
print(math)
