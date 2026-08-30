%% One of every term kind the parser must accept, nested inside `env`.
{application, nested,
 [{description, "nested terms"},
  {vsn, "0.3.1"},
  {modules, [nested, nested_ffi]},
  {registered, []},
  {applications, [kernel, stdlib]},
  {env, [{bin, <<"payload">>},
         {empty_bin, <<>>},
         {chars, [$a, $\n, $\\, $z]},
         {floats, [1.5, -2.0e3, 0.25]},
         {ints, [-1, 0, 42]},
         {tree, [[a, b], {c, [d, {e, []}]}, []]},
         {unit, {}}]}]}.
