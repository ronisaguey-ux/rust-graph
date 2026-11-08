module Program

open System

let square x = x * x

let describe value = if value > 0 then "positive" else "negative"

let result = describe (square 4)

printfn "%s" result
