// SPDX-License-Identifier: MIT OR Apache-2.0
//! Building OTP-shaped directory trees in a temporary directory.
//!
//! Two builders live here, and between them they stand in for the two inputs
//! every later milestone consumes:
//!
//! - [`FakeOtp`] produces a runtime root — `bin/`, `erts-<vsn>/bin/`, `lib/`
//!   and `releases/` — complete enough that `ginary::otp::inspect_root`
//!   accepts it;
//! - [`FakeShipment`] produces the layout `gleam export erlang-shipment`
//!   writes, `<dir>/<app>/ebin/<app>.app`.
//!
//! Neither writes a byte that could be executed usefully: the ERTS binaries are
//! shell scripts that exit, and a `.beam` is the forty-eight bytes of
//! [`DUMMY_BEAM`]. What they do carry is the *structure*, which is all the
//! closure, assembly, stripping and discovery code reads. A test that needs a
//! real runtime is gated on the host toolchain instead — see
//! [`crate::common::tools::require_tools`].
//!
//! [`FakeOtp::with_erl_script`] is the one place a builder writes a program
//! that does something. `src/strip.rs` runs `<root>/bin/erl` by absolute path,
//! and a test cannot assert on the one-liner it passes to
//! `beam_lib:strip_files/1` without an `erl` that writes its own argument
//! vector down. The stub does exactly that and exits; it strips nothing, which
//! is why [`DUMMY_BEAM`] is already stripped.
//!
//! Both builders take applications through the same [`FakeApp`] description, so
//! an application can be moved between a fake OTP root and a fake shipment
//! without rewriting it. That matters for the closure tests, which turn on an
//! application being in one, the other, or both.
//!
//! Cargo runs doctests on the library only, never on `tests/`, so this example
//! is illustration rather than a test; the builders are checked against the
//! parser by `a_fake_shipment_writes_an_app_file_this_parser_reads_back` and
//! `a_fake_otp_root_writes_app_files_this_parser_reads_back` in
//! `tests/appfile.rs`.
//!
//! ```text
//! let dir = tempfile::tempdir().expect("tempdir");
//! let otp = FakeOtp::new()
//!     .erts_vsn("17.0.5")
//!     .release(29)
//!     .otp_version("29.0.5")
//!     .app("kernel", "11.0.3", &["stdlib"])
//!     .app_with("ssl", "11.7.4", |app| {
//!         app.applications(&["crypto", "public_key"])
//!             .priv_file("lib/x.so", b"..")
//!     })
//!     .build_in(&dir);
//! assert!(otp.root.join("bin/no_dot_erlang.boot").is_file());
//! ```

use std::path::{Path, PathBuf};

use crate::common::script::script;

/// The default ERTS version a [`FakeOtp`] uses, matching the host OTP 29.0.5.
pub const DEFAULT_ERTS_VSN: &str = "17.0.5";
/// The default OTP release a [`FakeOtp`] uses.
pub const DEFAULT_RELEASE: u32 = 29;
/// The default full OTP version a [`FakeOtp`] uses.
pub const DEFAULT_OTP_VERSION: &str = "29.0.5";
/// The `kernel` version a [`FakeOtp`] seeds itself with.
pub const DEFAULT_KERNEL_VSN: &str = "11.0.3";
/// The `stdlib` version a [`FakeOtp`] seeds itself with.
pub const DEFAULT_STDLIB_VSN: &str = "8.0.3";

/// A minimal, already-stripped BEAM file: `AtU8`, `Code` and `Line`, four
/// bytes of nothing each.
///
/// Structurally a BEAM and deliberately not a loadable one. Until A2 this was
/// the twelve bytes of a bare `FOR1 <size> BEAM` with no chunks at all, which
/// was enough for everything that only counted and copied files. `src/strip.rs`
/// opens them: it verifies that no staged module still holds
/// [`ginary::beam::DEBUG_INFO_CHUNK`] or [`ginary::beam::DOCS_CHUNK`] and that
/// every one still holds [`ginary::beam::CODE_CHUNK`], and a fake tree whose
/// modules have no `Code` at all could not tell a working verification from a
/// broken one.
///
/// It carries no `Dbgi` and no `Docs`, so a fake tree is what a *stripped* tree
/// looks like and running a stub `erl` over it is legitimately a no-op. A test
/// that needs a module with debug information in it writes one with
/// [`beam_bytes`], in the open, the same way a test that needs a broken OTP
/// root builds a whole one and breaks it.
pub const DUMMY_BEAM: &[u8] = b"FOR1\x00\x00\x00\x28BEAM\
AtU8\x00\x00\x00\x04\x00\x00\x00\x00\
Code\x00\x00\x00\x04\x00\x00\x00\x00\
Line\x00\x00\x00\x04\x00\x00\x00\x00";

/// Builds BEAM bytes holding exactly the chunks given, in the order given.
///
/// Real IFF padding: a chunk whose length is not a multiple of four is followed
/// by up to three zero bytes, which is the rule a reader that only added the
/// declared length would get wrong on the second chunk of any real module.
///
/// # Panics
///
/// If a chunk's data is longer than `u32::MAX`, which no test writes.
pub fn beam_bytes(chunks: &[([u8; 4], &[u8])]) -> Vec<u8> {
    let mut body: Vec<u8> = b"BEAM".to_vec();
    for (id, data) in chunks {
        body.extend_from_slice(id);
        let len = u32::try_from(data.len()).expect("a chunk shorter than 4 GiB");
        body.extend_from_slice(&len.to_be_bytes());
        body.extend_from_slice(data);
        body.resize(body.len().next_multiple_of(4), 0);
    }
    let mut bytes: Vec<u8> = b"FOR1".to_vec();
    bytes.extend_from_slice(
        &u32::try_from(body.len())
            .expect("a form shorter than 4 GiB")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&body);
    bytes
}

/// One application, described once and written into either layout.
#[derive(Clone, Debug)]
pub struct FakeApp {
    name: String,
    vsn: String,
    description: Option<String>,
    applications: Vec<String>,
    optional: Vec<String>,
    included: Vec<String>,
    modules: Vec<String>,
    registered: Vec<String>,
    mod_callback: Option<String>,
    env: Vec<(String, String)>,
    priv_files: Vec<(String, Vec<u8>)>,
}

impl FakeApp {
    /// A minimal application: one module named after it, no dependencies.
    pub fn new(name: &str, vsn: &str) -> Self {
        Self {
            name: name.to_owned(),
            vsn: vsn.to_owned(),
            description: None,
            applications: Vec::new(),
            optional: Vec::new(),
            included: Vec::new(),
            modules: vec![name.to_owned()],
            registered: Vec::new(),
            mod_callback: None,
            env: Vec::new(),
            priv_files: Vec::new(),
        }
    }

    /// Sets the `description` property.
    #[must_use]
    pub fn description(mut self, text: &str) -> Self {
        self.description = Some(text.to_owned());
        self
    }

    /// Sets the `applications` property.
    #[must_use]
    pub fn applications(mut self, names: &[&str]) -> Self {
        self.applications = owned(names);
        self
    }

    /// Sets the `optional_applications` property.
    ///
    /// Every name is also added to `applications` if it is not there already,
    /// because that is OTP's own rule: `optional_applications` marks a subset
    /// of `applications` whose absence at run time is tolerated. A builder
    /// that let the two drift would produce `.app` files no real tool writes.
    #[must_use]
    pub fn optional(mut self, names: &[&str]) -> Self {
        self.optional = owned(names);
        for name in &self.optional {
            if !self.applications.contains(name) {
                self.applications.push(name.clone());
            }
        }
        self
    }

    /// Sets the `included_applications` property.
    #[must_use]
    pub fn included(mut self, names: &[&str]) -> Self {
        self.included = owned(names);
        self
    }

    /// Sets the `modules` property, and with it the `.beam` files written.
    #[must_use]
    pub fn modules(mut self, names: &[&str]) -> Self {
        self.modules = owned(names);
        self
    }

    /// Sets the `registered` property.
    #[must_use]
    pub fn registered(mut self, names: &[&str]) -> Self {
        self.registered = owned(names);
        self
    }

    /// Adds a `{mod, {<module>, []}}` property.
    #[must_use]
    pub fn mod_callback(mut self, module: &str) -> Self {
        self.mod_callback = Some(module.to_owned());
        self
    }

    /// Adds one `env` entry, the value given as Erlang source.
    #[must_use]
    pub fn env(mut self, key: &str, erlang_value: &str) -> Self {
        self.env.push((key.to_owned(), erlang_value.to_owned()));
        self
    }

    /// Adds a file under the application's `priv/`, at a `/`-separated path.
    #[must_use]
    pub fn priv_file(mut self, relative: &str, bytes: &[u8]) -> Self {
        self.priv_files.push((relative.to_owned(), bytes.to_vec()));
        self
    }

    /// The name this application is written under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The version this application is written under.
    pub fn vsn(&self) -> &str {
        &self.vsn
    }

    /// Renders the `.app` file, as `gleam` and OTP both write it.
    ///
    /// Every name goes through [`atom`], so an application called `my-app` —
    /// which OTP itself has and which a hyphen makes a quoted atom — produces a
    /// file that parses rather than one that fails three milestones later.
    pub fn app_text(&self) -> String {
        let mut props = vec![format!("{{vsn, \"{}\"}}", self.vsn)];
        if let Some(description) = &self.description {
            props.push(format!("{{description, \"{description}\"}}"));
        }
        props.push(format!(
            "{{applications, [{}]}}",
            atom_list(&self.applications)
        ));
        if !self.optional.is_empty() {
            props.push(format!(
                "{{optional_applications, [{}]}}",
                atom_list(&self.optional)
            ));
        }
        if !self.included.is_empty() {
            props.push(format!(
                "{{included_applications, [{}]}}",
                atom_list(&self.included)
            ));
        }
        props.push(format!("{{modules, [{}]}}", atom_list(&self.modules)));
        props.push(format!("{{registered, [{}]}}", atom_list(&self.registered)));
        if let Some(module) = &self.mod_callback {
            props.push(format!("{{mod, {{{}, []}}}}", atom(module)));
        }
        if !self.env.is_empty() {
            let entries: Vec<String> = self
                .env
                .iter()
                .map(|(key, value)| format!("{{{}, {value}}}", atom(key)))
                .collect();
            props.push(format!("{{env, [{}]}}", entries.join(", ")));
        }
        format!(
            "{{application, {}, [\n    {}\n]}}.\n",
            atom(&self.name),
            props.join(",\n    ")
        )
    }

    /// Writes `ebin/` and `priv/` under `dir`, which must already exist.
    fn write_into(&self, dir: &Path) {
        let ebin = dir.join("ebin");
        create_dir_all(&ebin);
        write(
            &ebin.join(format!("{}.app", self.name)),
            self.app_text().as_bytes(),
        );
        for module in &self.modules {
            write(&ebin.join(format!("{module}.beam")), DUMMY_BEAM);
        }
        for (relative, bytes) in &self.priv_files {
            let path = dir.join("priv").join(relative);
            if let Some(parent) = path.parent() {
                create_dir_all(parent);
            }
            write(&path, bytes);
        }
    }
}

/// A fake OTP installation root.
#[derive(Clone, Debug)]
pub struct FakeOtp {
    erts_vsn: String,
    release: u32,
    otp_version: Option<String>,
    start_erl_data: bool,
    extra_erts_bins: Vec<String>,
    erl_script: Option<ErlScript>,
    apps: Vec<FakeApp>,
}

/// Which stub `bin/erl` a [`FakeOtp`] writes, if any.
#[derive(Clone, Debug)]
enum ErlScript {
    /// Records its arguments and exits 0.
    Succeeding,
    /// Records its arguments, replaces every `.beam` it is given with
    /// [`SHRUNKEN_BEAM`], and exits 0.
    Shrinking,
    /// Records its arguments, writes the held text to standard error, exits 1.
    Failing(String),
}

/// The module a [`FakeOtp::with_shrinking_erl_script`] stub writes over every
/// module it is handed.
///
/// Smaller than [`DUMMY_BEAM`] and still a module that passes the verification
/// stripping does: it holds `Code` and neither `Dbgi` nor `Docs`. The point is
/// the *size*, so that a test can tell a listing that was refreshed after
/// stripping from one that was not.
fn shrunken_beam() -> Vec<u8> {
    beam_bytes(&[(ginary::beam::CODE_CHUNK, b"x".as_slice())])
}

impl Default for FakeOtp {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeOtp {
    /// A root that [`ginary::otp::inspect_root`] accepts as it stands.
    ///
    /// It is seeded with `kernel` and `stdlib`, because a root without them is
    /// not an OTP installation and every test would otherwise have to add them.
    /// Adding an application of the same name replaces the seed.
    pub fn new() -> Self {
        Self {
            erts_vsn: DEFAULT_ERTS_VSN.to_owned(),
            release: DEFAULT_RELEASE,
            otp_version: Some(DEFAULT_OTP_VERSION.to_owned()),
            start_erl_data: true,
            extra_erts_bins: Vec::new(),
            erl_script: None,
            apps: vec![
                FakeApp::new("kernel", DEFAULT_KERNEL_VSN).mod_callback("kernel"),
                FakeApp::new("stdlib", DEFAULT_STDLIB_VSN).applications(&["kernel"]),
            ],
        }
    }

    /// Sets the ERTS version, and with it the `erts-<vsn>` directory name.
    #[must_use]
    pub fn erts_vsn(mut self, vsn: &str) -> Self {
        self.erts_vsn = vsn.to_owned();
        self
    }

    /// Sets the OTP release, and with it the `releases/<release>` directory.
    #[must_use]
    pub fn release(mut self, release: u32) -> Self {
        self.release = release;
        self
    }

    /// Sets the contents of `releases/<release>/OTP_VERSION`.
    #[must_use]
    pub fn otp_version(mut self, version: &str) -> Self {
        self.otp_version = Some(version.to_owned());
        self
    }

    /// Omits `releases/<release>/OTP_VERSION` altogether.
    #[must_use]
    pub fn without_otp_version(mut self) -> Self {
        self.otp_version = None;
        self
    }

    /// Adds programs to `erts-<vsn>/bin` beyond the four required ones.
    ///
    /// A real ERTS `bin` holds a dozen more — `erl`, `erlc`, `escript`,
    /// `heart`, `epmd`, `run_erl` — and assembly's whole job there is to copy
    /// four of them and refuse the rest. A root that holds only the four
    /// cannot show that.
    #[must_use]
    pub fn extra_erts_bins(mut self, names: &[&str]) -> Self {
        self.extra_erts_bins = owned(names);
        self
    }

    /// Omits `releases/start_erl.data`, leaving the release to be inferred.
    #[must_use]
    pub fn without_start_erl_data(mut self) -> Self {
        self.start_erl_data = false;
        self
    }

    /// Installs a stub `bin/erl` that records its argument vector and exits 0.
    ///
    /// `src/strip.rs` runs the OTP installation's own `erl` rather than
    /// whatever is on `PATH`, so this is how a test gets a runtime that can be
    /// *called* without an Erlang being installed. Every argument lands on its
    /// own line in `<root>/bin/erl.argv`, which
    /// [`FakeOtpRoot::erl_argv`] reads back, so a test asserts on the exact
    /// one-liner rather than on a substring of it.
    ///
    /// The stub strips nothing. That is the point: [`DUMMY_BEAM`] is already
    /// stripped, so a no-op `erl` leaves a tree that passes the verification
    /// stripping does afterwards, and a test that wants the verification to
    /// *fail* writes a module holding `Dbgi` into the staged tree itself.
    #[must_use]
    pub fn with_erl_script(mut self) -> Self {
        self.erl_script = Some(ErlScript::Succeeding);
        self
    }

    /// Installs a stub `bin/erl` that records its argument vector, replaces
    /// every `.beam` named after `-extra` with a smaller module, and exits 0.
    ///
    /// The other half of [`FakeOtp::with_erl_script`]. That stub changes no
    /// bytes, which is what makes an already-stripped tree pass verification;
    /// this one changes every module it is given, which is what makes a test
    /// about *sizes* — `ginary.stage.json` being rewritten after stripping —
    /// mean something. The replacement holds `Code` and no debug information,
    /// so the verification that follows still passes.
    #[must_use]
    pub fn with_shrinking_erl_script(mut self) -> Self {
        self.erl_script = Some(ErlScript::Shrinking);
        self
    }

    /// Installs a stub `bin/erl` that records its argument vector, writes
    /// `stderr` to standard error and exits 1.
    ///
    /// What a failing `beam_lib:strip_files/1` looks like from the outside: an
    /// Erlang term on standard error and a non-zero status. The reason a test
    /// needs it is that ginary must quote the term rather than swallow it.
    ///
    /// `stderr` may hold anything, apostrophes included — a `~p` of a quoted
    /// atom such as `'Elixir.Foo'` is exactly the shape a real term takes —
    /// because the text travels in a file beside the stub rather than inside
    /// the shell source.
    #[must_use]
    pub fn with_failing_erl_script(mut self, stderr: &str) -> Self {
        self.erl_script = Some(ErlScript::Failing(stderr.to_owned()));
        self
    }

    /// Adds an application with the given dependencies.
    #[must_use]
    pub fn app(self, name: &str, vsn: &str, applications: &[&str]) -> Self {
        self.push(FakeApp::new(name, vsn).applications(applications))
    }

    /// Adds an application described by a closure.
    #[must_use]
    pub fn app_with(self, name: &str, vsn: &str, build: impl FnOnce(FakeApp) -> FakeApp) -> Self {
        self.push(build(FakeApp::new(name, vsn)))
    }

    /// Replaces any application of the same name, keeping its position.
    fn push(mut self, app: FakeApp) -> Self {
        match self.apps.iter().position(|held| held.name == app.name) {
            Some(index) => self.apps[index] = app,
            None => self.apps.push(app),
        }
        self
    }

    /// Writes the tree under `dir`, which must already exist.
    pub fn build_in(self, dir: impl AsRef<Path>) -> FakeOtpRoot {
        let root = dir.as_ref().to_path_buf();

        let erts_bin = root.join(format!("erts-{}", self.erts_vsn)).join("bin");
        create_dir_all(&erts_bin);
        let bins = ginary::otp::REQUIRED_ERTS_BINARIES
            .iter()
            .map(|name| (*name).to_owned())
            .chain(self.extra_erts_bins.iter().cloned());
        for name in bins {
            write_executable(
                &erts_bin.join(&name),
                format!("#!/bin/sh\n# fake {name} written by tests/common/fake_otp.rs\nexit 0\n")
                    .as_bytes(),
            );
        }

        let lib = root.join("lib");
        create_dir_all(&lib);
        for app in &self.apps {
            let dir = lib.join(format!("{}-{}", app.name, app.vsn));
            create_dir_all(&dir);
            app.write_into(&dir);
        }

        create_dir_all(&root.join("bin"));
        write(&root.join("bin/no_dot_erlang.boot"), &self.boot_bytes());
        if let Some(kind) = &self.erl_script {
            let tail = match kind {
                ErlScript::Succeeding => "exit 0".to_owned(),
                ErlScript::Shrinking => {
                    write(&root.join("bin/erl.module"), &shrunken_beam());
                    "for arg in \"$@\"; do\n\
                     \x20 case \"$arg\" in *.beam) cp \"$0.module\" \"$arg\" ;; esac\n\
                     done\nexit 0"
                        .to_owned()
                }
                // The term travels in a file rather than in the script's own
                // source: an Erlang term printed with `~p` may hold an
                // apostrophe, and interpolating one into a single-quoted shell
                // string writes a stub that fails to parse instead of a stub
                // that fails on purpose.
                ErlScript::Failing(stderr) => {
                    write(&root.join("bin/erl.stderr"), stderr.as_bytes());
                    "cat \"$0.stderr\" >&2\nprintf '\\n' >&2\nexit 1".to_owned()
                }
            };
            // Through `script::script` rather than `write_executable`, because
            // this is the one stub a test actually execs and that helper is
            // what waits out the ETXTBSY window a sibling thread's fork opens.
            script(
                &root.join("bin"),
                "erl",
                &format!(
                    ": > \"$0.argv\"\n\
                     for arg in \"$@\"; do printf '%s\\n' \"$arg\" >> \"$0.argv\"; done\n\
                     {tail}"
                ),
            );
        }

        let releases = root.join("releases");
        create_dir_all(&releases.join(self.release.to_string()));
        if self.start_erl_data {
            write(
                &releases.join("start_erl.data"),
                format!("{} {}\n", self.erts_vsn, self.release).as_bytes(),
            );
        }
        if let Some(version) = &self.otp_version {
            write(
                &releases.join(self.release.to_string()).join("OTP_VERSION"),
                format!("{version}\n").as_bytes(),
            );
        }

        FakeOtpRoot {
            root,
            erts_vsn: self.erts_vsn,
            release: self.release,
        }
    }

    /// The bytes written to `bin/no_dot_erlang.boot`.
    ///
    /// A real boot script is `term_to_binary` output whose library paths appear
    /// as `$ROOT/lib/<name>-<vsn>/ebin` byte strings. Only `kernel` and
    /// `stdlib` are named, which is what `no_dot_erlang.boot` itself does, and
    /// the surrounding bytes are noise on purpose: a scanner that needs a
    /// well-formed term would pass here and fail on the real file.
    fn boot_bytes(&self) -> Vec<u8> {
        let dirs: Vec<String> = ["kernel", "stdlib"]
            .into_iter()
            .filter_map(|name| self.apps.iter().find(|app| app.name == name))
            .map(|app| format!("{}-{}", app.name, app.vsn))
            .collect();
        boot_bytes_for(&dirs.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

/// Boot-file bytes naming exactly the `<name>-<vsn>` directories given.
///
/// [`FakeOtp`] uses it for the boot file it writes, where the versions always
/// agree with the ones under `lib/`. A test proving that assembly *checks* the
/// agreement writes its own boot file over that one, naming a version the
/// library does not hold — which is the shape of the real failure, a boot file
/// carried over from another OTP installation.
pub fn boot_bytes_for(dirs: &[&str]) -> Vec<u8> {
    let mut bytes = vec![0x83, 0x68, 0x03, 0x64, 0x00, 0x06];
    bytes.extend_from_slice(b"script");
    for dir in dirs {
        bytes.push(0x6b);
        let path = format!("$ROOT/lib/{dir}/ebin");
        bytes.extend_from_slice(&u16::try_from(path.len()).unwrap_or(u16::MAX).to_be_bytes());
        bytes.extend_from_slice(path.as_bytes());
    }
    bytes.extend_from_slice(&[0x6a]);
    bytes
}

/// A written fake OTP root.
#[derive(Clone, Debug)]
pub struct FakeOtpRoot {
    /// The root itself, the directory holding `bin`, `erts-*`, `lib`.
    pub root: PathBuf,
    erts_vsn: String,
    release: u32,
}

impl FakeOtpRoot {
    /// `<root>/erts-<vsn>/bin`.
    pub fn erts_bin(&self) -> PathBuf {
        self.root
            .join(format!("erts-{}", self.erts_vsn))
            .join("bin")
    }

    /// `<root>/lib`.
    pub fn lib(&self) -> PathBuf {
        self.root.join("lib")
    }

    /// `<root>/bin/no_dot_erlang.boot`.
    pub fn boot_file(&self) -> PathBuf {
        self.root.join("bin").join("no_dot_erlang.boot")
    }

    /// The bytes of the boot file.
    ///
    /// # Panics
    ///
    /// If the boot file cannot be read.
    pub fn boot_bytes(&self) -> Vec<u8> {
        std::fs::read(self.boot_file()).expect("the fake boot file should be readable")
    }

    /// `<root>/bin/erl`, whether or not one was installed.
    pub fn erl(&self) -> PathBuf {
        self.root.join("bin").join("erl")
    }

    /// The argument vector the stub `bin/erl` was last called with.
    ///
    /// One entry per argument, in order, without the program itself. Empty
    /// when the stub was installed and never run.
    ///
    /// # Panics
    ///
    /// If no stub `erl` was installed, or if it was and the log cannot be read
    /// after it ran.
    pub fn erl_argv(&self) -> Vec<String> {
        let log = self.root.join("bin").join("erl.argv");
        assert!(
            self.erl().is_file(),
            "no stub erl was installed; call FakeOtp::with_erl_script"
        );
        match std::fs::read_to_string(&log) {
            Ok(text) => text.lines().map(str::to_owned).collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("cannot read {}: {error}", log.display()),
        }
    }

    /// `<root>/releases`.
    pub fn releases(&self) -> PathBuf {
        self.root.join("releases")
    }

    /// `<root>/releases/<release>`.
    pub fn release_dir(&self) -> PathBuf {
        self.releases().join(self.release.to_string())
    }

    /// `<root>/lib/<name>-<vsn>`, found by prefix.
    ///
    /// # Panics
    ///
    /// If no directory under `lib/` starts with `<name>-`.
    pub fn app_dir(&self, name: &str) -> PathBuf {
        let prefix = format!("{name}-");
        read_dir_names(&self.lib())
            .into_iter()
            .find(|entry| entry.starts_with(&prefix))
            .map(|entry| self.lib().join(entry))
            .unwrap_or_else(|| panic!("no `{name}-*` directory under {}", self.lib().display()))
    }
}

/// A fake `gleam export erlang-shipment` output directory.
#[derive(Clone, Debug, Default)]
pub struct FakeShipment {
    apps: Vec<FakeApp>,
}

impl FakeShipment {
    /// An empty shipment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an application with the given dependencies.
    #[must_use]
    pub fn app(self, name: &str, vsn: &str, applications: &[&str]) -> Self {
        self.push(FakeApp::new(name, vsn).applications(applications))
    }

    /// Adds an application described by a closure.
    #[must_use]
    pub fn app_with(self, name: &str, vsn: &str, build: impl FnOnce(FakeApp) -> FakeApp) -> Self {
        self.push(build(FakeApp::new(name, vsn)))
    }

    /// Replaces any application of the same name, keeping its position.
    fn push(mut self, app: FakeApp) -> Self {
        match self.apps.iter().position(|held| held.name == app.name) {
            Some(index) => self.apps[index] = app,
            None => self.apps.push(app),
        }
        self
    }

    /// Writes `<dir>/<app>/{ebin,priv}` for every application.
    ///
    /// Note the difference from [`FakeOtp`]: a shipment directory is named
    /// after the application alone, with no version, because the version lives
    /// only inside the `.app` file.
    pub fn build_in(self, dir: impl AsRef<Path>) -> FakeShipmentRoot {
        let root = dir.as_ref().to_path_buf();
        create_dir_all(&root);
        for app in &self.apps {
            let dir = root.join(&app.name);
            create_dir_all(&dir);
            app.write_into(&dir);
        }
        FakeShipmentRoot { root }
    }
}

/// A written fake shipment.
#[derive(Clone, Debug)]
pub struct FakeShipmentRoot {
    /// The directory holding one subdirectory per application.
    pub root: PathBuf,
}

impl FakeShipmentRoot {
    /// `<root>/<name>`.
    pub fn app_dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// `<root>/<name>/ebin/<name>.app`.
    pub fn app_file(&self, name: &str) -> PathBuf {
        self.app_dir(name).join("ebin").join(format!("{name}.app"))
    }
}

/// Writes a name as an Erlang atom, quoting it when it is not a bare one.
fn atom(name: &str) -> String {
    let mut chars = name.chars();
    let bare = chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@');
    if bare {
        name.to_owned()
    } else {
        format!("'{}'", name.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

/// Writes a list of names as Erlang atoms, separated by `, `.
fn atom_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| atom(name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Copies a slice of borrowed names.
fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// Creates a directory and its parents, failing the test if it cannot.
fn create_dir_all(path: &Path) {
    std::fs::create_dir_all(path)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", path.display()));
}

/// Writes a file, failing the test if it cannot.
fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
}

/// Writes a file and gives it mode 0755 on Unix.
fn write_executable(path: &Path, bytes: &[u8]) {
    write(path, bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("cannot chmod {}: {error}", path.display()));
    }
}

/// Adds the execute bits to a file, so a test can prove they are preserved.
///
/// [`FakeApp::priv_file`] writes a plain file, because most of `priv` is data.
/// A NIF and anything under `priv/bin` is not, and a staged tree that dropped
/// the execute bit would fail only when the application ran.
///
/// # Panics
///
/// If the file's permissions cannot be read or written.
#[cfg(unix)]
pub fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("cannot stat {}: {error}", path.display()))
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
        .unwrap_or_else(|error| panic!("cannot chmod {}: {error}", path.display()));
}

/// Removes the execute bits from a file, so a test can prove they are checked.
///
/// # Panics
///
/// If the file's permissions cannot be read or written.
#[cfg(unix)]
pub fn make_non_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = std::fs::metadata(path)
        .unwrap_or_else(|error| panic!("cannot stat {}: {error}", path.display()))
        .permissions();
    permissions.set_mode(0o644);
    std::fs::set_permissions(path, permissions)
        .unwrap_or_else(|error| panic!("cannot chmod {}: {error}", path.display()));
}

/// The entry names of a directory, sorted.
///
/// # Panics
///
/// If the directory cannot be read.
pub fn read_dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| {
            entry
                .expect("a readable directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}
