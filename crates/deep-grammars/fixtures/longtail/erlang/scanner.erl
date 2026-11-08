-module(scanner).
-export([scan/1, walk/2]).
-export_type([summary/0]).

-type summary() :: #{count := non_neg_integer(), bytes := non_neg_integer()}.

-spec walk(string(), non_neg_integer()) -> [string()].
walk(Root, Depth) ->
    case Depth > 8 of
        true ->
            [];
        false ->
            case file:list_dir(Root) of
                {ok, Entries} ->
                    lists:flatmap(fun(Entry) -> visit(Root, Entry, Depth) end, Entries);
                {error, _} ->
                    []
            end
    end.

visit(Root, Entry, Depth) ->
    Path = filename:join(Root, Entry),
    case filelib:is_dir(Path) of
        true -> walk(Path, Depth + 1);
        false -> [Path]
    end.

-spec scan(string()) -> summary().
scan(Root) ->
    Files = walk(Root, 0),
    Sizes = [filelib:file_size(F) || F <- Files],
    #{count => length(Files), bytes => lists:sum(Sizes)}.
