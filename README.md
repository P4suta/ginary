<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# ginary

`ginary` turns a Gleam application into one executable file. The people who run that file need
no Erlang installation, no `PATH` entry and no unpacking step: they copy the file, mark it
executable and run it.

Gleam on the Erlang target cannot do this today. `gleam export escript` and
`gleam export erlang-shipment` both hand the end user an artifact that still requires a BEAM
runtime on the machine. ginary takes the shipment as input and appends it, together with a
trimmed BEAM runtime, to a copy of the ginary binary itself.

## Status

**Alpha.** `ginary build` works end to end on Linux x86_64 against the host's own OTP
installation: run it in a Gleam project and it writes one executable that runs on a machine with
no Erlang. Cross-target builds, prebuilt runtimes and the runtime-configuration keys are not
implemented yet — see [Limitations](#limitations) and [CHANGELOG.md](CHANGELOG.md).

## Quickstart

There is no release yet, so install from the repository:

```console
$ cargo install --git https://github.com/P4suta/ginary --locked
```

You need `gleam`, an OTP installation (`erl` on `PATH`) and `strip` on the machine that
*builds*. The machine that *runs* the artifact needs none of them.

```console
$ cd my_gleam_app
$ ginary build
category      files  before    after     saved
erts_binary   4      56602456  10722528  45879928
otp_beam      202    10147388  1886063   8261325
...
total         214    66775640  12626139  54149501

needs: libc.so.6 (GLIBC_2.38), libgcc_s.so.1, libm.so.6, libstdc++.so.6, libtinfo.so.6

artifact: build/ginary/my_gleam_app (3235824 stub + 4101920 payload + 64 trailer)
```

and on a machine with no Erlang at all:

```console
$ command -v erl || echo "no erlang here"
no erlang here
$ ./my_gleam_app arg1 arg2
```

The first run extracts the runtime into `~/.cache/ginary/<app>/<key>` and then `execve`s it;
every later run finds the entry already there. `ginary inspect ./my_gleam_app` says what is
inside one, `--verify` re-hashes its payload, and `--launch-plan` prints the argument vector and
environment the launcher would use.

## Configuration

Settings live in the project's `gleam.toml`, under `[tools.ginary]`, which the Gleam compiler
ignores. Every key is optional, and a project with no table at all builds with the defaults.

```toml
[tools.ginary]
output = "build/ginary"        # directory the artifact is written into
strip = true                   # strip the runtime; the two keys below override it
strip_elf = true               # strip the native binaries
strip_beams = true             # strip the .beam modules
compression_level = 19         # zstd, 1 to 22
otp_applications = ["sasl"]    # extra applications: bundled, not started
erts_extra_bins = ["heart"]    # extra programs from the runtime's bin
erl_flags = ["+fnu"]           # emulator flags placed before -eval
```

A key ginary does not know is an error naming the key and the file: a setting the user believes
is in force and nothing reads is worse than a refused build. `erl_flags` may not hold `-boot`,
`-extra`, `-noshell`, `-pa` or `-pz`, because the launcher builds each of those itself from the
manifest. An `erts_extra_bins` entry — and a `--extra-bin` — is one program *name*: the value is
joined onto the runtime's `bin` directory to find the program and onto the staged one to write
it, so a name holding a path separator or `..` is refused rather than followed.

Every command-line flag wins over the table, and the table wins over the defaults. The two list
settings merge rather than replace: `--extra-otp-app` is appended to `otp_applications` and
`--extra-bin` to `erts_extra_bins`, deduplicated. Run `ginary build --help` for the full list.

## Exit codes

A packaged application exits with whatever the Gleam program exited with. The codes 121 to 125
are ginary's own and can only come from the launcher, before the application started:

| code | meaning |
|---|---|
| 121 | the running executable could not be opened, or ginary panicked |
| 122 | the trailer is unusable, or the manifest is a format this build does not read |
| 123 | the payload is corrupt |
| 124 | the cache could not be written or read |
| 125 | the runtime would not start |

The `ginary` command line tool itself exits 0 on success, 1 on a failure it reports, and 2 on a
usage error.

## Environment variables

| variable | read by | effect |
|---|---|---|
| `GINARY_CACHE_DIR` | the artifact | Use this directory as the cache root, verbatim. The escape hatch for a read-only or `noexec` home directory. |
| `GINARY_ERL_FLAGS` | the artifact | Extra emulator flags for one run, split on whitespace and placed before `-eval`. |
| `GINARY_CMD` | the artifact | Maintenance, kept out of `argv`: `directory` prints the cache entry and creates nothing, `extract-only` extracts and prints it, `inspect` prints the manifest and geometry as JSON. |
| `GINARY_DEBUG=1` | both | One human-readable line per phase on standard error. |
| `GINARY_TRACE=<file>` | both | One JSON object per phase, appended to the file. |
| `GINARY_SUPERVISE=1` | the artifact | Spawn and wait instead of `execve`; a child killed by a signal exits `128 + signo`. |
| `SOURCE_DATE_EPOCH` | `ginary build` | Pins the manifest's `created_at`, so two builds of one project produce byte-identical artifacts. |

`GINARY_PRUNE_DAYS`, `GINARY_OFFLINE` and `ginary cache prune` are planned rather than
implemented; see [docs/dev/debugging.md](docs/dev/debugging.md) for the whole table.

## How it works

```
ginary build
  gleam export erlang-shipment      the application and its hex dependencies
  + the host OTP root               erts-<vsn>, kernel, stdlib, and whatever the
                                    .app dependency closure actually needs
  -> staging root -> strip -> tar (deterministic) + zstd -> payload
  -> copy of the ginary binary + payload + 64-byte trailer
  -> build/ginary/<app>

./my_gleam_app args...
  read the trailer at the end of my own executable
    no trailer  -> this is the ginary CLI, parse argv with clap
    trailer     -> launcher mode, never look at argv
        resolve  <cache>/<app>/<sha256[:16]>
        extract  into a sibling temp directory, then rename (atomic)
        execve   <root>/erts-<vsn>/bin/erlexec
                   -boot <root>/bin/no_dot_erlang -noshell +B
                   -pa <root>/lib/*/ebin
                   -eval "'<app>@@main':run('<app>')"
                   -extra <the user's arguments, unmodified>
```

The launcher replaces its own process with the BEAM, so exit codes and signals need no
forwarding, and the application receives its arguments exactly as typed.

## Limitations

- **Linux x86_64 only, against the host's OTP.** The runtime that goes into an artifact is the
  one `erl` reports on the build machine. Cross-target builds, prebuilt runtime downloads and
  musl variants are Phase C of [the roadmap](docs/dev/log/); `--otp-root` is the only override
  today.
- **glibc, dynamically linked.** A host-OTP artifact needs the C library of the machine it was
  built on, or newer. The `needs:` line every build prints is the exact list — for the OTP 29.0.5
  runtime this repository is developed against it is `libc.so.6`, `libgcc_s.so.1`, `libm.so.6`,
  `libstdc++.so.6` and `libtinfo.so.6`, with a `GLIBC_2.38` floor. An artifact built on Ubuntu
  24.04 will not start on Debian 12 (glibc 2.36), and ginary says so at build time rather than
  leaving the user's loader to.
- Erlang target only. The Gleam JavaScript target is out of scope.
- The BEAM is bundled, not embedded. The runtime is extracted to a per-user cache directory on
  first run; ginary does not link the emulator into the executable. The cache is never pruned
  yet.
- Hot code upgrades are not supported. `releases/` is not shipped and `release_handler` is not
  available.
- Native code (NIFs, port programs) must match the target being packaged, and nothing is
  recorded about it in the manifest yet.
- Runtime configuration — `vm_args`, `sys_config`, distribution and `epmd` — is Phase B.
- Artifacts are not small. A trimmed runtime plus a small application is roughly 7.5 MB.

## Documentation

- [docs/format.md](docs/format.md) — the payload trailer and manifest specification.
- [docs/dev/architecture.md](docs/dev/architecture.md) — module map and data flow.
- [docs/dev/testing.md](docs/dev/testing.md) — test infrastructure and toolchain gating.
- [docs/dev/debugging.md](docs/dev/debugging.md) — diagnostic environment variables.
- [docs/adr/](docs/adr/) — architecture decision records.
- [CONTRIBUTING.md](CONTRIBUTING.md) — the TDD workflow and the local gates.

## Licence

Dual licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
