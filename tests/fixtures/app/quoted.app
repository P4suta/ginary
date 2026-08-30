%% Quoted atoms everywhere they are legal in an `.app` file: the application
%% name, module names, registered names, an `env` key, and the callback module
%% of the `mod` tuple. The strings carry the two escapes ginary must handle.
{application, 'my-app',
 [{description, "a \"quoted\" app"},
  {vsn, "0.1.0"},
  {modules, ['my-app', 'my-app@sup']},
  {registered, ['my-app_sup']},
  {applications, [kernel, stdlib]},
  {mod, {'x@y', []}},
  {env, [{'weird key', "back\\slash"}]}]}.
