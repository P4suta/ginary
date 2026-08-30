%% SPDX-License-Identifier: MIT OR Apache-2.0
%%
%% The whole observable behaviour of the `hello_ffi` fixture.
%%
%% Four things are proved by running it out of a staged root:
%%
%%   * `init:get_plain_arguments/0` returns exactly what came after `-extra`,
%%     which is the argument contract the launcher has to honour;
%%   * `code:priv_dir/1` resolves, which means the application's `priv` was
%%     staged beside its `ebin` and the code path found it;
%%   * `file:get_cwd/0` is the directory the process was started in, not
%%     wherever the runtime was unpacked;
%%   * `erlang:halt/1` propagates an exit code, and `erlang:error/1` reaches
%%     Gleam's generated `hello_ffi@@main`, which prints a runtime error and
%%     exits 1.

-module(hello_ffi_ffi).

-export([main/0]).

-spec main() -> no_return().
main() ->
    Args = init:get_plain_arguments(),
    io:format("args=~ts~n", [lists:join(" ", Args)]),
    {ok, Greeting} = file:read_file(filename:join(code:priv_dir(hello_ffi), "greeting.txt")),
    io:format("~ts", [Greeting]),
    {ok, Cwd} = file:get_cwd(),
    io:format("cwd=~ts~n", [Cwd]),
    case Args of
        ["--crash" | _] ->
            erlang:error(boom);
        [First | _] ->
            erlang:halt(exit_code(First));
        [] ->
            erlang:halt(0)
    end.

%% The first argument as an exit code, or 0 when it is not an integer.
-spec exit_code(string()) -> integer().
exit_code(Text) ->
    try
        list_to_integer(Text)
    catch
        error:badarg -> 0
    end.
