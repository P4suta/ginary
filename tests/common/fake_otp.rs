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
//! shell scripts that exit, and the `.beam` files are twelve bytes. What they
//! do carry is the *structure*, which is all the closure, assembly and
//! discovery code reads. A test that needs a real runtime is gated on the host
//! toolchain instead — see [`crate::common::tools::require_tools`].
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

/// The first twelve bytes of a BEAM file: an IFF `FOR1` chunk of form `BEAM`.
///
/// Enough that a reader can tell the file apart from noise, and deliberately
/// not enough to load. Nothing in ginary opens a `.beam`; `strip` will, and
/// those tests are gated on the real toolchain.
pub const DUMMY_BEAM: &[u8] = b"FOR1\x00\x00\x00\x04BEAM";

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
    apps: Vec<FakeApp>,
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

    /// Omits `releases/start_erl.data`, leaving the release to be inferred.
    #[must_use]
    pub fn without_start_erl_data(mut self) -> Self {
        self.start_erl_data = false;
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
        for name in ginary::otp::REQUIRED_ERTS_BINARIES {
            write_executable(
                &erts_bin.join(name),
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
        let mut bytes = vec![0x83, 0x68, 0x03, 0x64, 0x00, 0x06];
        bytes.extend_from_slice(b"script");
        for name in ["kernel", "stdlib"] {
            let Some(app) = self.apps.iter().find(|app| app.name == name) else {
                continue;
            };
            bytes.push(0x6b);
            let path = format!("$ROOT/lib/{}-{}/ebin", app.name, app.vsn);
            bytes.extend_from_slice(&u16::try_from(path.len()).unwrap_or(u16::MAX).to_be_bytes());
            bytes.extend_from_slice(path.as_bytes());
        }
        bytes.extend_from_slice(&[0x6a]);
        bytes
    }
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
