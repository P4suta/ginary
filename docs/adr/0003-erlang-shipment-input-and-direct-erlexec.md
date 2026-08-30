<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# 0003 — Package an erlang-shipment and start it by exec'ing erlexec directly

Status: Accepted · 2026-08-30

## Context

Gleam offers two export formats. `gleam export escript` produces a single archive but cannot
carry `priv/` and cannot handle NIFs, which rules out any application that reads a data file or
loads native code. `gleam export erlang-shipment` produces
`build/erlang-shipment/<app>/{ebin,priv,include}` with development dependencies already
excluded, and `code:priv_dir/1` resolves correctly against it.

The shipment also ships an `entrypoint.sh`. It cannot be used as the process ginary starts: its
shebang is on the fourth line, so it is not `execve`-able, and it resolves `erl` from `PATH`,
which is precisely the dependency ginary exists to remove.

Starting `erlexec` directly was investigated on OTP 29.0.5 (erts 17.0.5). The findings:

- `erlexec` derives nothing from `argv[0]`. It needs `ROOTDIR`, `BINDIR`, `EMU=beam` and
  `PROGNAME` in the environment, and it also reads `HOME` — with `HOME` unset it terminates
  abnormally.
- `-boot <root>/bin/no_dot_erlang` starts kernel and stdlib only and does not read `~/.erlang`,
  which is what a hermetic launch requires. The `no_dot_erlang.boot` file hard-codes
  `$ROOT/lib/kernel-11.0.3/ebin` and `$ROOT/lib/stdlib-8.0.3/ebin`, so those two directories
  must exist with their exact versioned names.
- The version-less shipment directories can be placed on the code path with `-pa`. With that
  layout `code:priv_dir/1`, `gleam_erlang_ffi:priv_directory/1` and
  `application:ensure_all_started(gleam_crypto)` all work, and the only OTP applications loaded
  are `kernel`, `stdlib` and `crypto` — `compiler`, `sasl` and `runtime_tools` are not needed.
- `releases/` is never consulted by `erlexec`, and the system starts without `release_handler`.
- `erl_child_setup` (ports and `os:cmd`) and `inet_gethost` (DNS) are spawned from `$BINDIR` and
  are therefore mandatory. `epmd`, `heart`, `escript`, `erlc`, `dialyzer`, `typer`, `ct_run`,
  `yielding_c_fun`, `run_erl`, `to_erl` and `erl_call` are not.
- Gleam generates `<app>@@main:run/1`, which calls `application:ensure_all_started`, then
  `main()`, then `init:stop(0|1)`. User arguments arrive after `-extra` and are read by
  `init:get_plain_arguments()`.

## Decision

The input to `ginary build` is the output of `gleam export erlang-shipment`, and on Unix the
launcher **replaces its own process** with:

```
program: <root>/erts-<vsn>/bin/erlexec
args:    -boot <root>/bin/no_dot_erlang -noshell +B [-start_epmd false] [+fnu]
         -pa <root>/lib/<app>/ebin <root>/lib/<dep>/ebin ...
         [-args_file ...] [-config ...] <manifest erl_flags> <GINARY_ERL_FLAGS>
         -eval "'<app>@@main':run('<app>')"
         -extra <the user's argv[1..], unmodified, as OsString>
env set: ROOTDIR BINDIR EMU=beam PROGNAME=<app>
         HOME and ERL_CRASH_DUMP only when the user has not set them
env rm:  ERL_LIBS ERL_FLAGS ERL_AFLAGS ERL_ZFLAGS ERL_OTP*_FLAGS ERL_ROOTDIR ERL_EPMD_PORT
```

`entrypoint.sh` and `releases/` are not shipped. Which OTP applications to include is computed
from the transitive closure of `applications` and `included_applications` in the `.app` files,
seeded with the shipment roots plus `kernel` and `stdlib`, rather than from a fixed list. Before
packaging, the boot file is scanned for `$ROOT/lib/<name>-<vsn>/ebin` references and each one is
verified to exist in the staging root.

Windows has no `execve`; there the launcher spawns `erl.exe`, which resolves its own root, and
waits, with the child in a Job Object so it dies with the launcher.

## Consequences

Exit codes and signals need no forwarding on Unix: the BEAM *is* the process, so `init:stop(N)`
and `halt(N)` reach the shell directly. The launcher does not interpret `argv` at all, so a
packaged application receives `--help` and `--version` itself.

Hot code upgrades are not supported, because `releases/` and `release_handler` are absent. The
runtime is hermetic: a user's `ERL_LIBS` or `~/.erlang` cannot change what a packaged
application loads, which also means a user cannot use those to patch one.

Because `erlexec` reads `HOME`, the launcher must supply one when the environment has none — a
detail that only shows up in `env -i` containers and CI.

The dependency closure being computed rather than fixed means an application that needs an OTP
application not reachable from its `.app` files must say so, through Gleam's
`[erlang] extra_applications` (bundled and started) or ginary's `otp_applications` (bundled
only). The error for a missing application names what requested it and where ginary looked.
