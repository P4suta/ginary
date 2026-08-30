% one percent starts a comment
%% and so do two
{application, comments,        % a comment after a term
 [{description, "100% not a comment"},
  {vsn, "1.0.0"},              % a comment after a property
  {modules, []},
  {registered, []},
  {applications, [kernel,      % a comment inside a list
                  stdlib]}]}.
% a comment after the final full stop
