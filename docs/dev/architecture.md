<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Architecture

## One binary, two modes

ginary ships as a single executable. `main()` looks at the end of its own file before it does
anything else:

```
main()
  selfexe::open_self()          /proc/self/exe, falling back to current_exe()
  trailer::read_from(file)
    Err(_)   -> exit 121 / 122   a broken artifact never becomes the CLI
    Ok(None) -> cli::run()       no trailer: this copy is the build tool
    Ok(Some) -> launcher::run()  a trailer: this copy is a packaged application
```

The consequence for every module: nothing on the launcher path may depend on clap, on the
network, or on anything only the builder needs. The launcher must start fast, must never panic,
and must never look at `argv`.

## Module map

Modules marked *(A0)* or *(A1a)* exist; the rest are the plan.

```
build side
  config.rs        [tools.ginary] in gleam.toml, merged with CLI flags
  gleam.rs         runs `gleam export erlang-shipment`, enumerates the output
  otp.rs           (A1a) discovers the host OTP root, release, ERTS version
  erts_source.rs   host | directory | tarball | catalogue | docker
  catalog.rs       the signed prebuilt-OTP catalogue
  download.rs      HTTPS fetch with checksum, retry and atomic rename
  appfile.rs       (A1a) a subset of Erlang terms, enough to read a .app file
  closure.rs       transitive closure of `applications` -> AppSet
  native.rs        detects ELF/Mach-O/PE under priv/, matches them to the target
  assemble.rs      builds the staging root
  strip.rs         `strip` on ELF, `beam_lib:strip_release` on .beam
  report.rs        size and dependency accounting
  manifest.rs      ginary.json
  payload.rs       deterministic tar + zstd; safe unpack
  trailer.rs       the 64-byte trailer
  stub.rs          finds and validates the target's ginary binary
  sign_macos.rs    Mach-O section injection and ad-hoc signing
  verify.rs        re-hash, list dynamic dependencies, report issues
  bundle.rs        orchestrates the above

launcher side
  selfexe.rs       opens the running executable by inode
  cache.rs         resolve, clean, extract, rename
  launch.rs        builds the LaunchPlan (argv and env)
  launcher.rs      the launcher-mode entry point
  diag.rs          phase timing, GINARY_DEBUG, GINARY_TRACE

shared
  target.rs        (A0) <os>-<arch>[-<libc>]
  cache_dir.rs     (A0) GINARY_CACHE_DIR > XDG_CACHE_HOME > HOME
  doctor.rs        (A0) toolchain and environment probing
  cli.rs           (A0) clap definitions and dispatch
  process.rs       (A1a) PATH search and a child process under a timeout
  error.rs         exit-code mapping
```

## Build data flow

```
gleam.toml [tools.ginary]        CLI flags
        \                       /
         config::Config <------'
                |
                v
  gleam export erlang-shipment ---> build/erlang-shipment/<app>/{ebin,priv}
                |                            |
                v                            v
  erts_source::resolve -> ResolvedErts   appfile::parse_app_file
        (host | dir | tarball |               |
         catalogue | docker)                  v
                |                    closure::app_dependency_closure
                |                            |
                +-------------+--------------+
                              v
                      assemble::stage  ->  staging root
                              |
                              v
                        strip::run  (ELF + .beam)
                              |
                    +---------+---------+
                    v                   v
            manifest::write       report::measure
                    |
                    v
   payload::pack (sorted tar, deterministic headers, zstd)
                    |
                    v
   stub::resolve(target) -> copy -> append payload -> append trailer
                    |
                    v
        build/ginary/<app>-<target>[.exe]
```

`process.rs` exists because two callers need the same bounded child process and
neither may hang: `doctor` probes four tools, `otp` asks `erl` for its code root.
Its reader threads are never joined, so a grandchild that inherited the pipes
cannot outlive the caller's timeout; only the direct child is killed and reaped.
Both output streams are returned, because a program that fails writes its
diagnosis to standard error and nothing at all to standard output.
Nothing on the launcher path uses it.

`inspect_root` is the single point of truth about a runtime: whatever the ERTS came from, the
real `beam.smp` is read with the `object` crate to derive the target, the linkage and the
minimum glibc. Nothing downstream trusts the provenance metadata alone.

## Launch data flow

```
./my_gleam_app arg1 arg2
        |
        v
  selfexe::open_self  ->  trailer::read_from  ->  payload::locate
        |
        v
  read entry 0 of the payload -> manifest::Manifest
        |
        v
  cache_dir::resolve -> <cache>/<app>/<sha256[:16]>
        |
   hit? -+---- yes ---------------------------+
        |                                     |
        no                                    |
        v                                     |
  extract into .<key>.tmp-<pid>               |
  verify SHA-256, then rename() (atomic)      |
        |                                     |
        +-------------------+-----------------+
                            v
                     launch::preflight
             erlexec, beam.smp, erl_child_setup,
             inet_gethost present and executable
                            |
                            v
                     launch::plan -> LaunchPlan { program, argv, env }
                            |
                            v
                   execve (the process is replaced)
```

The rename is the completion marker: there is no partial rename, so the presence of
`<key>/ginary.json` as a regular file is the only proof that a cache entry is complete.
Concurrent first runs race on the rename, and the loser deletes its own temporary tree.

## Invariants

- The launcher never interprets the user's arguments. `--help` belongs to the application.
- The launcher never panics; every failure maps to a documented exit code between 121 and 125.
- The builder never mutates the shipment; it only reads it.
- Cache entries are immutable once renamed into place, so a running application cannot observe a
  half-written runtime.
- Identical input produces identical artifact bytes.
