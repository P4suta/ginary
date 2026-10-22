# SPDX-License-Identifier: MIT OR Apache-2.0
def required($full):
  . as $needs |
  ($full | type) == "boolean" and
  .changes.result == "success" and
  (.changes.outputs.runtime == "true" or .changes.outputs.runtime == "false") and
  (["lint", "msrv"] | all(.[];
    $needs[.].result == (if $full or $needs.changes.outputs.runtime == "true" then "success" else "skipped" end))) and
  (["test", "smoke", "cross-build", "smoke-matrix", "macos", "windows", "coverage", "formal", "mutants", "fuzz"] |
    all(.[]; $needs[.].result == (if $full then "success" else "skipped" end)));
