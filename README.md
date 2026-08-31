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

## Reading an artifact

Four commands read a packaged application from the outside. None of them extracts anything and
none of them runs it, so all four are safe to point at a file somebody else built.

```console
$ ginary inspect ./my_gleam_app --verify   # what is in it, and does the payload still hash
$ ginary verify ./my_gleam_app             # the deep check: every file, and every binary
$ ginary sbom ./my_gleam_app               # an SPDX 2.3 bill of materials beside it
$ ginary crashdump ./erl_crash.dump        # why a packaged application died
```

`inspect --verify` re-hashes the payload against the trailer and stops there. `verify` streams
it a second time and answers four more questions: whether every file is the one
`ginary.index.json` describes, whether every entry is one a launcher would extract at all,
whether every native binary in it was built for the machine the artifact targets, and whether
any of them needs a shared library the artifact does not carry. It exits 1 with a table when it
finds something, and `--json` gives the whole report.

```console
$ ginary verify ./my_gleam_app
payload:  ok
files:    248 checked against the index
objects:  6

path                                        machine  class  glibc  needed
erts-17.0.5/bin/beam.smp                    x86_64   64     2.38   libc.so.6, libm.so.6
lib/crypto-5.9.2/priv/lib/crypto.so         x86_64   64     2.34   libc.so.6
...
```

`ginary build --sbom` writes the same document a build produces, as `<app>.spdx.json` beside the
artifact, or wherever `--sbom-out` says. It is a function of the artifact and of the project's
`manifest.toml`: the document namespace comes from the payload's SHA-256 rather than from a
random UUID and a clock, so a reproducible build has a reproducible bill of materials. A package
whose origin the shipment does not record is `NOASSERTION` rather than a guess.

`ginary crashdump` reads an `erl_crash.dump` as a stream — a dump can be larger than the machine
it is read on — and prints the slogan, the system version, the taints and the five largest
processes by heap. A dump the runtime was killed while writing is summarised rather than
refused, and says so.

`ginary doctor` answers the other half: whether *this* machine can build, whether the cache
directory can be written to and executed out of (a `noexec` mount is the failure users actually
hit), what project it is standing in, and what the host OTP's `crypto` NIF needs.

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
erl_flags = ["+SDio", "4"]     # emulator flags placed before -eval

vm_args = "config/vm.args"     # an erl -args_file, copied into the artifact
sys_config = "config/sys.config"  # a sys.config, copied into the artifact
distribution = false           # bundle epmd and start the runtime distributed
filename_encoding = "utf8"     # utf8 | latin1 | auto -> +fnu | +fnl | +fna
heart = false                  # bundle heart and start the runtime under it
targets = ["host"]             # host | all | a canonical target name, repeatable

[tools.ginary.env]             # variables set only when the caller has not
LOG_LEVEL = "info"

[tools.ginary.target.linux-aarch64-musl]   # one sub-table per target
erts = "catalog"               # host | catalog | dir:PATH | tarball:PATH | docker:IMAGE
otp_variant = "static"         # the catalog variant: static | dynamic | default
```

A key ginary does not know is an error naming the key and the file: a setting the user believes
is in force and nothing reads is worse than a refused build. `erl_flags` may not hold `-boot`,
`-extra`, `-noshell`, `-pa` or `-pz`, because the launcher builds each of those itself from the
manifest. An `erts_extra_bins` entry — and a `--extra-bin` — is one program *name*: the value is
joined onto the runtime's `bin` directory to find the program and onto the staged one to write
it, so a name holding a path separator or `..` is refused rather than followed.

Every command-line flag wins over the table, and the table wins over the defaults. The two list
settings merge rather than replace: `--extra-otp-app` is appended to `otp_applications` and
`--extra-bin` to `erts_extra_bins`, deduplicated. `--distribution`, `--vm-args` and
`--sys-config` override the three runtime settings of the same name; a flag naming a file is
relative to the working directory, while a value in the table is relative to the project. Run
`ginary build --help` for the full list.

### Targets

`targets` says what one build produces and `[tools.ginary.target.<name>]` says how to produce
one; they are two keys because TOML will not let one name be both an array of strings and a
table. `--target` is repeatable and replaces the list rather than adding to it. Each entry is
`host`, `all` — every target ginary models — or a canonical `<os>-<arch>[-<libc>]` name, and a
target named twice is built once.

**A cross build needs two things.** The artifact for another machine is that machine's *stub* — a
ginary of this exact version, cross-compiled — with the payload appended to it, and the payload
holds a BEAM runtime for that machine too. The stub half is "Stubs" below; the runtime half is
"Cross-building" below, and a target other than the host has to say which of them it wants with
`[tools.ginary.target.<name>] erts = ...`: `catalog` to take it out of the prebuilt-OTP catalog,
`dir:PATH` for a runtime root somebody unpacked, or `tarball:PATH` for an archive of one. A build
that is not told refuses and quotes the table to write. An `erts` of `docker:` parses now and is
refused at build time, naming the container-image milestone it arrives with — the same milestone
`ginary doctor` prints for it, because both read it off the same value. A relative `dir:` or
`tarball:` path is relative to the project, as `vm_args` and `sys_config` are. An entry of
`targets` that names no target is refused where the manifest is read, so `ginary doctor` reports
the list rather than printing a row for a build nobody can run.

Whatever a source claims, the bundled `beam.smp` itself is read: the target, the linkage and the
minimum glibc come out of that file, and a runtime for another target fails the build naming
both. What was found is recorded in the artifact's manifest as the `otp` block —
`{linkage, libc: {kind, min}, nif_loading, source}` — which `ginary inspect` prints and
`docs/format.md` specifies. `nif_loading` is `false` for a statically linked runtime, which has
no dynamic loader to load a NIF with.

Naming a target puts it in the file name, and the spelling decides that rather than the place it
was spelled. `host` selects the machine ginary is running on, which is the build a bare
`ginary build` already performs, so `--target host`, `targets = ["host"]` and naming nothing all
write `build/ginary/<app>` — the name every earlier version wrote. Naming a target — `all`, or a
canonical name, the host's own included, on the command line or in the table — writes
`build/ginary/<app>-<target>` and a `build/ginary/<app>-<target>.json` copy of the manifest
beside it, so CI can read what was built for a machine it cannot run.

`ginary doctor` prints a row per target: the source, whether this ginary can resolve it today,
and for a runtime it actually read, the linkage and the minimum libc. `resolves` answers for
today and for this machine: a source that arrives with a later milestone reads `not yet`, and so
does a runtime that is on this machine and cannot be read — the row says which root was refused
and why. A target that is not this machine and names `erts = "host"` reads `not yet` too: the
host's own emulator is for the host, and a build would refuse it.

### Stubs

A stub is the same ginary, built with `--no-default-features`: the launcher, the payload reader
and the cache, and none of the command line. It is what a cross-target artifact is made of, and
running one on its own prints what it is and which target it is for rather than a help text
there are no commands behind.

Every ginary binary — both flavors — carries a 128-byte identity marker naming the version, the
target, the payload format and the flavor, so a file that claims to be a stub can be checked
rather than trusted. `docs/format.md` specifies it.

**Stubs are version-locked.** The launcher inside a stub reads the payload this ginary writes, so
a stub built by another ginary is refused by name rather than assumed compatible. A build for a
target other than this machine looks for one in this order:

1. `--stub PATH`, which is an instruction: the file named there is used or the build fails, and
   nothing else is tried.
2. `$GINARY_STUB_DIR/ginary-stub-<version>-<target>`, then
   `$GINARY_STUB_DIR/ginary-<version>-<target>`.
3. The running executable, when the target is this machine.
4. `<cache>/stubs/<version>/<target>`, where a fetched stub will be kept.

A target with no stub anywhere is refused with every path that was tried and how to make one.
Downloading a stub from a release is not implemented; `mise run stubs:build` is how one is made.

Building them is one task, and it needs `cross` and a Docker daemon:

```console
$ mise run stubs:build          # target/stubs/ginary-stub-<version>-<target>[.exe]
$ export GINARY_STUB_DIR=$PWD/target/stubs
$ ginary build --target linux-aarch64-musl
```

It attempts five targets. The four Linux ones build today. `windows-x86_64` does **not** yet:
the launcher path is Unix-only, so the crate does not compile for `x86_64-pc-windows-gnu` — the
task keeps the target, prints `FAILED windows-x86_64` and exits non-zero rather than dropping it,
and the Windows launcher is Phase D. The two macOS stubs are not attempted at all and cannot be:
there is no macOS toolchain in a Linux container, so they come from the release build on a macOS
runner and from nowhere else.

Whatever the stub came from, it has to pass every gate before a payload is appended: exactly one
identity marker, this ginary's version, this payload format, the target that was asked for, an
ELF or PE header that agrees with the marker, and no trailer of its own — a file that already
carries a payload is an artifact, not a stub.

### Cross-building: the OTP catalog

A runtime for another machine comes out of the *catalog*: one JSON document naming, per OTP
version, per target, per variant, a `.tar.zst` with its SHA-256, its length, and the facts a
build cannot read until it has unpacked it — the linkage, whether a NIF can be loaded, the libc
floor. `docs/format.md` specifies the schema.

**The catalog is local first.** There is no hosted one and nothing is published: the pipeline is
`ginary otp repack`, and it runs here. One task builds the three Linux runtimes this
repository is tested against, into `dist/otp` (roughly 130 MB of downloads, once):

```console
$ mise run stubs:build                  # once: target/stubs/ginary-stub-<version>-<target>
$ export GINARY_STUB_DIR=$PWD/target/stubs
$ mise run otp:repack                   # once: dist/otp/catalog.json and its tarballs
$ ginary otp list --catalog dist/otp/catalog.json
version  target              variant  linkage  nifs  size      cached
29.0.5   linux-aarch64-musl  static   static   no    13.0 MiB  no
29.0.5   linux-x86_64-gnu    default  dynamic  yes   13.1 MiB  no
29.0.5   linux-x86_64-musl   static   static   no    12.5 MiB  no
```

Then point a target at it and build:

```toml
# gleam.toml
[tools.ginary.target."linux-aarch64-musl"]
erts = "catalog"
```

```console
$ GINARY_CATALOG=$PWD/dist/otp/catalog.json ginary build --target linux-aarch64-musl
$ docker run --rm --network none --platform linux/arm64 \
    -v "$PWD/build/ginary/worker-linux-aarch64-musl:/app:ro" alpine:3.20 /app
```

`--catalog PATH` and `GINARY_CATALOG` name a catalog; without either, ginary reads the one
`ginary otp update` installed at `<cache>/otp/catalog.json`, and without that the embedded one,
which is empty and says so. The five commands are `ginary otp list`, `fetch`, `path`, `update`
and `repack`; `list` and `path` never reach the network, and `path` prints where a runtime is or
says which `fetch` would put it there rather than quietly downloading forty megabytes.

**The static musl runtime cannot load a NIF.** Upstream publishes three Linux builds per
architecture and the default variant for a musl target is the fully static one, which runs on any
Linux — Alpine, a distroless image, a scratch container — because it needs no loader at all. That
is exactly why it cannot `dlopen` anything: an application whose dependencies include a NIF (a
`priv/lib/*.so`) needs the dynamic build, `otp_variant = "dynamic"` for a musl target or the
`linux-*-gnu` target, and the artifact's manifest records `nif_loading` either way so that
`ginary inspect` answers the question before a user's program does.

One runtime is fetched and extracted once, however many builds ask for it at the same time: the
fill is held under an exclusive `flock` on `<cache>/otp/.locks/<entry>/.lock`, so a second build
waits for the first rather than racing it into the same directory.

**The trust model is that nothing is taken on trust.** Every tarball is held to the `sha256` and
the `size` its catalog entry states, whether it was fetched over HTTPS or found beside the
catalog on disk; the upstream asset a repack starts from is held to the digest the release API
reported for it, and that digest is recorded in the entry. And the catalog's *claims* — the
target, the linkage, the libc — are checked against the extracted runtime's own `beam.smp` before
anything is packaged: an entry that says `linux-aarch64-musl` and unpacks to an x86-64 emulator
stops the build naming both sides. A catalog is an index, never evidence.

### The runtime settings

```toml
# gleam.toml
name = "worker"

[tools.ginary]
vm_args = "config/vm.args"
sys_config = "config/sys.config"
distribution = true
heart = true

[tools.ginary.env]
LOG_LEVEL = "info"
```

```text
# config/vm.args — flags the launcher does not own
-sname worker
-setcookie "a shared secret"
+S 4:4
```

```erlang
%% config/sys.config — one term, and it is a list
[{kernel, [{logger_level, notice}]}].
```

Building that project stages the args file at `releases/vm.args` and the configuration at
`releases/sys.config`, bundles `epmd` (for `distribution`) and `heart`, and produces an artifact
that starts its runtime with, in order:

```text
-args_file <root>/releases/vm.args
-boot <root>/bin/no_dot_erlang -noshell +B +fnu
-config <root>/releases/sys
-heart
-pa ... <erl_flags> <GINARY_ERL_FLAGS> -eval ... -extra <your arguments>
```

Four things about that vector are worth knowing.

- **The args file comes first, so ginary's own flags win.** `erl` takes the last value of a
  repeated flag. Everything in `vm.args` is a *default* the launcher's own flags override, which
  is why the file may not hold `-args_file`, `-boot`, `-extra`, `-noinput`, `-noshell`, `-pa` or
  `-pz`: the build refuses those by name, with the line they are on.
- **`-config` carries no extension.** `erl` appends `.config` itself. The file is staged as
  `releases/sys.config` and the argument names `releases/sys`. A `sys.config` that is not exactly
  one top-level term, and that term a list, fails the build with a `file:line:column`.
- **`distribution` removes `-start_epmd false` and bundles the daemon.** Without a `-name` or an
  `-sname` — in `erl_flags` or in the args file — the build warns: a distributed runtime with no
  node name is a runtime nothing can reach.
- **`env` is applied only when the caller has not set the variable**, and only after the launcher
  scrubs `ERL_LIBS`, `ERL_FLAGS`, `ERL_AFLAGS`, `ERL_ZFLAGS`, `ERL_ROOTDIR`, `ERL_EPMD_PORT` and
  the `ERL_OTP*_FLAGS` family. A name in that scrub, an `ERL_`-prefixed name, and `ROOTDIR`,
  `BINDIR`, `EMU`, `PROGNAME` or `HOME` are all refused at build time. With `heart`, the launcher
  also sets `HEART_COMMAND` to the artifact's own path and the arguments it was given — unless
  the caller exported one, which is a supervision policy ginary does not know better than.
  `heart` hands that value to `/bin/sh -c`, so any element that a shell would split or expand is
  single-quoted; an ordinary path and ordinary arguments come through as they are.

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
| `GINARY_CMD` | the artifact | Maintenance, kept out of `argv`, one of five values: `directory` prints the cache entry and creates nothing, `extract-only` extracts and prints it, `inspect` prints the manifest and geometry as JSON, `selftest` extracts, preflights and starts the runtime on a no-op `halt`, reporting each step `PASS` or `FAIL`, and `uninstall` removes every cache entry of this application that nobody is running out of, reporting what it kept and why. It removes only what the cache wrote, so an `erl_crash.dump` beside the entries survives it. |
| `GINARY_DEBUG=1` | both | One human-readable line per phase on standard error. |
| `GINARY_TRACE=<file>` | both | One JSON object per phase, appended to the file. |
| `GINARY_SUPERVISE=1` | the artifact | Spawn and wait instead of `execve`; a child killed by a signal exits `128 + signo`. |
| `GINARY_PRUNE_DAYS` | the artifact | How many days an unused cache entry of this application may live. Defaults to 14; `0` turns pruning off. A value that is not a count of days falls back to the default rather than failing a launch. |
| `GINARY_STUB_DIR` | `ginary build` | A directory of prebuilt stubs, searched for `ginary-stub-<version>-<target>` and then `ginary-<version>-<target>` before the cache. `mise run stubs:build` fills one. |
| `SOURCE_DATE_EPOCH` | `ginary build` | Pins the manifest's `created_at`, so two builds of one project produce byte-identical artifacts. |

Every run prunes its own application's stale entries as it starts, best effort and never fatal;
`ginary cache prune [--days N] [--all] [--app NAME]` does the same on demand over the whole
cache. An entry a process is running out of is never removed, whatever its age — see
[ADR 0010](docs/adr/0010-cache-locking-and-pruning.md). `GINARY_OFFLINE` is planned rather than
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

- **A cross build still has to be handed its runtime.** The stub for another target is found and
  proved — `--stub`, `$GINARY_STUB_DIR`, the cache — but the BEAM that goes into the payload is
  the one `erl` reports on the build machine, or the one `--otp-root` or a `dir:`/`tarball:`
  source names. A target other than the host with no `erts` named for it is refused, quoting the
  table to write. Prebuilt runtime downloads and the musl variants are Phase C of
  [the roadmap](docs/dev/log/).
- **Only the four Linux stubs build here.** `mise run stubs:build` attempts them and
  `windows-x86_64`; the Windows one does not compile yet, because the launcher path is Unix-only
  (Phase D), and the task says so and exits non-zero. No macOS stub can be built on Linux at all;
  the two of them come from the release build on a macOS runner.
- **glibc, dynamically linked.** A host-OTP artifact needs the C library of the machine it was
  built on, or newer. The `needs:` line every build prints is the exact list — for the OTP 29.0.5
  runtime this repository is developed against it is `libc.so.6`, `libgcc_s.so.1`, `libm.so.6`,
  `libstdc++.so.6` and `libtinfo.so.6`, with a `GLIBC_2.38` floor. An artifact built on Ubuntu
  24.04 will not start on Debian 12 (glibc 2.36), and ginary says so at build time rather than
  leaving the user's loader to.
- Erlang target only. The Gleam JavaScript target is out of scope.
- The BEAM is bundled, not embedded. The runtime is extracted to a per-user cache directory on
  first run; ginary does not link the emulator into the executable.
- Hot code upgrades are not supported. `releases/` is not shipped and `release_handler` is not
  available.
- Native code (NIFs, port programs) must match the target being packaged. `ginary verify` says
  when it does not, but the build does not yet refuse it.
- Artifacts are not small. A trimmed runtime plus a small application is roughly 7.5 MB.

## Documentation

- [docs/format.md](docs/format.md) — the payload trailer and manifest specification.
- [docs/dev/architecture.md](docs/dev/architecture.md) — module map and data flow.
- [docs/dev/testing.md](docs/dev/testing.md) — test infrastructure and toolchain gating.
- [docs/dev/debugging.md](docs/dev/debugging.md) — diagnostic environment variables.
- [docs/dev/formal.md](docs/dev/formal.md) — the TLA+ model of the cache protocol.
- [docs/adr/](docs/adr/) — architecture decision records.
- [CONTRIBUTING.md](CONTRIBUTING.md) — the TDD workflow and the local gates.

## Licence

Dual licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
