{application, included,
 [{description, "included applications"},
  {vsn, "2.5.0"},
  {modules, [included]},
  {registered, [included_sup]},
  {applications, [kernel, stdlib, crypto]},
  {included_applications, [sasl, runtime_tools]}]}.
