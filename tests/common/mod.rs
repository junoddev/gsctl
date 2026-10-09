//! Hermetic test environment for running the `gsctl` binary.
//!
//! Every invocation runs under its own `dbus-run-session` with a cleared
//! environment. HOME, every XDG directory, the dconf profile and the schema
//! source all point into a temp dir, so nothing can reach the real session bus
//! or the real dconf database:
//!
//! - `env_clear()` drops the real `DBUS_SESSION_BUS_ADDRESS`; `dbus-run-session`
//!   supplies a private bus, and `XDG_RUNTIME_DIR` (GLib's fallback bus
//!   location) is a temp dir too.
//! - The private bus uses its own config (`bus.conf`): it listens in the temp
//!   runtime dir, only activates services from the temp service dir, ignores
//!   `/etc/dbus-1`, and has AppArmor mediation off, which otherwise stops
//!   `dbus-daemon` from starting inside sandboxes such as bubblewrap.
//! - `dconf-service` is activated on that private bus and inherits the temp
//!   `XDG_CONFIG_HOME`, so it writes `$tmp/config/dconf/user`.
//! - `DCONF_PROFILE` points at a temp profile, so `/etc/dconf` is ignored.
//! - `XDG_DATA_DIRS` is a temp dir holding only the compiled test schemas and
//!   a copy of the dconf D-Bus service file, so tests don't depend on what is
//!   installed.
//!
//! `tests/isolation.rs` checks these properties.

#![allow(dead_code)]

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

pub const DBUS_RUN_SESSION: &str = "/usr/bin/dbus-run-session";
pub const DCONF_SERVICE_FILE: &str = "/usr/share/dbus-1/services/ca.desrt.dconf.service";
pub const EXTENSION_UUID: &str = "demo@gsctl.test";
pub const SYSTEM_EXTENSION_UUID: &str = "system@gsctl.test";

pub struct TestEnv {
    tmp: tempfile::TempDir,
    runs: Cell<u32>,
}

pub struct Run {
    pub status: i32,
    pub stdout: String,
    /// gsctl's own stderr.
    pub stderr: String,
    /// Messages from dbus-run-session and dbus-daemon, kept apart from gsctl's
    /// stderr so they can't corrupt its JSON errors.
    pub bus_log: String,
}

impl Run {
    fn from_output(out: Output, stderr: String) -> Run {
        Run {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr,
            bus_log: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// Parse stdout as JSON, panicking with the full output on failure.
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| {
            panic!("stdout is not JSON ({e}):\n{}\nstderr:\n{}{}", self.stdout, self.stderr, self.log())
        })
    }

    /// Parse stderr as a JSON error object.
    pub fn error(&self) -> Value {
        serde_json::from_str(&self.stderr).unwrap_or_else(|e| {
            panic!("stderr is not JSON ({e}):\n{}\nstdout:\n{}{}", self.stderr, self.stdout, self.log())
        })
    }

    /// Assert exit 0 and return stdout JSON.
    pub fn ok(&self) -> Value {
        assert_eq!(
            self.status,
            0,
            "expected success\nstdout:\n{}\nstderr:\n{}{}",
            self.stdout,
            self.stderr,
            self.log()
        );
        self.json()
    }

    fn log(&self) -> String {
        if self.bus_log.is_empty() { String::new() } else { format!("\nbus log:\n{}", self.bus_log) }
    }
}

impl TestEnv {
    pub fn new() -> TestEnv {
        let tmp = tempfile::Builder::new().prefix("gsctl-test-").tempdir().expect("tempdir");
        let env = TestEnv { tmp, runs: Cell::new(0) };
        for dir in ["home", "config", "state", "data-home", "share/dbus-1/services"] {
            fs::create_dir_all(env.path(dir)).unwrap();
        }
        let runtime = env.path("runtime");
        fs::create_dir_all(&runtime).unwrap();
        set_mode(&runtime, 0o700);

        compile_schemas(&manifest_dir().join("tests/schemas"), &env.schema_dir());
        let ext_schemas = env.extension_dir().join("schemas");
        compile_schemas(&manifest_dir().join("tests/schemas/extensions").join(EXTENSION_UUID), &ext_schemas);
        fs::write(
            env.extension_dir().join("metadata.json"),
            format!(r#"{{"uuid": "{EXTENSION_UUID}", "name": "Demo", "shell-version": ["50"]}}"#),
        )
        .unwrap();

        // A system-wide extension without its own schemas or settings-schema,
        // enabled by the session mode, like Ubuntu's dock.
        let sys_ext = env.path("share/gnome-shell/extensions").join(SYSTEM_EXTENSION_UUID);
        fs::create_dir_all(&sys_ext).unwrap();
        fs::write(
            sys_ext.join("metadata.json"),
            format!(r#"{{"uuid": "{SYSTEM_EXTENSION_UUID}", "name": "System Demo", "shell-version": ["50"]}}"#),
        )
        .unwrap();
        fs::write(sys_ext.join("extension.js"), "const s = this.getSettings('org.gnome.shell.extensions.gsctl-system');\nconst i = new Gio.Settings({schema_id: 'org.gsctl.test'});\n").unwrap();
        let modes = env.path("share/gnome-shell/modes");
        fs::create_dir_all(&modes).unwrap();
        fs::write(
            modes.join("testmode.json"),
            format!(r#"{{"parentMode": "user", "enabledExtensions": ["{SYSTEM_EXTENSION_UUID}"]}}"#),
        )
        .unwrap();

        fs::copy(DCONF_SERVICE_FILE, env.path("share/dbus-1/services/ca.desrt.dconf.service"))
            .expect("copy dconf D-Bus service file");
        env.write_profile(false);
        env.write_bus_config();
        env
    }

    fn write_bus_config(&self) {
        let conf = format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <keep_umask/>
  <listen>unix:dir={runtime}</listen>
  <auth>EXTERNAL</auth>
  <servicedir>{services}</servicedir>
  <apparmor mode="disabled"/>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
            runtime = self.path("runtime").display(),
            services = self.path("share/dbus-1/services").display(),
        );
        fs::write(self.path("bus.conf"), conf).unwrap();
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(rel)
    }

    pub fn schema_dir(&self) -> PathBuf {
        self.path("share/glib-2.0/schemas")
    }

    pub fn extension_dir(&self) -> PathBuf {
        self.path("data-home/gnome-shell/extensions").join(EXTENSION_UUID)
    }

    /// The dconf database the private dconf-service writes to.
    pub fn user_db(&self) -> PathBuf {
        self.path("config/dconf/user")
    }

    pub fn journal(&self) -> PathBuf {
        self.path("state/gsctl/journal.ndjson")
    }

    /// Add a read-only admin database that locks `/org/gsctl/test/locked-s`.
    pub fn lock_key(&self) {
        let keyfiles = self.path("admin-db.d");
        fs::create_dir_all(keyfiles.join("locks")).unwrap();
        fs::write(keyfiles.join("00-test"), "[org/gsctl/test]\nlocked-s='admin'\n").unwrap();
        fs::write(keyfiles.join("locks/00-test"), "/org/gsctl/test/locked-s\n").unwrap();
        let status = Command::new("/usr/bin/dconf")
            .arg("compile")
            .arg(self.path("admin.db"))
            .arg(&keyfiles)
            .env_clear()
            .status()
            .expect("dconf compile");
        assert!(status.success());
        self.write_profile(true);
    }

    fn write_profile(&self, with_admin_db: bool) {
        let mut profile = String::from("user-db:user\n");
        if with_admin_db {
            profile.push_str(&format!("file-db:{}\n", self.path("admin.db").display()));
        }
        fs::write(self.path("dconf-profile"), profile).unwrap();
    }

    /// Build a command that runs `program` inside a private D-Bus session with
    /// the hermetic environment.
    pub fn command(&self, program: impl AsRef<Path>) -> Command {
        let mut cmd = Command::new(DBUS_RUN_SESSION);
        cmd.arg(format!("--config-file={}", self.path("bus.conf").display()))
            .arg("--")
            .arg(program.as_ref())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C.UTF-8")
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("XDG_DATA_HOME", self.path("data-home"))
            .env("XDG_DATA_DIRS", self.path("share"))
            .env("XDG_RUNTIME_DIR", self.path("runtime"))
            .env("DCONF_PROFILE", self.path("dconf-profile"))
            .env("GSETTINGS_BACKEND", "dconf")
            .env("GNOME_SHELL_SESSION_MODE", "testmode")
            .env("GSETTINGS_SCHEMA_DIR", self.schema_dir());
        cmd
    }

    /// Run a shell script inside one private D-Bus session (so several gsctl
    /// processes share a bus, e.g. `watch` plus `set`). `$GSCTL` is the binary.
    pub fn session_script(&self, script: &str) -> Run {
        let out = self
            .command("/bin/sh")
            .args(["-c", script])
            .env("GSCTL", env!("CARGO_BIN_EXE_gsctl"))
            .env("TMP", self.path(""))
            .output()
            .expect("run script");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Run::from_output(out, stderr)
    }

    /// Run gsctl with the hermetic environment but no reachable session bus.
    pub fn gsctl_without_bus(&self, args: &[&str]) -> Run {
        let cmd = self.command(env!("CARGO_BIN_EXE_gsctl"));
        let out = Command::new(env!("CARGO_BIN_EXE_gsctl"))
            .args(args)
            .env_clear()
            .envs(cmd.get_envs().filter_map(|(k, v)| Some((k.to_owned(), v?.to_owned()))))
            .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", self.path("runtime/no-bus").display()))
            .output()
            .expect("run gsctl");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Run::from_output(out, stderr)
    }

    pub fn gsctl(&self, args: &[&str]) -> Run {
        let n = self.runs.get() + 1;
        self.runs.set(n);
        let stderr_file = self.path(&format!("stderr-{n}"));
        let out = self
            .command("/bin/sh")
            .args(["-c", r#"exec "$@" 2>"$GSCTL_TEST_STDERR""#, "sh", env!("CARGO_BIN_EXE_gsctl")])
            .args(args)
            .env("GSCTL_TEST_STDERR", &stderr_file)
            .output()
            .expect("run gsctl");
        let stderr = fs::read_to_string(&stderr_file).unwrap_or_default();
        Run::from_output(out, stderr)
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn compile_schemas(src: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "xml") {
            fs::copy(&p, target.join(p.file_name().unwrap())).unwrap();
        }
    }
    let status =
        Command::new("glib-compile-schemas").arg("--strict").arg(target).status().expect("glib-compile-schemas");
    assert!(status.success(), "glib-compile-schemas failed for {}", src.display());
}

fn set_mode(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}
