-module(report).
-export([render/1, emit/2]).

render(#{count := Count, bytes := Bytes}) ->
    Rows = [row("files", Count), row("bytes", Bytes)],
    lists:flatten(["## Scanner summary", "\n", Rows, "\n"]).

row(Label, Value) ->
    io_lib:format("| ~-16s | ~10w |", [Label, Value]).

emit(Summary, Path) ->
    Text = render(Summary),
    ok = file:write_file(Path, Text),
    {ok, byte_size(Text)}.
