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
| `GINARY_CMD=<command>` | implemented | Artifact-side maintenance, kept out of `argv` so the packaged application still owns its own flags. `directory` prints the cache entry the artifact would use and creates nothing; `extract-only` extracts and prints the entry without launching; `inspect` prints the manifest, the payload geometry and the digest as one JSON object. Any other value is a usage error and exits 2. `selftest` and `uninstall` are still planned. |
| `GINARY_ERL_FLAGS` | implemented | Extra emulator flags for one run, split on ASCII whitespace and placed after the manifest's own flags and before `-eval`. |
| `GINARY_FAULT=<point>[:<action>]` | implemented (test builds) | Fault injection, compiled in only under `cfg(feature = "fault-injection")` and therefore absent from release builds, which never read the variable at all. Points: `after-extract:pause` (sleep with the temporary tree on disk), `rename:eexist` (extract, then lose the rename race), `unpack:corrupt` (the payload changes under the reader), `launcher:panic` (panic on the launcher path, so the panic hook has something to catch). |
| `GINARY_OFFLINE=1` | planned | The builder refuses to reach the network and lists what it would have fetched. |
| `GINARY_REQUIRE_TOOLCHAIN=1` | implemented (convention) | Turns a skipped toolchain-gated test into a failure. See [testing.md](testing.md). |

The launcher **removes** `ERL_LIBS`, `ERL_FLAGS`, `ERL_AFLAGS`, `ERL_ZFLAGS`, `ERL_ROOTDIR`,
`ERL_EPMD_PORT` and every variable whose name begins `ERL_OTP` and ends `_FLAGS` before starting
the runtime. If a packaged application behaves differently from a `gleam run`, that scrubbing is
the first thing to check. It **sets** `ROOTDIR`, `BINDIR`, `EMU=beam` and `PROGNAME`
unconditionally, and `HOME` and `ERL_CRASH_DUMP` only when the caller has not: a `HOME` you
exported is yours.

## Diagnosing the environment today

```console
$ ginary doctor
host target: linux-x86_64-gnu
rustc/cargo: not required (neither ginary nor its artifacts need a Rust toolchain)
cache dir: /home/user/.cache/ginary (from HOME)
gleam: 1.18.1 (/usr/local/bin/gleam)
erl: OTP 29, erts 17.0.5 (/usr/local/bin/erl)
strip: 2.42 (/usr/bin/strip)
docker: not found
```

`ginary doctor --json` prints the same information as an object with `format_version`,
`host_target`, `rustc_required`, `cache_dir`, `cache_dir_source`, `cache_dir_error` and a
`tools` array of `{name, found, version, path}`. Each probe is killed after ten seconds, so a
hung `docker` cannot hang `doctor`; the tool is then reported as found with no version.

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
```

To keep the intermediate tree instead:

```console
$ ginary stage --out /tmp/stage ...
```

## Reading a crash dump (planned)

The launcher points `ERL_CRASH_DUMP` at the application's cache directory unless the user set
it, so a crash never litters the working directory. `ginary crashdump <path>` summarises the
`Slogan`, the system version and the largest processes.

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
