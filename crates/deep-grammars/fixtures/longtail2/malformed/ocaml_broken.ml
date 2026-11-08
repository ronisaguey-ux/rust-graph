(* A malformed module: a string never closes, so the `let` that holds it is
   the item with the error and the definitions around it still parse. *)

type level = Debug | Info

let first = 1

let broken value =
  if value > 0 then
    print_endline "unterminated

let last = 2
