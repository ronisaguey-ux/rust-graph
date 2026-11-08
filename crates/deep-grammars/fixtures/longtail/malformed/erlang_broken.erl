-module(broken).
-export([scan/1]).

scan(Root) ->
    case file:list_dir(Root) of
        {ok, Entries} -> Entries
    end
