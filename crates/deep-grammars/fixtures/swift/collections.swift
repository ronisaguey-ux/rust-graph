var names = ["ada", "grace", "linus"]
names.append("alan")

var scores: [String: Int] = ["ada": 95, "grace": 88]
let ada = scores["ada"]
let best = scores["missing"] ?? 0

var matrix: [[Int]] = [[1, 2], [3, 4]]
let cell = matrix[0][1]

let optionalName: String? = nil
let length = optionalName?.count ?? 0
let forced: String = optionalName!

print(names)
print(ada)
print(best)
print(cell)
print(length)
print(forced)
