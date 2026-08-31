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

Modules marked *(A0)*, *(A1a)*, *(A1b)*, *(A1c)*, *(A2)*, *(A3a)*, *(A3b)* or *(A4)* exist; the
rest are the plan.

```
build side
  config.rs        (A4) [tools.ginary] in gleam.toml, merged with CLI flags
  gleam.rs         (A4) runs `gleam export erlang-shipment`
  otp.rs           (A1a) discovers the host OTP root, release, ERTS version
  erts_source.rs   host | directory | tarball | catalogue | docker
  catalog.rs       the signed prebuilt-OTP catalogue
  download.rs      HTTPS fetch with checksum, retry and atomic rename
  appfile.rs       (A1a) a subset of Erlang terms, enough to read a .app file
  closure.rs       (A1b) transitive closure of `applications` -> AppSet
  native.rs        detects ELF/Mach-O/PE under priv/, matches them to the target
  assemble.rs      (A1c) builds the staging root
  beam.rs          (A2) the chunk table of a compiled module
  elf.rs           (A2) read-only inspection of a native binary
  strip.rs         (A2) `strip` on ELF, `beam_lib:strip_files` on .beam
  report.rs        (A2) size and dependency accounting
  manifest.rs      (A3a) ginary.json and ginary.index.json
  payload.rs       (A3a) deterministic tar + zstd; safe unpack
  trailer.rs       (A3a) the 64-byte trailer
  stub.rs          finds and validates the target's ginary binary
  sign_macos.rs    Mach-O section injection and ad-hoc signing
  verify.rs        re-hash, list dynamic dependencies, report issues
  bundle.rs        (A4) orchestrates the above
  inspect.rs       (A4) reads a packaged application from the outside

launcher side
  selfexe.rs       (A3b) opens the running executable by inode
  cache.rs         (A3b) resolve, sweep, extract, rename, clean
  launch.rs        (A3b) builds the LaunchPlan (argv and env), execs or supervises
  launcher.rs      (A3b) the launcher-mode entry point and GINARY_CMD
  diag.rs          (A3a) phase timing, GINARY_DEBUG, GINARY_TRACE
  fault.rs         (A3b) named fault points, `fault-injection` feature only

shared
  target.rs        (A0) <os>-<arch>[-<libc>]
  cache_dir.rs     (A0) the build side's view of `cache::resolve`
  doctor.rs        (A0) toolchain and environment probing
  cli.rs           (A0) clap definitions and dispatch
  process.rs       (A1a) PATH search and a child process under a timeout
  error.rs         (A3b) LauncherError and the exit codes 121 to 125
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
                       strip::strip  (ELF + .beam)
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

That is the finished shape. A4 implements the host half of it — `erts_source::resolve` is
`otp::discover` and `stub::resolve` is the running executable, so the artifact is
`build/ginary/<app>` with no target suffix; see "One build, end to end" below.

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
`UnsafeSymlink`, and one that points back at a directory containing it is `SymlinkCycle`. Both
are checked on the `ebin` and `priv` themselves as well as on everything under them — an
application whose `priv` *is* a link to elsewhere on the build machine is the same defect, and
`read_dir` follows it without a word. An `ebin` or `priv` that is itself a link has no
enclosing subtree to be held to, so the second boundary reaches it as the exclusion itself:
a link resolving to one of the excluded directories, or to anything inside one, is
`ExcludedSymlinkTarget`, because `priv -> src` staging the sources under another name is the
same silent leak as `priv/x -> ../src` and only the name on the door differs. A file whose name
is not valid UTF-8 is `NonUtf8Name`: the listing is text, and an artifact holding a file its own
index cannot name is worse than one that was not built.

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

The listing stops describing the tree the moment a later phase rewrites a byte of it, so
`StagedRoot::refresh` re-stats every file it lists, recomputes the per-application totals and
writes `ginary.stage.json` again. What it carries over untouched is the account that *cannot* be
re-derived from a directory — the excluded ERTS binaries, the junk that was removed, the boot
references that were checked — because those are records of what staging decided, and a refresh
that dropped them would lose `--explain` for good.

`ginary stage <shipment> --root NAME --out DIR` is the developer window onto all of this, and
the input to `tests/stage_run.rs`, which boots what it wrote.

## Stripping the staged root

`strip::strip(root, &OtpInfo, &StripOptions)` is the one phase that *changes* the tree assembly
wrote, and it is two unrelated halves because a staged root holds two unrelated kinds of binary.

```
staged root
   |
   +-- every file whose first four bytes are \x7fELF
   |     strip --strip-all       anything else
   |     strip --strip-unneeded  a shared object (ET_DYN named *.so, or with no PT_INTERP)
   |     then elf::inspect again: same class, same machine, or the file was destroyed
   |     one that starts like an ELF and will not parse is a reported skip, not a failure
   |
   +-- <otp.root>/bin/erl -noshell -env ERL_CRASH_DUMP /dev/null
   |     -eval 'beam_lib:strip_files(Files)' -extra <every staged .beam>
   |     then beam::chunks on every staged .beam: no Dbgi, no Docs, still a Code
   |
   v
StagedRoot::refresh  ->  ginary.stage.json rewritten with the sizes the tree now has
   |
   v
report::measure(before, strip_report, root)  ->  SizeReport
```

Four decisions shape it, and each has tests that would fail without it.

**Files are found by their magic, never by their name.** A NIF under `priv/lib` is not required
to be called `.so` and a `.so` is not required to be a NIF; a `priv/lib/x.so` that is really a
shell script must not be handed to a binary tool. `elf::is_elf` reads four bytes and answers.
Which arguments the tool then gets is decided by `e_type`: `ElfInfo::kind` being
`ElfKind::SharedObject` is necessary, because a program is not a library, and the name or an
absent `PT_INTERP` decides the rest — a position-independent *executable* is an `ET_DYN` too and
must still be stripped all the way, while glibc's own `libc.so.6` shows that a real library may
carry an interpreter. A file that begins with the magic and is not a whole ELF is neither
stripped nor fatal: it is a `warning:` line under the strip table, which is the same decision
`report::measure` reaches about the same file.

**Neither tool is trusted to have done what it said.** `strip --strip-all` on a shared object
produces a file that is smaller and will not load, and `beam_lib` answers
`{ok, _}` for a list it was given and changed nothing in — the two failures that are invisible
until the artifact is in somebody else's hands. So every ELF is re-inspected and has to still be
an ELF of the same class and machine, and every staged `.beam` is re-read and has to have lost
`Dbgi` and `Docs` and kept `Code`. `Line` stays on purpose: it is what turns a crash in a
packaged application into a stack trace with line numbers. ADR 0007 records the trade.

**A missing tool is a reported skip; a failing tool is an error.** `strip` is not part of the
Rust toolchain and an OTP root assembled from a tarball of ERTS binaries alone genuinely has no
`bin/erl`. Both are `ElfOutcome::Skipped` / `BeamOutcome::Skipped` with a reason naming what was
looked for, because a bigger artifact is better than a build that will not run on a machine
missing a developer tool. A tool that *runs* and fails is an error naming the file and quoting
what the tool wrote to standard error.

**The runtime is the installation's own, by absolute path, and it is given files.** `beam_lib`
lives inside OTP, so stripping modules means starting Erlang, and the release that rewrites them
has to be the release they came from. Whatever `erl` is on `PATH` is not consulted. The modules
arrive after `-extra` rather than being interpolated into the expression, so a name holding a
quote cannot become Erlang source — and they arrive as *paths* passed to
`beam_lib:strip_files/1`, not as a directory passed to `strip_release/1`, which would expand
`<root>/lib/*/ebin/*.beam` through `filelib:wildcard/1`. There the root is a glob prefix: a
staged root named `build[1]` would match nothing and one named `build*` would rewrite modules
under every sibling directory whose name starts `build`. Passing the list ginary walked also
keeps the two halves honest, since every module the report counts is one the runtime was handed,
`priv` included. A tree too large for one argument vector is stripped in several calls; see
`strip::MAX_ARGUMENT_BYTES`.

One property of `beam_lib` shows through into `src/beam.rs`: it writes every module
it rewrote through `zlib:gzip/1`, so a stripped `.beam` on disk is a gzip member wrapping the IFF
form. The code server unwraps it on the way in and so does `beam::form`, which is why the
verification, `ginary beam chunks` and the report all read a stripped module as readily as an
unstripped one. Whether ginary should instead rewrite the modules uncompressed — the payload's
own zstd does better on uncompressed input — is a measurement the plan defers.

Stripping is idempotent, and that is not incidental: `strip` over an already-stripped file and
`strip_files` over an already-stripped module both write the same bytes back, so
"identical input produces identical artifact bytes" survives this phase. `tests/strip.rs` and
`tests/stage_run.rs` each assert it, the second over a real runtime.

## The size and dependency report

`report::measure(before, strip_report, root)` answers the two questions that decide whether an
artifact is shippable, and it answers both from the tree rather than from what a tool claimed.
The strip report is an *input* to the account and never a source for it.

**How big is it, and where did the size go?** Each of `assemble::Category`'s buckets carries a
`bytes_before` read from the listing staging wrote and a `bytes_after` stat'd from the disk now,
so "the artifact is 12 MB" becomes "the ERTS binaries are 10.7 MB of it, and stripping already
took 45.9 MB off them".

**Where will it not run?** An artifact carries its own BEAM and not its own libc. Every ELF in
the tree is inspected, and the union of their `DT_NEEDED` entries with the highest `GLIBC_x.y`
any of them requires becomes the `needs:` line — the portability floor, stated at build time
rather than discovered by a user whose loader refuses the artifact. The glibc versions are
compared numerically, component by component, because sorting the strings puts `2.9` above
`2.38` and would report a floor two hundred releases too low.

Nothing in the report is fatal. A file the listing names and the tree does not hold, or one that
starts like an ELF and will not parse, is a line in `warnings` and the rest of the report is
still produced: a report that refuses to print because one file is odd is worse than one that
prints and says which file was odd.

## One build, end to end

`bundle::build(&BuildOptions, &Diag)` is the whole of `ginary build`, and every step of it is a
module above answering the one question it owns.

```
BuildOptions                        flags merged over [tools.ginary]
     |
  check_stub(/proc/self/exe)        a packaged application may not be a stub -> BundledStub
     |                              first, because the remedy is "install plain ginary" and a
     |                              build that exported and staged before saying so wasted both
     v
  gleam::export_shipment            or existing_shipment() under --skip-export
     |                              gleam's own stderr travels verbatim
     v
  otp::discover(--otp-root)
     v
  closure::app_dependency_closure   roots = [app], extra = otp_applications
     v
  assemble::stage -> <project>/build/ginary/.work-<pid>/root
     v                              force: a work directory is this build's own
  strip::strip
     |
     +-- report::measure(before = the pre-strip listing, root = the tree now)
     |
  StagedRoot::refresh               the listing now holds the sizes the tree has
     v
  manifest_for                      OtpInfo for the versions, the AppSet for the applications,
     |                              -pa with the packaged application first
     v
  NamedTempFile in the output directory
     stub bytes -> payload::pack(level) -> trailer::to_bytes
     chmod 0755 -> persist(<output>/<app>)
     v
  BuildReport                       the A2 size table, the needs: line, the artifact line
```

Two rules are this module's own, and both are about what is left behind.

**The work directory belongs to the project, never to the destination.** It is
`<project>/build/ginary/.work-<pid>/root` whatever `--out` says, so `ginary build --out
/usr/local/bin` does not stage a whole OTP installation into `/usr/local/bin`. The process id is
what keeps two concurrent builds of one project apart, and it is removed on every path out of the
build — success, failure, and the injected `GINARY_FAULT=pack:fail` that exists to test exactly
that. `--keep-staging` keeps it and prints where it is. A removal that cannot happen is never
fatal and never silent: `bundle::remove_work_dir` returns a warning line naming the directory and
what the operating system said, which a successful build carries in `BuildReport::warnings` and
prints above its `artifact:` line, and a failed one records through `Diag`.

**The artifact is published by a rename.** The whole file is written into a `NamedTempFile` *in
the output directory*, so the rename cannot cross a filesystem, and the destination is either
absent or a complete, executable artifact. A build that fails between the stub and the trailer
leaves nothing at all.

## Launch data flow

One packaged application, from `execve` to `execve`. Every arrow is a step the launcher takes
before the Gleam code runs, and every dotted branch is a numbered exit that the application
itself could never have produced.

```
$ ./my_gleam_app --name world
        |
        v
  main()
    selfexe::open_self()            /proc/self/exe first: an artifact renamed or unlinked
        |                           while it starts is still readable by inode
        |  Err ......................> 121  ginary: cannot open the running executable
        v
    trailer::read_from(&file)       the last 64 bytes
        |  Err ......................> 122  ginary: <what is wrong with the file>
        |  Ok(None) .................> cli::run()   no magic: this copy is the build tool
        v
    error::install_panic_hook()     launcher path only; a panic becomes one line and 121
        |
        v
  launcher::run(file, path, trailer)
    Diag::from_env                  GINARY_DEBUG=1 -> stderr, GINARY_TRACE=<file> -> JSONL
    Env::from_env                   one snapshot; every decision below is a pure function of it
        |
        +-- GINARY_CMD set? --------> directory     resolve only, print <cache>/<app>/<key>, 0
        |                             extract-only  extract, print the entry, 0
        |                             inspect       manifest + geometry + sha256 as JSON, 0
        |                             anything else usage on stderr, 2
        v
    payload::read_manifest          seek to payload_offset, read entry 0, stop
        |                           a format_version this build does not read is 122, not 123
        |                           Manifest::validate: `app` and every launch path is one
        |                           relative component, before either reaches a join -> 122
        v
    cache::prepare                  GINARY_CACHE_DIR > XDG_CACHE_HOME/ginary > HOME/.cache/ginary
        |                           EACCES/EROFS -> ${TMPDIR:-/tmp}/ginary-<uid> + one warning
        |                           the fallback root is mkdir 0700, or checked: a directory,
        |                           owned by this uid, that no group or other may write to
        v
    cache::ensure_extracted         the ten steps of ADR 0005
        |   1  <key>/ginary.json a regular file?  -> hit, done
        |   2  sweep .tmp-<pid>/.corrupt-<pid> whose /proc/<pid> is gone
        |   3  mkdir .<key>.tmp-<pid>            (<app> dir is 0700)
        |   4  seek -> Take(len) -> sha256 -> zstd -> tar
        |   5  refuse symlinks, hardlinks, devices, `..`, absolute paths
        |   6  digest must match the trailer   -> else remove tmp, 123
        |   7  chmod 0755 under <bindir>
        |   8  syncfs(tmp), else per-file sync_all; then fsync(<app> dir)
        |   9  rename(tmp, <key>)  EEXIST/ENOTEMPTY/EISDIR -> a concurrent process won:
        |                          verify its ginary.json, remove tmp, use its entry
        |  10  no other marker: the rename is the completion marker
        v
    launch::preflight               erlexec, beam.smp, erl_child_setup, inet_gethost present
        |                           and u+x; <boot>.boot present
        |  Err -> remove the entry, extract once more, check again
        |         still Err ........> 124  ginary: the runtime cache at <entry> is unusable: ...
        v
    launch::plan                    pure: program, argv, env set, env remove
        |                           argv: -boot -noshell +B -start_epmd false -pa... <erl_flags>
        |                                 $GINARY_ERL_FLAGS -eval -extra <the user's arguments>
        v
    Diag records the whole plan     argv and the environment difference, as JSON arrays
        |
        +-- GINARY_SUPERVISE=1 ----> launch::supervise  spawn, wait, mirror the code
        |                                               (signal -> 128 + signo)
        v
    launch::exec                    execve: this process becomes the runtime
           Err ......................> 125  ginary: cannot start <program>: ...
                                           + hint: ld-linux missing, or a noexec cache
```

The rename is the completion marker: there is no partial rename, so the presence of
`<key>/ginary.json` as a regular file is the only proof that a cache entry is complete.
Concurrent first runs race on the rename, and the loser deletes its own temporary tree.

The preflight retry is the one place the launcher guesses. An extracted tree that has lost a
file was far more likely damaged *after* extraction — a `tmpwatch`, a half-finished `rm -rf` —
than packed wrong, so the entry is removed and extracted once more. A second failure is 124 and
names the file; a third extraction would be a loop, and a loop is what a user reports as a hang.

## Invariants

- The launcher never interprets the user's arguments. `--help` belongs to the application.
- The launcher never panics; every failure maps to a documented exit code between 121 and 125.
- The builder never mutates the shipment; it only reads it.
- Cache entries are immutable once renamed into place, so a running application cannot observe a
  half-written runtime.
- Identical input produces identical artifact bytes, stripping included.
- No tool that rewrites a file in the artifact is trusted; every one of them is checked
  afterwards against what it claimed to have done.
- Every binary parser answers with a typed error rather than a panic, whatever bytes it is given.
