# SPDX-License-Identifier: MIT OR Apache-2.0
include "required";

def jobs: ["changes", "lint", "msrv", "test", "smoke", "cross-build", "smoke-matrix", "macos", "windows", "coverage", "formal", "mutants", "fuzz"];
def result: "success", "failure", "cancelled", "skipped";

def baseline($full; $runtime):
  {
    changes: {result: "success", outputs: {runtime: $runtime}},
    lint: {result: (if $full or $runtime == "true" then "success" else "skipped" end)},
    msrv: {result: (if $full or $runtime == "true" then "success" else "skipped" end)}
  } +
  (reduce jobs[3:][] as $job ({};
    .[$job] = {result: (if $full then "success" else "skipped" end)}));

def cases:
  (false, true) as $full | ("false", "true") as $runtime |
  baseline($full; $runtime) as $base |
  jobs[] as $job | result as $result |
  {
    full: $full,
    needs: ($base | .[$job].result = $result),
    expected: ($result == $base[$job].result)
  };

def actual: . as $case | .needs | required($case.full);

def report(verdict):
  reduce cases as $case ({total: 0, matched: 0, admitted: 0};
    ($case | verdict) as $actual |
    .total += 1 |
    .matched += (if $actual == $case.expected then 1 else 0 end) |
    .admitted += (if $actual == true then 1 else 0 end));

def invalid:
  null, 0, "invalid", [],
  (.full = null), (.full = "true"), (.full = 1),
  (del(.needs.changes.outputs)),
  (jobs[] as $job | del(.needs[$job]), (.needs[$job] = null), (.needs[$job].result = "unknown")),
  ([null, true, false, 0, 1, "", "TRUE", [], {}][] as $runtime | .needs.changes.outputs.runtime = $runtime);

report(actual) as $production |
report(true) as $approve_all |
report(false) as $reject_all |
report(.needs | all(.[]; .result == "success" or .result == "skipped")) as $allow_skips |
{full: true, needs: baseline(true; "true")} |
[invalid | (try actual catch false) == false] as $invalid |
if $production == {total: 208, matched: 208, admitted: 52} and
   $approve_all.matched < $production.total and
   $reject_all.matched < $production.total and
   $allow_skips.matched < $production.total and
   ($invalid | length) > 0 and ($invalid | all)
then {cases: $production.total, invalid: ($invalid | length), rejecting_controls: 3}
else error("required-job contract failed")
end
