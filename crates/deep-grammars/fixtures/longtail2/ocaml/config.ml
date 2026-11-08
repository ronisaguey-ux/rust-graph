(* Service configuration: the record and variant types, and the accessors a
   scanner sees at the top of a module. *)

type level = Debug | Info | Warning | Error

type settings = {
  name : string;
  port : int;
  verbose : bool;
  level : level;
}

let default_level = Info

let make name port =
  { name = name; port = port; verbose = false; level = default_level }

let is_verbose settings = settings.verbose

let describe settings =
  match settings.level with
  | Debug -> "debug"
  | Info -> "info"
  | Warning -> "warning"
  | Error -> "error"
