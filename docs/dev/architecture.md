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

Modules marked *(A0)*, *(A1a)*, *(A1b)* or *(A1c)* exist; the rest are the plan.

```
build side
  config.rs        [tools.ginary] in gleam.toml, merged with CLI flags
  gleam.rs         runs `gleam export erlang-shipment`, enumerates the output
  otp.rs           (A1a) discovers the host OTP root, release, ERTS version
  erts_source.rs   host | directory | tarball | catalogue | docker
  catalog.rs       the signed prebuilt-OTP catalogue
  download.rs      HTTPS fetch with checksum, retry and atomic rename
  appfile.rs       (A1a) a subset of Erlang terms, enough to read a .app file
  closure.rs       (A1b) transitive closure of `applications` -> AppSet
  native.rs        detects ELF/Mach-O/PE under priv/, matches them to the target
  assemble.rs      (A1c) builds the staging root
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

## The application closure

`closure::app_dependency_closure(shipment, otp_lib, roots, extra)` answers the only question
assembly asks: which applications go into the artifact, and where is each one read from. It is a
worklist over `.app` files, seeded with the roots the caller named, the extras the configuration
added, and `kernel` and `stdlib`, which are seeds unconditionally because a BEAM that cannot boot
them is not a runtime.

```
seeds = roots (Root) | extra (Extra) | kernel, stdlib (Always)
        |
        v
  pop a name
        |
  not a directory name? -> InvalidAppName { name, chain from a seed }
        |
  resolve: 1. <shipment>/<name>/ebin/<name>.app
           2. <otp_lib>/<name>-<vsn>, <vsn> matching ^[0-9]+(\.[0-9]+)*$
        |
   both? -> shipment wins, warning names every OTP directory ignored
   neither? -> AppNotFound { name, chain from a seed, searched }
   two OTP versions, no shipment copy? -> AmbiguousOtpApp { name, candidates }
        |
        v
  push `applications` and `included_applications` (required)
  push `optional_applications` only if they resolve, else record in skipped_optional
```

The resolution order decides more than which copy wins. The shipment is probed
first and a shipment hit ends the lookup, so the OTP candidates are then only the
material of a warning: a `lib` holding both `crypto-5.9.2` and `crypto-5.9.3`
cannot fail a build whose `crypto` comes from the shipment and which would never
have read either directory. The same reasoning covers an optional edge, which is
a probe rather than a requirement: an ambiguous or unusable name there is
recorded in `skipped_optional` with a warning saying why, never raised. The
ambiguity is an error exactly where it decides something — a required
application that has to come from the OTP library.

Application names come out of `.app` files, which ginary does not write, and
every lookup interpolates one into a path. A name that is empty or holds `/`,
`\`, `..` or a NUL byte is refused before any path is built from it, so a
`{applications, ['../../escape']}` cannot make the closure stat — much less
hand assembly an `ebin` — outside the shipment and the OTP library.

Three properties are the point of the module, and each has tests that would fail without it:

- **Determinism.** The applications live in a `BTreeMap`, the OTP `lib` directory is listed once
  and its candidates sorted, the worklist pops names in name order rather than in discovery
  order, and `requested_by` is collected in a `BTreeSet` and filled in only after the walk, so a
  requester found late is not missed. Permuting `roots`, permuting `extra`, or reading the
  directories back in a different order all produce the same `AppSet`, down to the JSON bytes.
- **Termination.** A name is resolved once. Cycles and self-references are therefore ordinary, not
  special cases.
- **No silent skipping.** A missing required application is an error carrying the full chain from
  a seed and the paths that were searched; a missing *optional* application is recorded in
  `skipped_optional` and reported, and one skipped for any reason a reader could not guess —
  an ambiguous OTP library, a name that is not a name — says which reason in `warnings`. Nothing
  is dropped, and no error layer repeats what its own `source()` already says, because
  `src/main.rs` prints one line per link of the chain.

`AppSet::chain` walks `requested_by` backwards, breadth first, to the nearest seed, so
`ginary closure --explain` can say why an application is in an artifact in the shortest terms
available rather than in whatever order the worklist happened to find it.

`ginary closure <shipment> --root NAME` is the developer window onto all of this, the same tool
`ginary build --explain` will use later. `--root` is required: there is no reliable way to guess
which application of a shipment is the one being packaged, and guessing wrong would silently
bundle the wrong closure.

`inspect_root` is the single point of truth about a runtime: whatever the ERTS came from, the
real `beam.smp` is read with the `object` crate to derive the target, the linkage and the
minimum glibc. Nothing downstream trusts the provenance metadata alone.

## The staging root

`assemble::stage(&AppSet, &OtpInfo, &StageOptions, out)` turns that bill of materials into a
directory. What it writes is the exact tree the payload is made of and the exact tree the
launcher will find in its cache, so every later phase — strip, report, pack, extract, launch —
reads this layout and nothing else.

```
<out>/
  bin/no_dot_erlang.boot         the only boot script
  erts-<vsn>/bin/                beam.smp erlexec erl_child_setup inet_gethost
                                 plus whatever --extra-bin named (heart, epmd)
  lib/<name>-<vsn>/{ebin,priv}   an application from the OTP library
  lib/<name>/{ebin,priv}         an application from the shipment
  ginary.stage.json              what was staged, why, and how big it is
```

The two `lib` shapes are not cosmetic. An OTP application keeps its version in its directory
name because that is where `code:lib_dir/1` looks for it; a shipment application does not have
one, because that is what `gleam export erlang-shipment` writes and what the launcher's `-pa`
names. Assembly preserves the difference rather than normalising it.

Four rules shape the tree, and each is a test in `tests/assemble.rs` that fails without it.

**The result is atomic.** Staging happens in a sibling `<out>.tmp-<pid>` and is renamed onto
`out` at the end. `out` therefore either does not exist or is complete, and a failure leaves
neither a partial `out` nor a temporary tree — the same completion-marker discipline the cache
uses at the other end of the pipeline.

**Nothing is copied by default.** Under `erts-<vsn>/bin` only `otp::REQUIRED_ERTS_BINARIES` and
the names in `StageOptions::extra_bins` are staged; every other program that was there is
recorded, with a one-line reason, in `StagedRoot::excluded_erts_bins()`. Under an application
only `ebin` and `priv` are staged, `*.appup` is dropped, and `src`, `include`, `doc`,
`examples`, `c_src` and `mibs` never travel — they are left behind by not being on the
allowlist, not by a filter, so nothing *inside* `ebin` or `priv` is pruned by name and
`snmp`'s runtime `priv/mibs/*.bin` survives. Symlinks are dereferenced, under two boundaries
rather than one: a link to a *file* may point anywhere inside the application, and a link to a
*directory* may not leave the `ebin` or `priv` it was found in, or `ebin/x -> ../src` would walk
around a structural rule. A link that crosses its boundary or points at nothing is
`UnsafeSymlink`, one that points back at a directory containing it is `SymlinkCycle`, and both
are checked on the `ebin` and `priv` themselves as well as on everything under them — an
application whose `priv` *is* a link to elsewhere on the build machine is the same defect, and
`read_dir` follows it without a word. A file whose name is not valid UTF-8 is `NonUtf8Name`: the
listing is text, and an artifact holding a file its own index cannot name is worse than one that
was not built.

**The boot file is checked against the tree.** `no_dot_erlang.boot` hardcodes the `kernel` and
`stdlib` versions it was generated against, as literal `$ROOT/lib/<name>-<vsn>/ebin` byte
strings that `otp::boot_lib_dirs` reads out. If the staged tree does not hold exactly those
directories the runtime fails to boot without saying why, so
`AssembleError::BootReferencesMissingApp` says it here, naming both the version the boot file
wants and the version the closure resolved.

**The tree describes itself.** `ginary.stage.json` lists every file with its size, its mode and
its category — `erts_binary`, `boot`, `otp_beam`, `gleam_beam`, `priv`, `app_resource`, `other` —
sorted by path. It is the precursor of the artifact's `ginary.index.json` and the input to the
size report. It holds no absolute path and no timestamp, and it does not list itself, because a
file whose contents depend on its own length cannot be reproduced. Staging the same inputs twice
therefore produces byte-identical trees, which is the first place the "identical input produces
identical artifact bytes" invariant is actually testable.

Junk removal is the one place assembly deletes rather than declines to copy:
`crypto-*/priv/lib/otp_test_engine.so`, an application's `priv/obj/` directory and
`priv/lib/*.a` are removed after the copy, and each removal is recorded in
`StagedRoot::junk_removed` with its size, so `--keep-junk` and the report can both say exactly
what the default cost.

`ginary stage <shipment> --root NAME --out DIR` is the developer window onto all of this, and
the input to `tests/stage_run.rs`, which boots what it wrote.

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
