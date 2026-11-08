module Pipeline

let numbers = [1; 2; 3]

let doubled = numbers |> List.map (fun x -> x * 2)

let total = List.fold (fun acc x -> acc + x) 0 doubled

let greet name = "hello " + name

let banner = greet "world" |> String.toUpper
