<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Debugging

Every switch below is listed with its status. **Implemented** means it works in the current
revision. **Planned** means the design is settled but no code exists; setting the variable does
nothing today.

## Environment variables

| variable | status | effect |
|---|---|---|
| `GINARY_CACHE_DIR` | implemented (resolution only) | Overrides the cache root. Resolution and precedence are implemented and reported by `ginary doctor`; nothing extracts into it yet. |
| `GINARY_DEBUG=1` | planned | Human-readable progress on stderr, prefixed `ginary[debug]:`: the resolved self path, the trailer, the cache decision, the argv and env difference handed to the runtime, and per-phase timings. |
| `GINARY_TRACE=<file\|dir\|1>` | planned | JSON Lines trace, one object per phase. `1` writes `<app_dir>/trace/<timestamp>-<pid>.jsonl` and keeps the last 20 files. The `LaunchPlan` is always recorded immediately before `execve`, so `ginary trace show <file>` can print a copy-pasteable `env -i ... erlexec ...` command that reproduces the launch. |
| `GINARY_SUPERVISE=1` | planned | Spawns the runtime and waits instead of calling `execve`, which is the code path Windows uses anyway. Records the exit status, the signal, the elapsed time and a summary of any `erl_crash.dump`. |
| `GINARY_CMD=<command>` | planned | Artifact-side maintenance, kept out of `argv` so the packaged application still owns its own flags. Values: `inspect`, `directory`, `extract-only`, `selftest` (runs `erlexec -eval 'halt(0)'` against the extracted runtime), `uninstall`. |
| `GINARY_FAULT=<point>[:<action>]` | planned | Fault injection, compiled in only under `cfg(feature = "fault-injection")` and therefore absent from release builds. Points: `after-extract:pause`, `rename:eexist`, `write:enospc`, `exec:eacces`, `trailer:corrupt`. |
| `GINARY_ERL_FLAGS` | planned | Extra emulator flags appended to the manifest's, for one run. |
| `GINARY_OFFLINE=1` | planned | The builder refuses to reach the network and lists what it would have fetched. |
| `GINARY_REQUIRE_TOOLCHAIN=1` | implemented (convention) | Turns a skipped toolchain-gated test into a failure. See [testing.md](testing.md). |

The launcher deliberately **removes** `ERL_LIBS`, `ERL_FLAGS`, `ERL_AFLAGS`, `ERL_ZFLAGS`,
`ERL_OTP*_FLAGS`, `ERL_ROOTDIR` and `ERL_EPMD_PORT` before starting the runtime (planned). If a
packaged application behaves differently from a `gleam run`, that scrubbing is the first thing
to check.

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

## Reproducing a launch by hand (planned)

Once the launcher exists, the intended loop is:

```console
$ GINARY_TRACE=1 ./my_gleam_app
$ ginary trace show ~/.cache/ginary/my_gleam_app/trace/*.jsonl
env -i HOME=/tmp ROOTDIR=... BINDIR=... EMU=beam PROGNAME=my_gleam_app \
  ~/.cache/ginary/my_gleam_app/<key>/erts-17.0.5/bin/erlexec \
  -boot .../bin/no_dot_erlang -noshell +B -start_epmd false \
  -pa .../lib/my_gleam_app/ebin -eval "'my_gleam_app@@main':run('my_gleam_app')" -extra
```

and, to keep the intermediate tree:

```console
$ ginary build --keep-staging --staging-dir /tmp/stage
$ ginary stage --out /tmp/stage
```

## Reading a crash dump (planned)

The launcher points `ERL_CRASH_DUMP` at the application's cache directory unless the user set
it, so a crash never litters the working directory. `ginary crashdump <path>` summarises the
`Slogan`, the system version and the largest processes.

## Exit codes

Codes 121 to 125 come from the launcher, not from the application; they are listed in
[../format.md](../format.md). The CLI prints `error: ...` followed by one `  caused by: ...`
line per cause and exits 1. A clap usage error exits 2.
