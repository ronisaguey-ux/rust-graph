-module(rules).
-export([applies/2, evaluate/2]).

-define(MAX_SIZE, 1000000).
-define(MAX_DEPTH, 8).

applies(size, Path) ->
    filelib:file_size(Path) =< ?MAX_SIZE;
applies(depth, Path) ->
    length(filename:split(Path)) =< ?MAX_DEPTH;
applies(_, _) ->
    true.

evaluate(Rules, Files) ->
    lists:filter(
      fun(File) ->
              lists:all(fun(Rule) -> applies(Rule, File) end, Rules)
      end, Files).
