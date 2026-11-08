module Types

type Point = { X: int; Y: int }

type Shape =
    | Circle of float
    | Square of float

type Name = string

let area shape =
    match shape with
    | Circle radius -> radius * radius
    | Square side -> side * side
