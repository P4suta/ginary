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
| `GINARY_TRACE=<file>` | implemented | JSON Lines, one object per phase, appended to the file: `{"t_us":..,"phase":..,"kv":{..}[,"elapsed_us":..]}`. The whole `LaunchPlan` is recorded immediately before `execve`, so the launch that failed can be reproduced from the trace. A file that cannot be opened costs one warning and the run carries on. |
| `GINARY_SUPERVISE=1` | implemented | Spawns the runtime and waits instead of calling `execve`, which is the code path Windows will use anyway. The exit code is mirrored; a child killed by a signal exits `128 + signo`. Records the exit status, the signal and the elapsed time, and if an `erl_crash.dump` appeared during the run, prints its `Slogan` line. |
| `GINARY_CMD=<command>` | implemented | Artifact-side maintenance, kept out of `argv` so the packaged application still owns its own flags, and one of five values. `directory` prints the cache entry the artifact would use and creates nothing; `extract-only` extracts and prints the entry without launching; `inspect` prints the manifest, the payload geometry and the digest as one JSON object; `selftest` extracts, preflights and starts the runtime with `-eval erlang:halt(0)` and no `-extra`, printing `extract:`, `preflight:` and `run:` with `PASS` or `FAIL` and exiting 0 or 1; `uninstall` removes every cache entry of this application that nobody holds, prints what it removed and what it kept and why, and exits 0 even when it kept something. Any other value is a usage error and exits 2. |
| `GINARY_ERL_FLAGS` | implemented | Extra emulator flags for one run, split on ASCII whitespace and placed after the manifest's own flags and before `-eval`. |
| `GINARY_FAULT=<point>[:<action>]` | implemented (test builds) | Fault injection, compiled in only under `cfg(feature = "fault-injection")` and therefore absent from release builds, which never read the variable at all. Points: `after-extract:pause` (sleep with the temporary tree on disk), `rename:eexist` (extract, then lose the rename race), `unpack:corrupt` (the payload changes under the reader), `before-lock` (the cache entry is removed between the preflight and the shared lock, which is what a prune that won the race leaves behind), `launcher:panic` (panic on the launcher path, so the panic hook has something to catch), `pack:fail` (the *builder* stops between the stub and the payload, so a test can assert that a failed build leaves neither a work directory nor a half-written artifact). |
| `GINARY_PRUNE_DAYS=<n>` | implemented | How many days an unused cache entry of the running application may live before the next launch prunes it. Defaults to 14; `0` turns pruning off for that run. A value that is not a count of days falls back to the default rather than failing a launch: a misspelt housekeeping preference must not stop an application from starting. |
| `GINARY_OFFLINE=1` | planned | The builder refuses to reach the network and lists what it would have fetched. |
| `GINARY_REQUIRE_TOOLCHAIN=1` | implemented (convention) | Turns a skipped toolchain-gated test into a failure. See [testing.md](testing.md). |

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
  it either: `--all` is "whatever its age", not "whatever is using it". `ginary cache clean` is
  the blunt instrument that ignores the lock, and running it under a live application is the
  thing the lock exists to prevent.
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
entry carries its reason (`locked`, `fresh` or `unremovable`) in the same string. An entry that
vanished has to be explainable from a trace, and a count explains nothing. Nothing a prune does
reaches standard error: pruning is housekeeping, and housekeeping does not decide whether an
application starts.

`GINARY_CMD=uninstall` removes only what the cache wrote — `<key>` entries and the
`.<key>.tmp-<pid>`, `.<key>.corrupt-<pid>` and `.<key>.trash-<pid>` residue beside them. Anything
else in the application directory is left where it is, `erl_crash.dump` included, which is why
that directory survives an uninstall when a dump is in it.

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
  creates a file in the resolved cache directory, chmods it 0755 and tries to spawn it, then
  removes it. `access(2)` reports the mode bits and says nothing about the mount, and a cache on
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
`{name, found, version, path}`. Each tool probe is killed after ten seconds, so a hung `docker`
cannot hang `doctor`; the tool is then reported as found with no version.

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

`GINARY_TRACE` exists so that a bug report carries the launch that failed rather than a
description of it. The last `exec` record holds the program, the whole argument vector and the
environment difference, each as a JSON array encoded in a string:

```console
$ GINARY_TRACE=/tmp/t.jsonl ./my_gleam_app --name world
$ tail -1 /tmp/t.jsonl | jq -r '.kv.argv | fromjson | @sh'
'-boot' '/home/u/.cache/ginary/my_gleam_app/8f2a.../bin/no_dot_erlang' '-noshell' '+B' \
'-start_epmd' 'false' '-pa' '.../lib/my_gleam_app/ebin' '-eval' "'my_gleam_app@@main':run(...)" \
'-extra' '--name' 'world'
$ tail -1 /tmp/t.jsonl | jq -r '.kv.program, (.kv.env_set|fromjson[]), (.kv.env_remove|fromjson[])'
```

Those three pieces are a runnable `env -i` command, and nothing is elided: every `-pa` is in the
record, which is what makes the reproduction complete rather than indicative.

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

- hashes it against `ginary.index.json` — *every* file, not only the native ones, so an artifact
  whose index does not describe what it carries is a finding rather than a surprise at run time
  (`IndexMismatch`), as is a file the index does not name (`IndexOrphan`) or an index row naming
  nothing (`IndexMissing`);
- reads it into memory only when its first bytes are the ELF magic, and only up to 100 MB, and
  then asks `src/elf.rs` what it is: a machine that is not the one the manifest targets is
  `MachineMismatch`, and a `DT_NEEDED` outside the allowlist in `src/verify.rs` is
  `UnexpectedNeeded` — a library the artifact expects a stranger's machine to already have. A
  file that begins with the magic and does not parse as an ELF is `UnreadableObject`, because a
  file that looks like native code and is not readable as native code is the reader's decision
  and not the verifier's.

It also checks *where each entry lands* and what it *is*, which are the two rules
`payload::unpack` applies and a report has to apply too. An entry whose name is absolute, holds
`..`, or normalises to nothing is `UnsafePath` — `payload::destined_path` is the shared rule —
and it is raised before the index is consulted, so an escaping entry counts towards neither
`files_checked` nor `IndexOrphan`. The kind check is by position rather than by name:
`ginary.json` and `ginary.index.json` are entries 0 and 1; an entry after them landing on either
name — as the name itself or as a directory holding a file — is `ReservedEntry`, which is the
payload `payload::unpack` refuses outright. An entry that is neither a regular file nor a
directory is `UnsupportedEntry`, naming what it is instead. A directory entry is the one thing
passed over: `docs/format.md` permits one and `ginary.index.json` lists files only, so there is
nothing to check it against.

Nothing is extracted and nothing is run, so `verify` is safe to point at an artifact somebody
else built. It exits 0 when there is nothing to say and 1 with the table above otherwise;
`--json` carries the whole report, including the object table.

When the payload digest itself does not match, `verify` stops there and says so: every entry
past the damage is bytes nobody wrote, and a table of findings about them would describe the
damage rather than the artifact.

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
any of 121 to 125 it chooses to leave: the launcher is gone by then, replaced by the runtime.

The CLI half prints `error: ...` followed by one `  caused by: ...` line per cause and exits 1.
A clap usage error, and an unrecognised `GINARY_CMD`, exit 2.

ADR [0008](../adr/0008-launcher-exit-codes-and-env-protocol.md) records why the numbers start at
121 and why maintenance travels in the environment.
