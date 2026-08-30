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

**Pre-alpha. Nothing usable ships yet.** The current version implements only `ginary version`
and `ginary doctor`. There is no `build` command, no payload format implementation and no
launcher. See [CHANGELOG.md](CHANGELOG.md) for what has actually landed and
[docs/dev/log/](docs/dev/log/) for the per-milestone record.

## Planned quickstart

Once `build` exists the workflow is intended to be:

```console
$ cd my_gleam_app
$ ginary build
   packaged my_gleam_app 8.4 MB -> build/ginary/my_gleam_app
$ ./build/ginary/my_gleam_app --help
```

and on a machine with no Erlang at all:

```console
$ command -v erl || echo "no erlang here"
no erlang here
$ ./my_gleam_app arg1 arg2
```

Configuration will live in the project's `gleam.toml` under `[tools.ginary]`, which the Gleam
compiler ignores.

## How it works

```
ginary build
  gleam export erlang-shipment      the application and its hex dependencies
  + host or catalogue OTP root      erts-<vsn>, kernel, stdlib, and whatever the
                                    .app dependency closure actually needs
  -> staging root -> strip -> tar (deterministic) + zstd -> payload
  -> copy of the ginary binary for the target + payload + 64-byte trailer
  -> build/ginary/<app>-<target>

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

- Erlang target only. The Gleam JavaScript target is out of scope.
- The BEAM is bundled, not embedded. The runtime is extracted to a per-user cache directory on
  first run; ginary does not link the emulator into the executable.
- Hot code upgrades are not supported. `releases/` is not shipped and `release_handler` is not
  available.
- Native code (NIFs, port programs) must match the target being packaged. Cross-packaging an
  application with NIFs requires either a prebuilt artifact or a build hook.
- Artifacts are not small. A trimmed runtime plus a small application is roughly 8 MB
  compressed.
- The bundled Linux runtime is dynamically linked against glibc unless a musl variant is
  selected, so it has a minimum glibc version.

## Documentation

- [docs/format.md](docs/format.md) — the payload trailer and manifest specification.
- [docs/dev/architecture.md](docs/dev/architecture.md) — module map and data flow.
- [docs/dev/testing.md](docs/dev/testing.md) — test infrastructure and toolchain gating.
- [docs/dev/debugging.md](docs/dev/debugging.md) — diagnostic environment variables.
- [docs/adr/](docs/adr/) — architecture decision records.
- [CONTRIBUTING.md](CONTRIBUTING.md) — the TDD workflow and the local gates.

## Licence

Dual licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
