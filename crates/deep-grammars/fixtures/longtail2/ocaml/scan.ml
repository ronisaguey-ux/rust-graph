module type SCANNER = sig
  type report
  val run : string -> report
end

module Scanner : SCANNER = struct
  type report = { matches : int; ok : bool }

  let rules = [| "password"; "secret"; "token" |]

  let count text rule =
    let hits = ref 0 in
    let index = ref 0 in
    while !index < String.length text do
      if String.sub text (!index) (String.length rule) = rule then incr hits;
      incr index
    done;
    !hits

  let run text =
    let total = ref 0 in
    Array.iter (fun rule -> total := !total + count text rule) rules;
    { matches = !total; ok = !total = 0 }
end

class counter initial =
  object (self)
    val mutable value = initial
    method bump step = value <- value + step
    method reset = value <- 0
    method get = value
  end
