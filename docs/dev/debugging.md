<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Debugging

Every switch below is listed with its status. **Implemented** means it works in the current
revision. **Planned** means the design is settled but no code exists; setting the variable does
nothing today.

## Environment variables

| variable | status | effect |
|---|---|---|
| `GINARY_CACHE_DIR` | implemented | Overrides the cache root outright, and is used verbatim — relative paths included. The escape hatch for a read-only or `noexec` home directory. `ginary cache dir` prints what it resolves to and why. |
| `GINARY_DEBUG=1` | implemented | Human-readable progress on stderr, prefixed `ginary[debug]: `, one line per phase with its facts and its elapsed time: `start`, `read_manifest`, `cache_sweep`, `cache_tmp`, `extract`, `chmod`, `sync`, `rename` or `cache_hit`, `preflight_retry`, `exec`. |
| `GINARY_TRACE=<file>` | implemented | Appends schema-2 JSON Lines with `run_id`, `sequence`, `event`, `t_us`, `phase`, `kv`, optional `operation_id` and optional `elapsed_us`. Default capture redacts arguments, environment values, crash slogans, credentials and URL query/fragment values. A nonempty destination must begin with a complete ginary trace record (at most 1 MiB); unrelated existing files are preserved. Open/write failures update `Diag::health()` and produce a warning while the application continues. |
| `GINARY_TRACE_SENSITIVE=1` | implemented | Explicitly retains arguments, environment values and other sensitive diagnostic values for local reproduction. Applies to trace and debug sinks. Review the resulting files before sharing them; `ginary diagnose` produces a summary without copying those values. |
| `GINARY_SUPERVISE=1` | implemented | Spawns the runtime and waits instead of calling `execve`, which is the code path Windows will use anyway. The exit code is mirrored; a child killed by a signal exits `128 + signo`. Records the exit status, the signal and the elapsed time, and if an `erl_crash.dump` appeared during the run, prints its `Slogan` line. |
| `GINARY_CMD=<command>` | implemented | Artifact-side maintenance, kept out of `argv` so the packaged application still owns its own flags, and one of five values. `directory` prints the cache entry the artifact would use and creates nothing; `extract-only` extracts and prints the entry without launching; `inspect` prints the manifest, the payload geometry and the digest as one JSON object; `selftest` extracts, preflights and starts the runtime with `-eval erlang:halt(0)` and no `-extra`, printing `extract:`, `preflight:` and `run:` with `PASS` or `FAIL` and exiting 0 or 1; `uninstall` removes every cache entry of this application that nobody holds, prints what it removed and what it kept and why, and exits 0 even when it kept something. Any other value is a usage error and exits 2. |
| `GINARY_ERL_FLAGS` | implemented | Extra emulator flags for one run, split on ASCII whitespace and placed after the manifest's own flags and before `-eval`. |
| `GINARY_FAULT=<point>[:<action>]` | implemented (test builds) | Fault injection, compiled in only under `cfg(feature = "fault-injection")` and therefore absent from release builds, which never read the variable at all. Points: `after-extract:pause` (sleep with the temporary tree on disk), `rename:eexist` (extract, then lose the rename race), `unpack:corrupt` (the payload changes under the reader), `before-lock` (remove the entry between preflight and locking), `launcher:panic` (panic on the launcher path), `pack:fail` (stop between the stub and payload), `output-write:fail` (stop after a partial temporary artifact/document write), `output-persist:fail` (stop immediately before replacing the final name), `artifact-sign:fail` (interrupt the macOS signing path after a partial temporary write), `artifact-sign:corrupt` (alter a finished signature before verification). The output points also accept `fail-document` to let artifact publication finish before failing its manifest or SBOM. |
| `GINARY_PRUNE_DAYS=<n>` | implemented | How many days an unused cache entry of the running application may live before the next launch prunes it. Defaults to 14; `0` turns pruning off for that run. A value that is not a count of days falls back to the default rather than failing a launch: a misspelt housekeeping preference must not stop an application from starting. |
| `GINARY_OFFLINE=1` | implemented | Forbids every fetch. `download::Net` refuses before a socket is opened and the error names the URL and the file it was for, so a build that would have gone to the network says what it wanted rather than reaching for it. There is no `--offline` flag: `download::Net` takes the switch as a parameter and every call site passes `false`, so the variable is the only way on. The parameter is one-way by construction — a build asked to stay offline is not put back on the network by an environment — and a flag that fed it would inherit that.  |
| `GINARY_GITHUB_BASE_URL=<base>` | implemented | Replaces `https://api.github.com` as the base of every GitHub API read, as a prefix, so one value redirects the whole host at a mirror or at a test server. |
| `GINARY_GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN` | implemented | The token the GitHub release API is read with, tried in that order, first non-empty wins; no scopes are needed for a public repository. An anonymous read is limited to 60 an hour **by source address**, and over it GitHub answers `403` — which `download` reads as a rate limit rather than reporting as a bare status, because a 403 that waiting fixes and a 403 that it does not are different answers. Sent to the API base only: never on the asset download, never to a host that is not the API base, and never carried on to a redirect (`RedirectAuthHeaders::Never`). |
| `GINARY_REQUIRE_TOOLCHAIN=1` | implemented (convention) | Turns a skipped toolchain-gated test into a failure. See [testing.md](testing.md). |
| `GINARY_REQUIRE_STUBS=1` | implemented (convention) | Turns a *missing cross-built stub* into a failure rather than a skip. Deliberately not the same switch as `GINARY_REQUIRE_TOOLCHAIN`: that one is a claim about programs the machine installs, and a stub is the output of `mise run stubs:build`, which needs `cross`, a docker daemon and minutes per target. Only a job that obtains them — by cross-building them, or by downloading what the job that did uploaded — sets it. See [testing.md](testing.md). |

The launcher **removes** `ERL_LIBS`, `ERL_FLAGS`, `ERL_AFLAGS`, `ERL_ZFLAGS`, `ERL_ROOTDIR`,
`ERL_EPMD_PORT` and every variable whose name begins `ERL_OTP` and ends `_FLAGS` before starting
the runtime. If a packaged application behaves differently from a `gleam run`, that scrubbing is
the first thing to check. It **sets** `ROOTDIR`, `BINDIR`, `EMU=beam` and `PROGNAME`
unconditionally, and `HOME`, `ERL_CRASH_DUMP`, every pair of the manifest's `launch.env` and
`HEART_COMMAND` only when the caller has not: a `HOME` you exported is yours, and so is a
`LOG_LEVEL` the artifact would otherwise have defaulted. `launch.env` is applied *after* the
scrub, so a name in the scrub list is never reintroduced; the build refuses such a name anyway.

## The cache lock, and pruning

Every launch takes a shared `flock` on `<entry>/.lock` immediately before `execve`, with
`FD_CLOEXEC` cleared, so the lock is inherited by the runtime and released by the kernel when the
runtime exits. Pruning takes `flock(LOCK_EX | LOCK_NB)` on the same file and skips any entry it
cannot get. Neither side ever waits: the launcher's `LOCK_SH` is non-blocking too, retried for
half a second and then given up on, so a foreign `flock -x` on an entry cannot hang an
application — it only costs that run its lock. Immediately after locking, the launcher re-checks
that the entry is still there and extracts it again if a prune took it in the meantime. [ADR 0010](../adr/0010-cache-locking-and-pruning.md) explains why, and
`tests/launcher.rs::the_shared_lock_outlives_the_launcher_and_dies_with_the_runtime` proves the
`execve` half with util-linux `flock(1)` rather than with ginary's own code: it runs an artifact
whose runtime sleeps, asserts from outside that `flock -n -x <entry>/.lock` **fails** while the
child runs — nothing of ginary is alive at that moment — and **succeeds** once it exits.

You can run the same check by hand:

```console
$ ./my_gleam_app &                                        # or any long-running artifact
$ entry=$(GINARY_CMD=directory ./my_gleam_app)
$ flock -n -x "$entry/.lock" true && echo free || echo held
held
$ kill %1; sleep 1
$ flock -n -x "$entry/.lock" true && echo free || echo held
free
```

Two symptoms and what they mean:

- **A cache entry never goes away, however old.** Something holds its lock. Find it with
  `fuser "$entry/.lock"` or `lsof "$entry/.lock"` — a runtime that is still running, or a
  descriptor a supervisor inherited and never closed. `ginary cache prune --all` will not remove
  it either: `--all` ignores age but honors the lock. `ginary cache clean` and
  `GINARY_CMD=uninstall` honor the same lock and preserve live extraction residue. Their
  reports explain each retained path rather than forcing a running entry away.
- **A cache entry disappeared under a running application.** The lock could not be taken and the
  launch went ahead anyway, which is deliberate — a lock that cannot be taken is a pruning risk
  and not a reason to refuse to start. `GINARY_TRACE` records it as a `lock` phase with the
  error. `flock` is advisory and per-filesystem, so a cache on NFS without a lock daemon is the
  usual cause; `GINARY_CACHE_DIR` on local disk is the fix.
- **A start records a `lock_retry` phase.** A prune removed the entry between the preflight and
  the lock, and the launcher extracted it again rather than starting out of a tree that was being
  deleted. One retry is all there is; a second disappearance is [exit
  code](../../README.md#exit-codes) 124 naming the entry.
- **`ginary cache prune` says an entry is `unremovable`.** Nobody holds it and it is old enough
  to go, and the file system refused the rename that moves it aside: a read-only application
  directory, a full disk, a mount that has gone away. It is reported rather than dropped, so the
  `total:` line counts it.

A prune that runs writes a `prune` phase to the trace: `removed` and `kept` count the two
columns, and `removed_paths` and `kept_paths` name them, as JSON arrays of strings — a `kept`
entry carries its reason (`locked`, `fresh`, `unowned` or `unremovable`) in the same string. An entry that
vanished has to be explainable from a trace, and a count explains nothing. Nothing a prune does
reaches standard error: pruning is housekeeping, and housekeeping does not decide whether an
application starts.

`GINARY_CMD=uninstall` removes only what the cache wrote — `<key>` entries and the
`.<key>.tmp-<pid>-<id>` (and legacy PID-only temporary names), `.<key>.corrupt-<pid>` and
`.<key>.trash-<pid>` residue beside them. The temporary id has twelve random alphanumeric
characters, so simultaneous calls in one process own separate trees. Anything
else in the application directory is left where it is, `erl_crash.dump` included, which is why
that directory survives an uninstall when a dump is in it.

Complete entries must have a supported, valid `ginary.json` identifying the containing
application. A hexadecimal directory name or a file called `ginary.json` alone does not prove
ownership. Invalid or mismatched markers remain `unowned`. Temporary, corrupt and trash trees
belonging to live processes remain `active`; cleanup never treats a completed temporary
manifest as permission to delete an extraction that is still running. A dead temporary tree
may precede writing its manifest. These ownership checks apply to pruning as well as cleaning.

### On Windows: where the cache is, and what holds it

Windows uses different cache roots and locking mechanisms. The portable F1 fixtures exercise
native Windows launch, selftest locking, concurrent maintenance and fault recovery; Unix
`flock` inheritance remains a separate Unix test. See [F1-cache.md](log/F1-cache.md) for the
local evidence and the [Windows](../../README.md#windows) section for supported distributions.

**The cache root** is resolved by the same precedence with two different variables in the
middle:

| order | root | provenance `ginary cache dir` prints |
|---|---|---|
| 1 | `%GINARY_CACHE_DIR%`, verbatim | `GINARY_CACHE_DIR` |
| 2 | `%LOCALAPPDATA%\ginary` | `LOCALAPPDATA` |
| 3 | `%TEMP%\ginary-<user>`, then `%TMP%`, then `C:\Windows\Temp` | `TEMP fallback` |

An exported-but-empty variable counts as unset, exactly as it does on unix. `LOCALAPPDATA` is
the per-user, non-roaming directory, which is what a cache of extracted runtimes should be —
`APPDATA` roams, and a domain login would carry forty megabytes of BEAM across the network.
The `<user>` component of the fallback comes from `%USERNAME%`; a value that is not one path
component, or is not set at all, becomes `unknown`, because a cache in the wrong directory is a
much bigger problem than one nobody can tell apart. The fallback root is *created* but not
proved to be this user's the way the unix `${TMPDIR}/ginary-<uid>` is: `%TEMP%` is per-account
and carries the ACL that says so, and writing one by hand is Win32 security work D2 does not do.

Extraction paths go through the `\\?\` prefix (`\\?\UNC\server\share` for a UNC root), which
skips the `MAX_PATH` normalisation. A cache entry is
`%LOCALAPPDATA%\ginary\<app>\<key>\lib\<name>-<vsn>\ebin\<module>.beam`, which is a hundred
and fifty characters before the application is named, so a deep home directory is exactly the
shape that would otherwise fail in the middle of an extraction.

The prefix goes on once per *walk*, and there are two kinds. An extraction gets it at
`CacheDirs::extraction_dir`, and everything the extraction writes is joined onto that: the
temporary tree, every file the unpacker creates, the per-file flush that reopens each of them,
and both ends of the rename. A path joined onto a verbatim path is verbatim too, which is what
makes one call enough — prefixing only the unpacker's destination moved the limit one step later,
into the flush.

A **removal** gets it for itself, in `cache.rs`: `sweep`, `discard_incomplete`, `prune_app`,
`uninstall`, `prune` and `clean` each put the directory they were given into the verbatim
spelling before listing it, rather than trusting the caller to have done it. Prefixing an
already-prefixed path is a no-op, so a caller holding either spelling reaches the same tree — and
a removal that walked the ordinary one would find a past-`MAX_PATH` entry, take its lock and then
fail the `rename` aside, reporting `unremovable` forever with no error anywhere.

`ensure_extracted` **answers with that same spelling**, and the rule the two helpers in
`src/winpath.rs` state is one sentence: *ginary opens the verbatim spelling and hands `erl.exe`
the ordinary one.* So the cache-hit check, the `<entry>\.lock` open, the manifest probe and
`launch::preflight` all open the directory the extraction created, and `GINARY_TRACE` shows
verbatim paths for `cache_tmp`, `cache_hit` and `rename` alike.

The prefix comes off in exactly two places, both with `winpath::plain_path`. The first is
`launch::plan`, which is where a cache path stops being ginary's business: `ROOTDIR`, `BINDIR`,
`HOME` and every path in the argument vector are put back into the ordinary spelling, because
`erl.exe` takes those apart and reassembles them rather than merely opening them. The one path
the plan keeps verbatim is the program the launcher spawns itself. The `launch` trace record
therefore shows a verbatim `program` beside an ordinary `ROOTDIR`, and that is not a bug.

The second is the removal reports. A `PruneReport`, a `CleanReport` and the path inside a cache
error name a path *to a person* rather than open one, so what `ginary cache prune`,
`ginary cache clean` and `GINARY_CMD=uninstall` print is the spelling their caller asked about,
not the one the walk used.

Real OTP startup from a cache entry beyond `MAX_PATH` is not yet qualified. The extraction,
cache lookup, lock and preflight use verbatim paths, while the runtime receives ordinary path
arguments. Synthetic long-path tests exercise ginary's operations; they do not establish which
OTP versions can start from that directory. If a real launch fails only under a deep cache root,
try a shorter `GINARY_CACHE_DIR` and retain the trace and runtime version.

**The lock** is not an `flock` — Windows has none. `<entry>\.lock` is opened with a *share mode*
instead, and the two locks become two share modes:

| lock | access asked for | `dwShareMode` |
|---|---|---|
| a running application's | read | `FILE_SHARE_READ` |
| a prune's | read and write | `FILE_SHARE_DELETE` |

Two launchers of one entry both succeed — each asks for read access and each permits it — and a
prune is refused for as long as either holds it. The prune's `FILE_SHARE_DELETE` shares no
reading and no writing, so that answer is unchanged; what it permits is the removal that
follows, which deletes `<entry>\.lock` along with the tree it is in.

What it does **not** permit is renaming the entry directory while the lock is still open inside
it. `FILE_SHARE_DELETE` speaks for the file it is on, not for an ancestor directory, and the
first Windows runner reported every complete entry `unremovable` because of it. So the two
happen in that order rather than at once: the lock proves nobody is using the entry, it is
released, and then the rename makes the claim. That leaves a window between "nobody holds this"
and "it is gone", and it is the price of being able to prune on Windows at all — an entry
another process grabs in that window is one whose rename fails, and a failed rename is reported
`unremovable` rather than forced. On unix nothing is given up: `rename(2)` asks nothing about
open descriptors, so the lock is held across the rename and no window opens.
`ginary::platform::rename_refuses_open_children` is the rule that separates the two, and
`docs/dev/log/E8.md` records the run that found it.

Three differences from the unix side are worth knowing when a Windows entry will not go away:

- the lock is **mandatory**, not advisory, so any program with `<entry>\.lock` open for writing
  blocks a prune, including one that has never heard of ginary;
- it belongs to the **handle**, and there is no `execve` for it to survive, so the launcher
  stays alive as the runtime's parent and holds it itself. `Get-Process` shows two processes,
  the artifact and `erl.exe` under it;
- a launcher that is killed takes the runtime with it, because the child is in a job object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. That is the *replacement* for the inherited descriptor:
  without it a killed launcher would leave a `beam.smp` holding an entry nothing releases.

Mode bits are a no-op throughout: `chmod` records `files=0` in the trace, nothing under the
bindir is made executable because there is no bit to set, and the `mode` column of
`ginary.index.json` is informational on a Windows artifact — it holds what the archive header
said, and nothing reads it back to enforce anything.

## Diagnosing the environment today

```console
$ ginary doctor
host target: linux-x86_64-gnu
rustc/cargo: not required (neither ginary nor its artifacts need a Rust toolchain)
cache dir: /home/user/.cache/ginary (from HOME)
cache writable: yes
cache executable: yes
gleam: 1.18.1 (/usr/local/bin/gleam)
erl: OTP 29, erts 17.0.5 (/usr/local/bin/erl)
strip: 2.42 (/usr/bin/strip)
docker: not found
otp: 29.0.5 (release 29, erts 17.0.5)
otp root: /opt/otp/lib/erlang
crypto: /opt/otp/lib/erlang/lib/crypto-5.9.2/priv/lib/crypto.so
crypto needs: libc.so.6
crypto note: nothing beyond a C runtime, so this OTP's OpenSSL is linked in statically; ...

project: my_gleam_app 1.0.0 (/home/user/src/my_gleam_app)
shipment: /home/user/src/my_gleam_app/build/erlang-shipment (412 seconds old)
[tools.ginary]: read
```

Three of those lines are the ones a failing machine is diagnosed from.

- **`cache writable` and `cache executable`** are a real probe, not a permission check: `doctor`
  exclusively creates a randomized file in the resolved cache directory, chmods it 0755 on
  Unix and tries to spawn it, then removes its owned file. Preexisting files and links are
  preserved. Cleanup refusal retains the probe path and error in `cache detail`; the capability
  answer remains visible. `access(2)` reports the mode bits and says nothing about the mount, and a cache on
  a `noexec` filesystem is the failure users actually hit — it is exit code 125 at run time. A
  failure prints what the operating system said, verbatim, and the `GINARY_CACHE_DIR` hint.
- **`crypto needs`** is the portability floor of every artifact built on this machine. An OTP
  built against a *static* OpenSSL leaves a `crypto.so` that needs nothing but a C runtime, and
  that is what lets an artifact start on a machine with no `libssl` of its own. One that needs
  `libcrypto.so.3` produces artifacts that will not start without it.
- **The project block** appears only when `doctor` is run inside a Gleam project. It reports the
  name and version, the exported shipment and its age, whether `[tools.ginary]` parses — the
  parser's own message, verbatim, because serde names the key and a paraphrase would lose it —
  and a table of every ELF under the shipment's `priv` directories, flagged when its machine is
  not this host's.

`ginary doctor --json` prints the same information as an object with `format_version`,
`host_target`, `rustc_required`, `cache_dir`, `cache_dir_source`, `cache_dir_error`,
`cache_probe`, `otp` (with its `crypto`), `project` and a `tools` array of
`{name, found, version, path}`. The version-2 report also supplies `tool_probes` with classified
outcomes, bounded output and remedies, plus `findings` for incomplete checks. Each tool probe
has a ten-second execution budget, followed by bounded child cleanup and pipe collection.
Timeout and incomplete capture are distinct from an unrecognized version response.
The cache executable probe has the same execution budget. A failed temporary-file cleanup
also produces the `cache_probe_cleanup_failed` finding when the executable itself ran.

`doctor` never fails. A missing tool is information, not an error, and the exit status stays 0.

## Looking inside the binaries an artifact is made of

Two commands answer the two questions a size or portability surprise turns into. Both are
read-only, both take any number of paths, and both take `--json`.

### `ginary beam chunks <file>...`

What a compiled module is made of, and whether it still carries debug information.

```console
$ ginary beam chunks tests/fixtures/beam/gleam@list.beam
tests/fixtures/beam/gleam@list.beam
id    offset  len
AtU8  20      1763
Code  1792    9434
StrT  11236   0
ImpT  11244   316
ExpT  11568   808
FunT  12384   172
LitT  12564   273
Meta  12848   45
LocT  12904   604
Attr  13516   39
CInf  13564   168
Dbgi  13740   27895
Docs  41644   7250
Line  48904   689
Type  49604   76
debug_info: yes
```

That module is checked in, so the transcript above is reproducible: `Dbgi` and `Docs` are 35 kB
of a 49 kB file, which is what stripping is for.

This is the window onto stripping. A module that is still large after a build shows here exactly
which chunk it is large because of, and `debug_info: yes` on a *staged* module means the beam
half of stripping did not run, or ran and did nothing — which `ginary stage` would also have
refused, since it re-reads every module afterwards.

A stripped module is a gzip member rather than a bare `FOR1` form, because
`beam_lib` writes what it rewrote through `zlib:gzip/1`. The command unwraps it
the way the code server does, so the table reads the same either way; the offsets are then
offsets into the uncompressed form.

`--json` prints `{format_version, files: [{path, chunks: [{id, offset, len}], debug_info}]}`, in
the order the paths were given. A file that is not a module is an error and exits 1; nothing
partial is printed.

### `ginary elf deps <file>...`

What a native binary needs from the machine that runs it.

```console
$ ginary elf deps ~/.local/share/mise/installs/erlang/29.0.5/erts-17.0.5/bin/beam.smp
.../erts-17.0.5/bin/beam.smp
  class     64
  machine   x86_64
  interp    /lib64/ld-linux-x86-64.so.2
  pie       yes
  stripped  no
  glibc_max 2.38
  needed    libtinfo.so.6, libstdc++.so.6, libm.so.6, libgcc_s.so.1, libc.so.6
```

`glibc_max` is the highest `GLIBC_x.y` in `.gnu.version_r`, compared numerically, and it is the
artifact's portability floor: a machine with an older glibc will not start the runtime, whatever
else is installed on it. `stripped` says whether the file still has a `.symtab`, which is the
first thing to check when a staged tree is smaller than expected — somebody else's build may have
stripped it already.

`--json` prints `{format_version, files: [{path, class, kind, machine, interp, needed,
glibc_max, is_pie, stripped}]}`. A file that is not an ELF is an error and exits 1. `kind` is
`e_type` — `executable`, `shared_object`, `relocatable`, `core` — and it is what decides which
arguments `strip` gets, so a position-independent program reads `shared_object` here and `pie
yes` in the table above: the header does not distinguish the two, and `interp` does not either.
glibc's own `libc.so.6` is a library that carries a program interpreter.

## Reading the size report

`ginary stage` prints the strip table and the size report under its own output:

```console
elf:   4 files, 56602456 -> 10722528 bytes, 45879928 saved
beams: 205 files, 10158920 -> 1889386 bytes, 8269534 saved
total: 209 files, 66761376 -> 12611914 bytes, 54149462 saved

category      files  before    after     saved
erts_binary   4      56602456  10722528  45879928
...
total         214    66775592  12626130  54149462

needs: libc.so.6 (GLIBC_2.38), libgcc_s.so.1, libm.so.6, libstdc++.so.6, libtinfo.so.6
```

A half of the strip table that did not run says so in place of its numbers — `nothing to strip`,
`skipped: <reason>`, `not asked for` — so a missing saving is never ambiguous. `--no-strip`,
`--strip-elf-only` and `--strip-beams-only` narrow it; ADR 0007 records why the default is both.

`--report json` prints the report alone, as one object, with nothing else on standard output, so
it can be piped:

```console
$ ginary stage ... --report json | jq '.needs_summary'
{
  "needed": ["libc.so.6", "libgcc_s.so.1", "libm.so.6", "libstdc++.so.6", "libtinfo.so.6"],
  "glibc_max": "2.38"
}
```

The two JSON shapes nest differently, and the path above is the one that works for `--report
json`: that object is `{format_version, strip, ...the report}` with the report's own members —
`categories`, `total_before`, `total_after`, `elf_deps`, `needs_summary`, `warnings` — at the top
level. Under `--json` the same report is one member of a larger object, so there the path is
`.report.needs_summary`.

`--report json` cannot be combined with `--json` or with `--explain`: the first prints the report
alone, the second prints the whole staging object — which carries the same report under `report`
and the strip account under `strip` — and the third asks for an account there would be nothing to
print beside. The conflict is with the *value*: `--report text` is the default and sits happily
next to either flag.

## Reproducing a launch by hand

For local reproduction, explicitly enable sensitive capture. The `exec` record then holds
the program, argument vector and environment difference, with arrays encoded in strings.
Select the last `exec` fact rather than the last line: lifecycle events can follow it.

```console
$ GINARY_TRACE_SENSITIVE=1 GINARY_TRACE=/tmp/t.jsonl ./my_gleam_app --name world
$ jq -s -r 'map(select(.phase == "exec" and .kv.argv)) | last | .kv.argv | fromjson | @sh' /tmp/t.jsonl
'-boot' '/home/u/.cache/ginary/my_gleam_app/8f2a.../bin/no_dot_erlang' '-noshell' '+B' \
'-start_epmd' 'false' '-pa' '.../lib/my_gleam_app/ebin' '-eval' "'my_gleam_app@@main':run(...)" \
'-extra' '--name' 'world'
$ jq -s -r 'map(select(.phase == "exec" and .kv.argv)) | last | .kv.program, (.kv.env_set|fromjson[]), (.kv.env_remove|fromjson[])' /tmp/t.jsonl
```

Every `-pa` is recorded. The environment fields are a difference against the caller's inherited
environment; they are not a full environment snapshot, so blindly replaying them with `env -i`
can change behavior. Keep any environment prerequisites needed by the application locally.

The three questions that come before it have their own commands, and none of them needs the
application to start:

```console
$ GINARY_CMD=directory ./my_gleam_app      # where would this artifact extract to?
/home/u/.cache/ginary/my_gleam_app/8f2a1c3d5e7b9a02
$ GINARY_CMD=inspect ./my_gleam_app | jq .manifest.launch
$ GINARY_CMD=extract-only ./my_gleam_app   # extract, and stop
$ GINARY_DEBUG=1 ./my_gleam_app            # and the second run says `cache_hit`
$ ginary cache dir                         # the same resolution, from the build tool
$ ginary cache clean --app my_gleam_app    # throw the entry away and start cold
$ ginary cache prune --days 7              # remove what nothing has used for a week
$ ginary cache prune --all --app my_gleam_app   # every entry nobody is holding
$ GINARY_CMD=selftest ./my_gleam_app       # does the runtime start on this machine?
$ GINARY_CMD=uninstall ./my_gleam_app      # remove everything this artifact extracted
```

`selftest` is the first thing to run against a machine an artifact will not start on: it
separates "the payload will not extract" from "the tree is incomplete" from "the runtime will
not come up", and the third of those is the only one that needs a real BEAM.

To keep the intermediate tree instead:

```console
$ ginary stage --out /tmp/stage ...
```

## Collecting a local diagnostic report

```console
$ ginary diagnose ./my_gleam_app --trace /tmp/t.jsonl --crashdump /tmp/erl_crash.dump --out ./diagnosis
diagnostics: ./diagnosis/report.json
summary: ./diagnosis/summary.txt
```

The artifact and both evidence flags are optional; `--out` is required and must name a new
directory under an existing parent. The command probes known local tools and cache readiness,
verifies a supplied artifact, and summarizes the supplied trace and crash dump. It does not
run or extract the supplied artifact, fetch dependencies, or upload the report.

`report.json` and `summary.txt` omit raw command arguments, environment values, evidence paths,
trace values and crash-dump terms. They retain tool outcomes and remedies, artifact integrity
counts, trace failures and run fingerprints, and process/heap counts. Retain the originals
locally for detailed investigation. `diagnose::summarize` offers the same evidence collection
without environment probes or cache writes for library callers.

Artifact summaries include `checks.integrity` and `checks.contents`. An unperformed or
incomplete contents scan has null file, object and issue counts; an integrity check that
could not complete has `payload_ok: null`. Neither case is reported as zero findings.

Collection reads at most 8 MiB of trace data, 64 KiB per trace line, 1024 distinct trace runs,
and a 16 MiB crash-dump prefix. Exceeding a bound, encountering malformed or unsupported trace
lines, or reading a truncated dump sets the affected summary to incomplete. A failed evidence
source is reported alongside the sources that could still be read. `complete` describes
evidence collection, not application health: a completely collected report can describe a
failed artifact or runtime.

## JSON and library migration reference

Read the version field before interpreting a report. CLI JSON uses `format_version`; trace
events use `schema_version`. Existing version-1 report structs remain available where the
detailed API is additive.

| interface | current version | migration |
|---|---|---|
| `build --report json` | 2 | `status` states success or failure, including preflight errors before a target begins. `targets` contains completed target results. Partial failure additionally names `failed_target`, `unattempted`, retained `staging`, `warnings`, `error`, and `causes`. `sboms` lists every target/path pair; `sbom_errors` names sidecar failures. The optional legacy `sbom` field names the first successful document; consume `sboms` for multiple targets. |
| `cache clean --json` | 2 | `removed` names individual reclaimed entries and `bytes` counts only reclaimed bytes. `kept` contains `{path, reason}` rows, including `locked`, `active`, `unowned`, and `unremovable`. The old `cache::clean` and `CleanReport` shape remain; use `clean_detailed` for retention reasons. Removal obeys the safer ownership and locking rules in both APIs. |
| `doctor --json` | 2 | The established environment fields remain, with `tool_probes` and `findings`. `doctor::Report::gather()` retains version 1; `doctor::DetailedReport::gather()` produces version 2 and is what the command uses. |
| `verify --json` | 2 | Adds `checks.integrity` and `checks.contents` outcomes (`passed`, `failed`, `not_run`, `incomplete`) and `DuplicateIndex`, `DestinationConflict`, and `FormatMismatch` findings. Destination comparisons follow the artifact target, including Windows aliases and reserved names. Read failures also produce JSON with `error` and `causes`, omitting unknown findings rather than claiming an empty list. `verify::verify_detailed` exposes stage outcomes; `verify` and `VerifyReport` remain available. |
| `GINARY_TRACE` | 2 | Records add run identity, per-run sequence, event type and optional operation identity. A legacy record without `schema_version` is treated as version 1 by `diagnose`; new readers should tolerate additive fields. Default redaction replaces sensitive fields with `[redacted]`. |
| `diagnose` | 1 | Versioned local collection with environment readiness, optional evidence summaries, collection completeness and actionable findings. Raw evidence stays in its original files. |

Doctor tool outcomes distinguish `available`, `missing`, `spawn_failed`, `timed_out`,
`nonzero_exit`, `invalid_output`, `incomplete_output`, and `wait_failed`. A probe contains its
exit code, elapsed time, reason and remedy, bounded stdout/stderr, omission counts, output
completeness, and child-cleanup evidence. It only accepts a version from successful, complete
UTF-8 output. The retained output may help local debugging and should be reviewed before
sharing a full doctor JSON report; the diagnostic collection keeps only sanitized readiness.

Explicit trace operations emit `start` and then `end`, `failure`, or `interrupted`, paired by
`run_id` and `operation_id`. Ordinary facts have `event: fact`; legacy phase guards may emit
an `end` without an operation ID or a preceding start. Order records by `sequence` within one
run, and use `t_us` only within that run. Concurrent file writers lock each complete record;
write failures remain best effort and are available through `Diag::health()`. A Unix launcher
records a `handoff` fact before `execve`, since successful replacement cannot emit a final
launcher event. Windows and supervised runs can report their returned outcomes.

`process::run_command` retains bounded output even when spawning, waiting or timeout handling
fails. `CapturedOutput::is_complete()` must be true before treating the output as a full parse
input. `ProcessReport::cleanup` records whether termination was requested, whether the direct
child was reaped and whether a background reaper remains. Legacy subprocess wrappers retain
their signatures and include bounded failure evidence in their errors.

## Reading a crash dump

The launcher points `ERL_CRASH_DUMP` at the application's cache directory unless the user set
it, so a crash never litters the working directory. `ginary crashdump <path>` summarises it.

```console
$ ginary crashdump ~/.cache/ginary/my_gleam_app/erl_crash.dump
dump version:    0.5
date:            Mon Aug 31 11:52:30 2026
slogan:          kaboom
system version:  Erlang/OTP 29 [erts-17.0.5] [source] [64-bit] [smp:8:8] [jit:ns]
taints:          crypto, asn1rt_nif
processes:       43
truncated:       no

heap  pid       name                    initial call
6772  <0.44.0>  -                       erlang:apply/2
4185  <0.45.0>  application_controller  application_controller:start/1
```

The `slogan` is why the runtime died and is the first thing to read. `taints` lists the NIFs and
drivers that were loaded, which is where to look when the answer is a segfault rather than an
Erlang error. The table is the five largest processes by heap, in words, which is where to look
when the answer is memory.

Two properties of the reader matter when the dump is a real one. It is never read into memory —
a dump from a runtime that died of memory exhaustion is routinely larger than the machine it is
being read on, so the file is streamed a line at a time, a single line contributes at most 64 KB
to any value, and the top processes are kept in a list of five rather than collected and sorted.
And a dump that stops mid-section is summarised rather than refused: a runtime killed while
writing its dump leaves exactly that, it is the case a reader most needs, and `truncated: yes`
says so. `--json` gives the same fields as an object.

## `verify`, and how it differs from `inspect --verify`

`ginary inspect --verify` streams the payload past a hasher and compares the result with the
trailer. That is the check the launcher itself makes, it costs one pass, and it answers exactly
one question: are these the bytes ginary wrote?

`ginary verify` is the deep check, and it exists because a payload whose digest matches can
still be wrong.

```console
$ ginary inspect --verify ./my_gleam_app
...
verify: ok
$ ginary verify ./my_gleam_app
payload:  ok
files:    248 checked against the index
objects:  6
...
issues:
  lib/hello/priv/lib/nif.so: needs `libssl.so.3`, which the artifact does not carry
$ echo $?
1
```

It streams the payload a second time and, per file:

- checks it against its `ginary.index.json` row — *every* file, not only the native ones, so an
  artifact whose index does not describe what it carries is a finding rather than a surprise at
  run time. All three of the row's columns, because a row can hold the right digest and the wrong
  metadata: the bytes are `IndexMismatch`, the length is `IndexSizeMismatch`, and the permission
  bits are `IndexModeMismatch` — the header mode is checked against the *normalisation* of the
  staged mode the row records, `0755` when it has the user execute bit and `0644` otherwise,
  which is the relation `docs/format.md` fixes and not plain equality. A file the index does not
  name is `IndexOrphan`, and an index row naming nothing is `IndexMissing`;
- reads it into memory only when its first bytes identify ELF, PE or Mach-O, and only up to
  100 MB, then checks its native headers: a machine that is not the one the manifest targets is
  `MachineMismatch`, an object format for a different operating system is `FormatMismatch`,
  and a `DT_NEEDED` outside the allowlist in `src/verify.rs` is
  `UnexpectedNeeded` — a library the artifact expects a stranger's machine to already have. A
  file that begins with native magic and does not parse is `UnreadableObject`, because a
  file that looks like native code and is not readable as native code is the reader's decision
  and not the verifier's.

It also checks *where each entry lands* and what it *is*, which are the two rules
`payload::unpack` applies and a report has to apply too. An entry whose name is absolute, holds
`..`, or normalises to nothing is `UnsafePath` — `payload::destined_path_for` is the shared
target-aware rule —
and it is raised before the index is consulted, so an escaping entry counts towards neither
`files_checked` nor `IndexOrphan`. The kind check is by position rather than by name:
`ginary.json` and `ginary.index.json` are entries 0 and 1; an entry after them landing on either
name — as the name itself or as a directory holding a file — is `ReservedEntry`, which is the
payload `payload::unpack` refuses outright. An entry that is neither a regular file nor a
directory is `UnsupportedEntry`, naming what it is instead. A directory entry is the one thing
passed over: `docs/format.md` permits one and `ginary.index.json` lists files only, so there is
nothing to check it against.

Target names retain their own separator rules on every verification host. A backslash or a
colon is an ordinary character in a Linux/macOS filename; Windows target names fold case and
reject device names and unsupported Win32 spellings. Verification can inspect a valid foreign
target without extracting it. If extraction is explicitly requested on a foreign host,
`payload::unpack` also requires the host to represent every component without changing its
meaning. It refuses a Unix backslash filename on Windows, Windows backslash separators on
Unix, and distinct Unix names or parent directories that would merge through Windows case
folding. The index is checked before its files are written, and a failed extraction never
publishes the cache completion marker. The older `payload::destined_path` API retains native
host component semantics for callers already using it.

Nothing is extracted and nothing is run, so `verify` is safe to point at an artifact somebody
else built. It exits 0 when there is nothing to say and 1 with the table above otherwise;
`--json` carries the whole report, including the object table.

When the payload digest itself does not match, `verify` stops there and says so: every entry
past the damage is bytes nobody wrote, and a table of findings about them would describe the
damage rather than the artifact.

The report explicitly marks content checks as `not_run` after a digest mismatch. If the
digest passes but an archive cannot be fully read, its contents outcome is `incomplete`.
Unreadable front matter leaves integrity `incomplete` and contents `not_run`. These errors
still produce `--json` output and a nonzero exit code. Library callers can use
`verify::verify_detailed`, or `VerifyError::checks()` on an unsuccessful read.

## Exit codes

Codes 121 to 125 come from the launcher, never from the application. Every one of them is
accompanied by exactly one line on standard error beginning `ginary: `, and some carry a second
line beginning `hint: `.

| code | meaning | what to look at |
|---|---|---|
| 121 | the running executable could not be opened, or ginary panicked | `/proc` not mounted, or a bug: a panic prints `ginary: internal error (this is a bug in ginary): ...` |
| 122 | the trailer is unusable, or the manifest is a format this build does not read | the file was truncated, padded, or built by a newer ginary. A file with *no* magic at all is not this: it is the ginary command line tool |
| 123 | the payload is corrupt | the digest does not match, or an entry is a symlink, a device or a path that leaves the root. Nothing is left in the cache |
| 124 | the cache could not be written or read | permissions, a full disk, or an extracted runtime that is still incomplete after one repair. The message names the path and, for a failed preflight, the file |
| 125 | the runtime would not start | `execve` failed. `ENOENT` on a program that is on disk means its `ld-linux` or one of its libraries is missing; `EACCES` on a program that is executable means the cache is on a `noexec` mount — set `GINARY_CACHE_DIR` |

A packaged application's *own* exit code passes through untouched, including 0 and including
any of 121 to 125 it chooses to leave. Unix normally replaces the launcher with the runtime;
Windows and supervised runs relay the child's result.

The CLI half prints `error: ...` followed by one `  caused by: ...` line per cause and exits 1.
A clap usage error, and an unrecognised `GINARY_CMD`, exit 2.

ADR [0008](../adr/0008-launcher-exit-codes-and-env-protocol.md) records why the numbers start at
121 and why maintenance travels in the environment.
