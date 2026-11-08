open Printf

module Util = struct
  let clamp value low high =
    if value < low then low
    else if value > high then high
    else value

  let rec repeat value count =
    if count <= 0 then []
    else value :: repeat value (count - 1)
end

external send_all : File_descr.t -> string -> int = "caml_send_all"

let handle config request =
  let path = request.path in
  let body =
    match path with
    | "/health" -> "ok"
    | _ -> sprintf "%s:%d" config.name config.port
  in
  (path, body)

let serve config =
  try
    let fd = connect config.port in
    handle config fd
  with
  | Failure message -> printf "failed: %s\n" message; None
  | _ -> None
