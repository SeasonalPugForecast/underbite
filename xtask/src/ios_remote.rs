//! `cargo xtask ios remote …`: finish the iOS core on a Mac over SSH.
//!
//! Rust is built on the build host (`cargo xtask ios build` cross-compiles every Apple
//! slice there, or here with `--local` or when no build host answers); the Mac only
//! needs Xcode. The working tree is copied with rsync, honouring `.gitignore` (so build
//! output and `.env` never leave this machine), into a directory on the Mac that this
//! command owns: it carries a `.underbite-remote` marker, and a directory without one is
//! never overwritten. The staged libraries and bindings follow into `.stage/`; the Mac
//! wraps them in the `XCFramework` with `xcodebuild` and runs `swift test`.
//!
//! The Mac comes from `--host`, else `UNDERBITE_MAC`, else `host` in the gitignored
//! `ios/remote.local.toml`; the directory likewise from `--dir`, `UNDERBITE_MAC_DIR`
//! or `dir`, relative to the Mac's home unless absolute (default `underbite-build`);
//! Xcode from `xcode` (default `/Applications/Xcode.app`), used through
//! `DEVELOPER_DIR`, so the Mac's `xcode-select` setting does not matter.
//!
//! **Lanes** (see [`crate::lane`]): a linked git worktree gets `-<lane>` appended to a
//! directory that comes from the config file or the default (`--dir` and
//! `UNDERBITE_MAC_DIR` are used as given), and `app` and `shots` use clones of the two
//! simulators (an iPhone and an iPad) named `<simulator> (<lane>)`, made on first use.
//! `device` signs with the Mac's login keychain, so it refuses to run in a lane. Each
//! run holds a lock on the directory (`<dir>.lock`), so two runs in one directory
//! queue. `clean --lane <name>` / `--all-lanes` removes lane directories (only those
//! with the marker) and their simulators. The config file is read from the worktree,
//! else from the main worktree.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::ios::{SWIFT, SWIFT_DEST, XCFRAMEWORK, path, stage, xcframework_args};
use crate::lane::{self, Which};
use crate::remote::{posix, quote};
use crate::workspace_root;

/// The marker that makes a directory on the Mac ours to replace.
const MARKER: &str = ".underbite-remote";
/// The default directory on the Mac, under its home.
const DEFAULT_DIR: &str = "underbite-build";
/// Where the staged Rust build goes on the Mac, inside the directory.
const STAGE: &str = ".stage";
/// The app's build products on the Mac, inside the directory.
const DERIVED: &str = ".derived";
/// Screenshots on the Mac, inside the directory.
const SCREENS: &str = ".screens";
/// App Store screenshots on the Mac, inside the directory.
const STORE_SCREENS: &str = ".store-shots";
/// The Release archive on the Mac, inside the directory.
const ARCHIVE_DIR: &str = ".archive";
/// The archive's file name.
const ARCHIVE_NAME: &str = "Underbite.xcarchive";
/// Where `store-shots` puts its screenshots here.
const STORE_LOCAL: &str = "target/ios-store-shots";
/// The store shots' launch arguments, one `<name>.args` file each.
const STORE_SHOTS_DIR: &str = "ios/shots/store";
/// The Swift script that rewrites the store screenshots without their alpha channel, run on the
/// Mac (relative to the repository).
const DROP_ALPHA: &str = "ios/shots/store/drop-alpha.swift";
/// The most screenshots App Store Connect takes for one device size.
const MAX_STORE_SHOTS: usize = 10;
/// The iPhone simulator for `store-shots` when `store_simulator` is not set: a 6.9-inch
/// iPhone, whose screenshots are 1320x2868 pixels.
const DEFAULT_STORE_SIMULATOR: &str = "iPhone 17 Pro Max";
/// The iPad simulator for `store-shots` when `store_ipad_simulator` is not set: a 13-inch
/// iPad Pro, whose screenshots are 2064x2752 pixels.
const DEFAULT_STORE_IPAD_SIMULATOR: &str = "iPad Pro 13-inch (M5)";
/// The portrait pixel sizes App Store Connect accepts for the 6.9-inch iPhone display
/// (<https://developer.apple.com/help/app-store-connect/reference/app-information/screenshot-specifications>).
const IPHONE_STORE_SIZES: [(u32, u32); 3] = [(1260, 2736), (1290, 2796), (1320, 2868)];
/// The portrait sizes it requires for an iPad app: the 13-inch display.
const IPAD_STORE_SIZES: [(u32, u32); 2] = [(2064, 2752), (2048, 2732)];
/// The iPhone simulator for `app` and `shots` when `simulator` is not set.
const DEFAULT_SIMULATOR: &str = "iPhone 17";
/// The iPad simulator when `ipad_simulator` is not set: an 11-inch iPad Air, which the
/// iOS 26 and later simulator runtimes offer.
const DEFAULT_IPAD_SIMULATOR: &str = "iPad Air 11-inch (M4)";
/// The app's bundle identifier.
const BUNDLE_ID: &str = "dev.savery.underbite";
/// The Xcode project, relative to the repository.
const PROJECT: &str = "ios/Underbite/Underbite.xcodeproj";
/// The scheme the project builds.
const SCHEME: &str = "Underbite";
/// The app bundle the build produces.
const APP: &str = "Underbite.app";
/// The Swift package that wraps the core, relative to the repository.
const KIT: &str = "ios/UnderbiteKit";
/// The launch arguments of each screenshot, one `<name>.args` file each.
const SHOTS_DIR: &str = "ios/shots";
/// Xcode when `xcode` is not set.
const DEFAULT_XCODE: &str = "/Applications/Xcode.app";
/// The oldest iOS the doctor accepts, for the SDK and the simulator runtime. The app
/// needs iOS 26; a newer SDK builds for it.
const MIN_IOS: u32 = 26;
/// Free disk space (in GiB) under which the doctor warns.
const MIN_FREE_GIB: u64 = 15;

const USAGE: &str = "\
usage: cargo xtask ios remote <step> [--host user@mac] [--dir path] [--debug] [--dry-run]
                               [--iphone name] [--release] [--local] [--simulator name]
                               [--unsigned]
       cargo xtask ios remote ui [--on iphone|ipad] [--only Class/test] [--host user@mac] [--dry-run]
       cargo xtask ios remote exec [--fetch path]… [--host user@mac] [--dir path] [--dry-run] -- command…
       cargo xtask ios remote clean (--lane <name> | --all-lanes) [--host user@mac] [--dry-run]

steps:
  doctor   show the Mac's macOS, Xcode, SDKs, simulator runtimes, device types and free
           disk space; fails unless an iOS 26 (or later) SDK and simulator runtime exist
  sync     copy the working tree to the Mac
  build    build the Rust, sync, and make the `XCFramework` on the Mac
  test     build, then `swift test` in ios/UnderbiteKit on the Mac
  app      build, then build the app (ios/Underbite) for the simulator
  shots    app, then run it in an iPhone and an iPad simulator, take a screenshot for each
           ios/shots/*.args file (a light one, and a dark one when it has a `dark` line;
           a `wait <seconds>` line waits longer before the shot) and copy them to
           target/ios-screens/<iphone|ipad>/
  store-shots
           app, then the App Store screenshots (ios/shots/store/*.args, light, status bar
           9:41) on `store_simulator` (default iPhone 17 Pro Max, 1320x2868) and
           `store_ipad_simulator` (default iPad Pro 13-inch (M5), 2064x2752) to
           target/ios-store-shots/<iphone|ipad>/; --simulator picks another iPhone
  archive  build, then a Release archive in the lane's directory on the Mac
           (.archive/Underbite.xcarchive), signed with the team of ios/remote.local.toml
           (`team`) unless --unsigned. Nothing is exported or uploaded
  ui       build, then run the UI tests (UnderbiteUITests: the accessibility audits) on the
           iPhone and then the iPad simulator, each with its accessibility switched on first
           (else the runner can time out waiting for it); --on iphone|ipad tests on one,
           --only runs one class or test (AccessibilityAuditTests/testConditions)
  exec     sync, then run a command in the lane's directory on the Mac (Xcode selected
           through DEVELOPER_DIR); --fetch copies a path (relative to the repository) back
  device   build, sign and install the app on the iPhone named `iphone` in
           ios/remote.local.toml (paired with the Mac; team from `team`). Over
           SSH the Mac's login keychain is locked: it asks for the Mac
           account's password to unlock it (run it in a terminal), or takes
           it from `keychain_password_command` there.
           --iphone <name> installs on that phone instead (a new one is
           registered with the team on its first install).
           --release builds the app optimized (the Rust core is always release
           unless --debug)
  clean    delete a lane's directory and simulators on the Mac (--lane underbite-lane-a),
           or every lane's (--all-lanes). Never the main checkout's

A linked git worktree is a lane: its directory on the Mac is `<dir>-<lane>` and its simulators
`<simulator> (<lane>)` (lane = the worktree directory's name; UNDERBITE_LANE overrides it,
empty switches it off). `device` runs only from the main checkout.

--simulator <name> uses that iPhone simulator instead of `simulator` (`store_simulator` for
store-shots).

The Mac is --host, else UNDERBITE_MAC, else `host` in ios/remote.local.toml.
The Rust core is built on the build host of `cargo xtask remote` when one answers
(remote.local.toml), else here; --local always builds it here.";

/// What to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Doctor,
    Sync,
    Build,
    Test,
    App,
    Shots,
    StoreShots,
    Archive,
    Ui,
    Exec,
    Device,
    Clean,
}

impl Step {
    /// Whether the Rust core is built and packaged first.
    fn packages(self) -> bool {
        matches!(
            self,
            Self::Build
                | Self::Test
                | Self::App
                | Self::Shots
                | Self::StoreShots
                | Self::Archive
                | Self::Ui
                | Self::Device
        )
    }
}

/// A parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // command-line flags
struct Args {
    step: Step,
    host: Option<String>,
    dir: Option<String>,
    debug: bool,
    dry_run: bool,
    /// The phone to install on (`device`), overriding `iphone` in the config.
    iphone: Option<String>,
    /// Build the app (Swift) in Release for the phone (`device`).
    release: bool,
    /// Build the Rust core here even when a build host answers.
    local: bool,
    /// `clean`: the lane to delete.
    lane: Option<String>,
    /// `clean`: every lane.
    all_lanes: bool,
    /// The iPhone simulator to use instead of the configured one.
    simulator: Option<String>,
    /// `archive`: do not sign.
    unsigned: bool,
    /// `ui`: the one simulator to test on (`iphone` or `ipad`), else both.
    on: Option<String>,
    /// `ui`: the one test class or test to run (`AccessibilityAuditTests/testConditions`).
    only: Option<String>,
    /// `exec`: paths to copy back afterwards.
    fetch: Vec<String>,
    /// `exec`: the command to run on the Mac.
    command: Vec<String>,
}

fn parse(args: &[&str]) -> Result<Args, String> {
    let mut iter = args.iter();
    let step = match iter.next() {
        Some(&"doctor") => Step::Doctor,
        Some(&"sync") => Step::Sync,
        Some(&"build") => Step::Build,
        Some(&"test") => Step::Test,
        Some(&"app") => Step::App,
        Some(&"shots") => Step::Shots,
        Some(&"store-shots") => Step::StoreShots,
        Some(&"archive") => Step::Archive,
        Some(&"ui") => Step::Ui,
        Some(&"exec") => Step::Exec,
        Some(&"device") => Step::Device,
        Some(&"clean") => Step::Clean,
        _ => return Err(USAGE.into()),
    };
    let mut out = Args {
        step,
        host: None,
        dir: None,
        debug: false,
        dry_run: false,
        iphone: None,
        release: false,
        local: false,
        lane: None,
        all_lanes: false,
        simulator: None,
        unsigned: false,
        on: None,
        only: None,
        fetch: Vec::new(),
        command: Vec::new(),
    };
    while let Some(arg) = iter.next() {
        match *arg {
            "--host" => out.host = iter.next().map(|s| (*s).to_owned()),
            "--dir" => out.dir = iter.next().map(|s| (*s).to_owned()),
            "--debug" => out.debug = true,
            "--dry-run" => out.dry_run = true,
            "--release" => out.release = true,
            "--local" => out.local = true,
            "--unsigned" => out.unsigned = true,
            "--all-lanes" => out.all_lanes = true,
            "--lane" => match iter.next() {
                Some(name) => out.lane = Some((*name).to_owned()),
                None => return Err(format!("--lane needs a lane name\n\n{USAGE}")),
            },
            "--simulator" => match iter.next() {
                Some(name) if !name.is_empty() => out.simulator = Some((*name).to_owned()),
                _ => return Err(format!("--simulator needs a simulator name\n\n{USAGE}")),
            },
            "--iphone" => match iter.next() {
                Some(name) if !name.is_empty() => out.iphone = Some((*name).to_owned()),
                _ => return Err(format!("--iphone needs a phone name\n\n{USAGE}")),
            },
            "--on" => match iter.next() {
                Some(&device) if device == "iphone" || device == "ipad" => {
                    out.on = Some(device.to_owned());
                }
                _ => return Err(format!("--on needs `iphone` or `ipad`\n\n{USAGE}")),
            },
            "--only" => match iter.next() {
                Some(test) if is_test_name(test) => out.only = Some((*test).to_owned()),
                _ => {
                    return Err(format!(
                        "--only needs a test class or test, like \
                         AccessibilityAuditTests/testConditions\n\n{USAGE}"
                    ));
                }
            },
            "--fetch" => match iter.next() {
                Some(path) => out.fetch.push(fetchable(path)?),
                None => return Err(format!("--fetch needs a path\n\n{USAGE}")),
            },
            "--" => out.command = iter.by_ref().map(|s| (*s).to_owned()).collect(),
            other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    if step != Step::Clean && (out.lane.is_some() || out.all_lanes) {
        return Err(format!("--lane and --all-lanes belong to clean\n\n{USAGE}"));
    }
    if step == Step::Clean {
        lane::which(out.lane.as_deref(), out.all_lanes)?;
    }
    if step != Step::Exec && !(out.fetch.is_empty() && out.command.is_empty()) {
        return Err(format!("--fetch and a command belong to exec\n\n{USAGE}"));
    }
    if step != Step::Archive && out.unsigned {
        return Err(format!("--unsigned belongs to archive\n\n{USAGE}"));
    }
    if step != Step::Ui && (out.on.is_some() || out.only.is_some()) {
        return Err(format!("--on and --only belong to ui\n\n{USAGE}"));
    }
    if step == Step::Exec && out.command.is_empty() {
        return Err(format!("exec needs a command after `--`\n\n{USAGE}"));
    }
    Ok(out)
}

/// A test class or test of the UI test target: letters, digits and `_`, with at most one `/`
/// between a class and a test.
fn is_test_name(text: &str) -> bool {
    let parts: Vec<&str> = text.split('/').collect();
    parts.len() <= 2
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// `path` when it stays inside the repository, without a trailing slash.
fn fetchable(path: &str) -> Result<String, String> {
    let trimmed = path.trim_end_matches('/');
    let inside = !trimmed.is_empty()
        && !trimmed.starts_with('/')
        && trimmed.split('/').all(|part| part != ".." && part != ".");
    if inside {
        Ok(trimmed.to_owned())
    } else {
        Err(format!(
            "--fetch `{path}`: use a path inside the repository, relative to its root"
        ))
    }
}

/// `ios/remote.local.toml`: the Mac, its Xcode, the simulators and the signing values.
#[derive(Debug, Default)]
struct LocalFile {
    host: Option<String>,
    dir: Option<String>,
    xcode: Option<String>,
    simulator: Option<String>,
    ipad_simulator: Option<String>,
    store_simulator: Option<String>,
    store_ipad_simulator: Option<String>,
    team: Option<String>,
    iphone: Option<String>,
    keychain_password_command: Option<String>,
    /// The file these values came from, when there is one.
    path: Option<PathBuf>,
}

impl LocalFile {
    fn parse(text: &str) -> Result<Self, String> {
        let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
        let mut out = Self::default();
        for (key, value) in table {
            let text = value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`{key}` must be a string"))?;
            match key.as_str() {
                "host" => out.host = Some(text),
                "dir" => out.dir = Some(text),
                "xcode" => out.xcode = Some(text),
                "simulator" => out.simulator = Some(text),
                "ipad_simulator" => out.ipad_simulator = Some(text),
                "store_simulator" => out.store_simulator = Some(text),
                "store_ipad_simulator" => out.store_ipad_simulator = Some(text),
                "team" => out.team = Some(text),
                "iphone" => out.iphone = Some(text),
                "keychain_password_command" => out.keychain_password_command = Some(text),
                _ => {
                    return Err(format!(
                        "unknown key `{key}` (use host, dir, xcode, simulator, ipad_simulator, \
                         store_simulator, store_ipad_simulator, team, iphone and \
                         keychain_password_command)"
                    ));
                }
            }
        }
        Ok(out)
    }
}

/// The Mac, the directory there, its Xcode and the simulators.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Remote {
    host: String,
    /// The directory this run uses: `base_dir` with `-<lane>` in a lane, unless
    /// `--dir` or `UNDERBITE_MAC_DIR` named it.
    dir: String,
    /// The directory without a lane: the one the main checkout uses.
    base_dir: String,
    /// This worktree's lane, if it is a linked one.
    lane: Option<String>,
    xcode: String,
    /// The iPhone simulator this run uses: `base_simulator`, or its clone for the lane.
    simulator: String,
    /// The iPhone simulator the main checkout uses: `--simulator`, else the configured one.
    base_simulator: String,
    /// The configured iPhone simulator (`clean` removes its lane clones too).
    config_simulator: String,
    /// The iPad simulator this run uses: `base_ipad`, or its clone for the lane.
    ipad: String,
    /// The iPad simulator the main checkout uses.
    base_ipad: String,
    /// The iPhone simulator `store-shots` uses: `base_store`, or its clone for the lane.
    store: String,
    /// The iPhone simulator the main checkout uses for `store-shots`: `--simulator`, else
    /// `store_simulator`.
    base_store: String,
    /// The iPad simulator `store-shots` uses: `base_store_ipad`, or its clone for the lane.
    store_ipad: String,
    /// The iPad simulator the main checkout uses for `store-shots`.
    base_store_ipad: String,
    /// The Apple development team for signing, if set here.
    team: Option<String>,
    /// The paired iPhone's name, for `device`.
    iphone: Option<String>,
    /// A command here that prints the Mac account's password, for `device` to
    /// unlock the keychain without asking (e.g. `secret-tool lookup …`).
    keychain_password_command: Option<String>,
}

/// `ios/remote.local.toml` of this worktree, else of the main worktree (a lane
/// has none of its own), or nothing when neither exists.
fn local_file() -> Result<LocalFile, String> {
    let Some(path) = lane::config_file("ios/remote.local.toml") else {
        return Ok(LocalFile::default());
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(LocalFile::default());
    };
    LocalFile::parse(&text)
        .map(|file| LocalFile {
            path: Some(path.clone()),
            ..file
        })
        .map_err(|e| {
            format!(
                "{}: {e}\n(values are quoted: host = \"user@mac\")",
                path.display()
            )
        })
}

fn remote(
    args: &Args,
    env: &dyn Fn(&str) -> Option<String>,
    file: LocalFile,
    lane: Option<String>,
) -> Result<Remote, String> {
    let nonempty = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    let host = nonempty(args.host.clone())
        .or_else(|| nonempty(env("UNDERBITE_MAC")))
        .or_else(|| nonempty(file.host))
        .ok_or_else(|| {
            "which Mac? pass --host user@mac, set UNDERBITE_MAC, or put `host = \"user@mac\"` \
             in ios/remote.local.toml"
                .to_string()
        })?;
    let explicit = nonempty(args.dir.clone()).or_else(|| nonempty(env("UNDERBITE_MAC_DIR")));
    // `clean --dir d` cleans the lanes of `d`; otherwise `d` is not a base at all.
    let named_base = explicit.clone().filter(|_| args.step == Step::Clean);
    let base_dir = named_base
        .or_else(|| nonempty(file.dir))
        .unwrap_or_else(|| DEFAULT_DIR.to_string());
    // A named directory is used as given; the file's or the default one is per lane.
    let dir = match (&explicit, &lane) {
        (Some(dir), _) => dir.clone(),
        (None, Some(lane)) => lane::lane_dir(&base_dir, lane),
        (None, None) => base_dir.clone(),
    };
    for checked in [&dir, &base_dir] {
        if checked.trim_matches('/').is_empty() || checked == "." || checked == "~" {
            return Err(format!(
                "`{checked}` is not a directory this command may replace"
            ));
        }
    }
    let xcode = nonempty(file.xcode).unwrap_or_else(|| DEFAULT_XCODE.to_string());
    let config_simulator =
        nonempty(file.simulator).unwrap_or_else(|| DEFAULT_SIMULATOR.to_string());
    let base_ipad =
        nonempty(file.ipad_simulator).unwrap_or_else(|| DEFAULT_IPAD_SIMULATOR.to_string());
    // `--simulator` names the iPhone of whichever step it is given to.
    let for_store = args.step == Step::StoreShots;
    let base_simulator = nonempty(args.simulator.clone().filter(|_| !for_store))
        .unwrap_or_else(|| config_simulator.clone());
    let base_store = nonempty(args.simulator.clone().filter(|_| for_store))
        .or_else(|| nonempty(file.store_simulator))
        .unwrap_or_else(|| DEFAULT_STORE_SIMULATOR.to_string());
    let base_store_ipad = nonempty(file.store_ipad_simulator)
        .unwrap_or_else(|| DEFAULT_STORE_IPAD_SIMULATOR.to_string());
    Ok(Remote {
        host,
        dir,
        base_dir,
        xcode,
        simulator: lane_simulator(&base_simulator, lane.as_deref()),
        base_simulator,
        config_simulator,
        ipad: lane_simulator(&base_ipad, lane.as_deref()),
        base_ipad,
        store: lane_simulator(&base_store, lane.as_deref()),
        base_store,
        store_ipad: lane_simulator(&base_store_ipad, lane.as_deref()),
        base_store_ipad,
        lane,
        team: nonempty(file.team),
        iphone: nonempty(file.iphone),
        keychain_password_command: nonempty(file.keychain_password_command),
    })
}

/// The simulator a lane uses: `<simulator> (<lane>)`; the main checkout's is the
/// configured one.
fn lane_simulator(base: &str, lane: Option<&str>) -> String {
    lane.map_or_else(|| base.to_string(), |lane| format!("{base} ({lane})"))
}

/// The Mac's Xcode, whatever `xcode-select` says.
fn developer_dir(remote: &Remote) -> String {
    format!(
        "export DEVELOPER_DIR={}",
        quote(&format!("{}/Contents/Developer", remote.xcode))
    )
}

/// A shell line for the Mac: Xcode, then `body` in the directory.
fn in_dir(remote: &Remote, body: &str) -> String {
    format!(
        "set -e; {}; cd {}; {body}",
        developer_dir(remote),
        quote(&remote.dir)
    )
}

/// Refuses a directory without the marker, else creates both.
fn claim(remote: &Remote) -> String {
    let dir = quote(&remote.dir);
    format!(
        "if [ -e {dir} ] && [ ! -e {dir}/{MARKER} ]; then \
         echo {dir} exists and was not made by cargo xtask ios remote >&2; exit 3; fi; \
         mkdir -p {dir} && touch {dir}/{MARKER}"
    )
}

fn rsync_args(remote: &Remote) -> Vec<String> {
    crate::remote::TREE_COPY
        .iter()
        .map(|a| (*a).to_owned())
        .chain([
            "--delete".into(),
            // Every .gitignore applies, so build output and secrets stay here.
            "--filter=:- .gitignore".into(),
            format!("--filter=P /{MARKER}"),
            format!("--filter=P /{STAGE}"),
            format!("--filter=P /{DERIVED}"),
            format!("--filter=P /{SCREENS}"),
            format!("--filter=P /{STORE_SCREENS}"),
            format!("--filter=P /{ARCHIVE_DIR}"),
            // What `build` made there stays for the steps after it (`exec`, `sync`).
            format!("--filter=P /{XCFRAMEWORK}"),
            format!("--filter=P /{SWIFT_DEST}/{SWIFT}"),
            "--exclude=/.git".into(),
        ])
        .chain(crate::remote::local_excludes())
        .chain(["./".into(), format!("{}:{}/", remote.host, remote.dir)])
        .collect()
}

/// Copies the staged build (`stage`, here) into the directory's `.stage/`.
fn rsync_stage(remote: &Remote, stage: &str) -> Vec<String> {
    vec![
        "-az".into(),
        "--delete".into(),
        format!("{stage}/"),
        format!("{}:{}/{STAGE}/", remote.host, remote.dir),
    ]
}

/// On the Mac: the bindings into the package and the `XCFramework` from `.stage`.
fn package(remote: &Remote) -> String {
    let args: Vec<String> = xcframework_args(STAGE, XCFRAMEWORK)
        .iter()
        .map(|a| quote(a))
        .collect();
    in_dir(
        remote,
        &format!(
            "mkdir -p {SWIFT_DEST}; cp {STAGE}/{SWIFT} {SWIFT_DEST}/{SWIFT}; rm -rf {XCFRAMEWORK}; \
             xcodebuild {} >/dev/null; echo wrote {XCFRAMEWORK}",
            args.join(" ")
        ),
    )
}

/// On the Mac: the app for the simulator `sim`, without signing.
fn app(remote: &Remote, sim: &str) -> String {
    let destination = quote(&format!("platform=iOS Simulator,name={sim}"));
    in_dir(
        remote,
        &format!(
            "xcodebuild -quiet -project {PROJECT} -scheme {SCHEME} \
             -destination {destination} -derivedDataPath {DERIVED} CODE_SIGNING_ALLOWED=NO build; \
             echo built the app"
        ),
    )
}

/// One screenshot: a file name and the app's launch arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Shot {
    /// The file name without `.args`; the screenshot is `<name>.png`.
    name: String,
    /// The launch arguments, split on whitespace.
    args: Vec<String>,
    /// Also take `<name>-dark.png` in the dark appearance.
    dark: bool,
    /// Seconds to wait before the screenshot, when longer than the usual wait.
    wait: Option<u32>,
}

/// The longest wait a shot may ask for, seconds.
const MAX_SHOT_WAIT: u32 = 180;

impl Shot {
    /// Seconds to wait after the launch: the usual `base`, or what the shot asked for when
    /// that is longer.
    fn wait_after_launch(&self, base: u32) -> u32 {
        self.wait.map_or(base, |w| w.max(base))
    }
}

/// A shot from `<name>.args`: line 1 the launch arguments (blank for none), then optional
/// lines in any order, each at most once: `dark` and `wait <seconds>` (up to
/// [`MAX_SHOT_WAIT`]).
fn parse_shot(name: &str, text: &str) -> Result<Shot, String> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid {
        return Err(format!(
            "{SHOTS_DIR}/{name}.args: the name may hold only letters, digits, `-`, `_` and `.`"
        ));
    }
    let mut lines = text.lines();
    let args = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut dark = false;
    let mut wait = None;
    for line in lines.map(str::trim).filter(|l| !l.is_empty()) {
        let mut words = line.split_whitespace();
        match (words.next(), words.next(), words.next()) {
            (Some("dark"), None, _) if !dark => dark = true,
            (Some("wait"), Some(n), None) if wait.is_none() => match n.parse::<u32>() {
                Ok(n) if (1..=MAX_SHOT_WAIT).contains(&n) => wait = Some(n),
                _ => {
                    return Err(format!(
                        "{SHOTS_DIR}/{name}.args: `wait` takes whole seconds from 1 to \
                         {MAX_SHOT_WAIT}, not `{n}`"
                    ));
                }
            },
            _ => {
                return Err(format!(
                    "{SHOTS_DIR}/{name}.args: after the arguments only `dark` and \
                     `wait <seconds>` (once each) are allowed, not `{line}`"
                ));
            }
        }
    }
    Ok(Shot {
        name: name.to_owned(),
        args,
        dark,
        wait,
    })
}

/// The shots in `dir` (`*.args`, sorted by name); none when it has no such files.
fn read_shots(dir: &Path) -> Result<Vec<Shot>, String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "args"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|file| {
            let name = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text =
                std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            parse_shot(&name, &text)
        })
        .collect()
}

/// The store shots in [`STORE_SHOTS_DIR`]: light only (a `dark` line is an error), at least one
/// and at most [`MAX_STORE_SHOTS`], the number App Store Connect takes per device size.
fn read_store_shots(dir: &Path) -> Result<Vec<Shot>, String> {
    let list = read_shots(dir)?;
    if let Some(shot) = list.iter().find(|s| s.dark) {
        return Err(format!(
            "{STORE_SHOTS_DIR}/{}.args: store screenshots are light only, drop the `dark` line",
            shot.name
        ));
    }
    if list.len() > MAX_STORE_SHOTS {
        return Err(format!(
            "{STORE_SHOTS_DIR}/ has {} shots; App Store Connect takes {MAX_STORE_SHOTS} per device size",
            list.len()
        ));
    }
    Ok(list)
}

/// The width, height and colour type of the PNG in `bytes` (from its `IHDR` chunk).
fn png_header(bytes: &[u8]) -> Option<(u32, u32, u8)> {
    const SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 26 || !bytes.starts_with(SIGNATURE) || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let word =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    Some((word(16), word(20), bytes[25]))
}

/// Whether a PNG colour type carries an alpha channel (grey with alpha, or RGBA).
fn has_alpha(color_type: u8) -> bool {
    matches!(color_type, 4 | 6)
}

/// The problems with the PNGs in `dir`: each must be one of `sizes`, and have no alpha channel
/// (App Store Connect refuses both otherwise). Empty when all is well; a directory without
/// PNGs is a problem too.
fn check_store_pngs(dir: &Path, sizes: &[(u32, u32)], expected: usize) -> Vec<String> {
    let mut found = 0;
    let mut problems = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    files.sort();
    for file in files {
        found += 1;
        let name = file.display();
        let mut head = vec![0u8; 26];
        let read = std::fs::File::open(&file).and_then(|mut f| {
            use std::io::Read as _;
            f.read_exact(&mut head)
        });
        match (read, png_header(&head)) {
            (Ok(()), Some((w, h, color))) => {
                if !sizes.contains(&(w, h)) {
                    problems.push(format!(
                        "{name}: {w}x{h} is not a size App Store Connect takes"
                    ));
                }
                if has_alpha(color) {
                    problems.push(format!("{name}: has an alpha channel"));
                }
            }
            _ => problems.push(format!("{name}: not a readable PNG")),
        }
    }
    if found != expected {
        problems.push(format!(
            "{}: {found} screenshots, expected {expected}",
            dir.display()
        ));
    }
    problems
}

/// On the Mac: makes the simulators the step needs, unless they exist. A missing configured
/// simulator is made from the device type of that name (the newest runtime). In a lane the
/// lane's simulator is a clone of it; `clone` needs the source shut down, so the configured
/// simulator is shut down first (once per lane; the main checkout boots it again when it
/// needs it).
fn ensure_simulators(remote: &Remote) -> String {
    ensure_pairs(
        remote,
        &[
            (&remote.simulator, &remote.base_simulator),
            (&remote.ipad, &remote.base_ipad),
        ],
    )
}

/// [`ensure_simulators`] for the two simulators `store-shots` uses.
fn ensure_store_simulators(remote: &Remote) -> String {
    ensure_pairs(
        remote,
        &[
            (&remote.store, &remote.base_store),
            (&remote.store_ipad, &remote.base_store_ipad),
        ],
    )
}

/// Makes each `(simulator, base)` pair of simulators: the base from its device type when
/// missing, and in a lane the lane's clone of it.
fn ensure_pairs(remote: &Remote, pairs: &[(&String, &String)]) -> String {
    let mut parts = vec![developer_dir(remote)];
    for (sim, base) in pairs {
        parts.push(ensure_simulator(remote, sim, base));
    }
    parts.join("; ")
}

fn ensure_simulator(remote: &Remote, sim_name: &str, base_name: &str) -> String {
    let (sim, base) = (quote(sim_name), quote(base_name));
    // `list` prints `    <name> (<udid>) (<state>)`; a lane's clone shares the name's start.
    let exists = quote(&format!(
        "^    {} \\([0-9A-Fa-f-]{{36}}\\)",
        ere_escape(base_name)
    ));
    let types = quote(&format!(
        "s/^{} (\\(com\\.apple\\.[^)]*\\))$/\\1/p",
        sed_escape(base_name)
    ));
    let mut line = format!(
        "if ! xcrun simctl list devices available | grep -qE {exists}; then \
         t=$(xcrun simctl list devicetypes | sed -n {types} | head -1); \
         if [ -z \"$t\" ]; then echo \"no simulator or device type named \"{base} >&2; exit 1; fi; \
         xcrun simctl create {base} \"$t\"; fi"
    );
    if remote.lane.is_some() {
        // The device list prints `    <name> (<udid>) (<state>)`: match the whole name.
        let listed = quote(&format!("    {sim_name} ("));
        let _ = write!(
            line,
            "; if xcrun simctl list devices available | grep -qF {listed}; \
             then echo simulator {sim} exists; \
             else xcrun simctl shutdown {base} 2>/dev/null || true; xcrun simctl clone {base} {sim}; fi"
        );
    }
    line
}

/// `text` for an `grep -E` regular expression.
fn ere_escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if "\\.[]*^$(){}+?|".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

/// `text` for a `sed` basic regular expression.
fn sed_escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if "\\.[]*^$/".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}

/// On the Mac: shuts down and deletes the simulators of the lanes named by `which`,
/// for every simulator a lane clones (the configured and the named iPhone's, and the iPad's).
fn delete_simulators(remote: &Remote, which: &Which) -> String {
    let mut bases: Vec<&str> = Vec::new();
    for base in [
        &remote.config_simulator,
        &remote.base_simulator,
        &remote.base_ipad,
        &remote.base_store,
        &remote.base_store_ipad,
    ] {
        if !bases.contains(&base.as_str()) {
            bases.push(base);
        }
    }
    let mut parts = vec![developer_dir(remote)];
    for base in bases {
        match which {
            Which::One(lane) => {
                let sim = quote(&lane_simulator(base, Some(lane)));
                parts.push(format!(
                    "xcrun simctl shutdown {sim} 2>/dev/null || true; \
                     if xcrun simctl delete {sim} 2>/dev/null; then echo deleted simulator {sim}; \
                     else echo no simulator {sim}; fi"
                ));
            }
            // Lane clones are listed `    <simulator> (<lane>) (<udid>) (<state>)`; the
            // configured one has no lane in parentheses, so it never matches.
            Which::All => {
                let list = quote(&format!(
                    "s/^    {} (.*) (\\([0-9A-Fa-f-]\\{{36\\}}\\)) (.*$/\\1/p",
                    sed_escape(base)
                ));
                parts.push(format!(
                    "for u in $(xcrun simctl list devices | sed -n {list}); do \
                     xcrun simctl shutdown \"$u\" 2>/dev/null || true; \
                     xcrun simctl delete \"$u\" && echo deleted simulator \"$u\"; done"
                ));
            }
        }
    }
    parts.join("; ")
}

/// `device` signs with the Mac's login keychain and installs on the phone, which only the
/// main checkout does.
fn check_lane(step: Step, lane: Option<&str>) -> Result<(), String> {
    match (step, lane) {
        (Step::Device, Some(lane)) => Err(format!(
            "`device` signs with the Mac's login keychain and installs on the phone: \
             run device from the main checkout (this is the lane `{lane}`)"
        )),
        _ => Ok(()),
    }
}

/// A screenshot smaller than this is a blank (white, not yet drawn) screen: a blank
/// one is about 70 KB, the plainest real ones about 160 KB, and most well over that.
const BLANK_BYTES: u32 = 120_000;

/// One screenshot: (re)launches the app with `args`, waits `wait` seconds and
/// shoots `dir/name.png`. A busy Mac (other lanes building) can leave the app
/// undrawn after the wait: a blank white screen makes a small PNG, so a shot
/// under [`BLANK_BYTES`] is taken again after a further wait, up to three
/// times, and one still blank is reported rather than passed off. The size
/// comes from `wc -c`: the Mac's `stat` may be GNU's (Homebrew), whose `-f`
/// means something else. A launch that fails (the first one after an install can) is tried
/// again, up to three times. `args` are quoted word by word.
fn launch_line(sim: &str, dir: &str, name: &str, args: &[String], wait: u32) -> String {
    let file = format!("{dir}/{name}.png");
    let args = args.iter().fold(String::new(), |mut all, a| {
        let _ = write!(all, " {}", quote(a));
        all
    });
    format!(
        "xcrun simctl terminate {sim} {BUNDLE_ID} 2>/dev/null || true; \
         l=0; until xcrun simctl launch {sim} {BUNDLE_ID}{args} >/dev/null; do l=$((l+1)); \
         [ $l -ge 3 ] && break; sleep 6; done; sleep {wait}; n=0; \
         while :; do xcrun simctl io {sim} screenshot {file} >/dev/null 2>&1; \
         size=$(wc -c < {file} 2>/dev/null) || size=0; [ ${{size:-0}} -ge {BLANK_BYTES} ] && break; \
         n=$((n+1)); if [ $n -ge 3 ]; then echo \"blank screenshot: {name}\" >&2; break; fi; \
         sleep 10; done"
    )
}

/// The simulators `shots` takes pictures on: the directory under [`SCREENS`] and the
/// simulator's name.
fn shot_devices(remote: &Remote) -> [(&'static str, &str); 2] {
    [("iphone", &remote.simulator), ("ipad", &remote.ipad)]
}

/// On the Mac: runs the app in each simulator and takes a light screenshot of every
/// shot, and a dark one of those that ask for it, into `.screens/<iphone|ipad>/`. The app
/// is removed before it is installed, so a run never starts from data an earlier build
/// left behind (a lane's simulators are clones that carry the base devices' app data).
fn shots(remote: &Remote, list: &[Shot]) -> String {
    capture(remote, list, shot_devices(remote), SCREENS, false)
}

/// The simulators `store-shots` takes pictures on, like [`shot_devices`].
fn store_devices(remote: &Remote) -> [(&'static str, &str); 2] {
    [("iphone", &remote.store), ("ipad", &remote.store_ipad)]
}

/// On the Mac: the light App Store screenshots of `list` in [`STORE_SCREENS`], one directory
/// per device, with the status bar set to 9:41, full battery and signal (and cleared again).
fn store_shots(remote: &Remote, list: &[Shot]) -> String {
    capture(remote, list, store_devices(remote), STORE_SCREENS, true)
}

/// The status bar the store screenshots show: 9:41, charged, full signal. The options are
/// those of `xcrun simctl status_bar <device> override`.
fn status_bar_override(sim: &str) -> String {
    format!(
        "xcrun simctl status_bar {sim} override --time 9:41 --batteryState charged \
         --batteryLevel 100 --wifiMode active --wifiBars 3 --cellularMode active \
         --cellularBars 4 --operatorName ''"
    )
}

/// On the Mac: runs the app in each of `devices` (a directory label and a simulator) and takes
/// the shots of `list` into `<screens>/<label>/`. With `store` the status bar is overridden and
/// only light shots are taken.
fn capture(
    remote: &Remote,
    list: &[Shot],
    devices: [(&str, &str); 2],
    screens: &str,
    store: bool,
) -> String {
    let app = format!("{DERIVED}/Build/Products/Debug-iphonesimulator/{APP}");
    let mut steps = vec![format!("rm -rf {screens}")];
    for (label, name) in devices {
        let sim = quote(name);
        let dir = format!("{screens}/{label}");
        steps.push(format!(
            "mkdir -p {dir}; xcrun simctl boot {sim} 2>/dev/null || true; \
             xcrun simctl bootstatus {sim} -b >/dev/null; xcrun simctl ui {sim} appearance light; \
             xcrun simctl uninstall {sim} {BUNDLE_ID} 2>/dev/null || true; \
             xcrun simctl install {sim} {app}"
        ));
        if store {
            steps.push(status_bar_override(&sim));
        }
        for (i, shot) in list.iter().enumerate() {
            // The first launch fetches; later ones start from the cache.
            steps.push(launch_line(
                &sim,
                &dir,
                &shot.name,
                &shot.args,
                shot.wait_after_launch(if i == 0 { 12 } else { 8 }),
            ));
            if shot.dark && !store {
                // The appearance is set while the app is not running, and read back
                // before the launch: a running app can keep its old look.
                steps.push(format!(
                    "xcrun simctl terminate {sim} {BUNDLE_ID} 2>/dev/null || true; \
                     xcrun simctl ui {sim} appearance dark; sleep 2; \
                     [ \"$(xcrun simctl ui {sim} appearance)\" = dark ] || \
                     {{ echo \"appearance did not switch to dark\" >&2; }}; {}; \
                     xcrun simctl terminate {sim} {BUNDLE_ID} 2>/dev/null || true; \
                     xcrun simctl ui {sim} appearance light",
                    launch_line(
                        &sim,
                        &dir,
                        &format!("{}-dark", shot.name),
                        &shot.args,
                        shot.wait_after_launch(if i == 0 { 12 } else { 8 })
                    )
                ));
            }
        }
        if store {
            steps.push(format!("xcrun simctl status_bar {sim} clear"));
        }
    }
    if store {
        // The simulator's screenshots carry an alpha channel; App Store Connect refuses it.
        steps.push(format!(
            "xcrun swift {DROP_ALPHA} {screens}/*/*.png || echo \"could not drop the alpha channel\" >&2"
        ));
    }
    in_dir(remote, &steps.join("; "))
}

/// On the Mac: the command, run in the directory with Xcode selected.
fn exec(remote: &Remote, command: &[String]) -> String {
    let words: Vec<String> = command.iter().map(|a| quote(a)).collect();
    in_dir(remote, &words.join(" "))
}

/// The UI test target the `ui` step runs.
const UI_TESTS: &str = "UnderbiteUITests";

/// The simulator's accessibility switches the test runner waits for: without them a run can
/// stop with "Timed out waiting for AX loaded notification" before its first test.
const ACCESSIBILITY_DEFAULTS: [&str; 3] = [
    "AccessibilityEnabled",
    "ApplicationAccessibilityEnabled",
    "AutomationEnabled",
];

/// On the Mac: for each of `devices` (a label and a simulator), switches its accessibility on,
/// boots it again so it starts with them (a switch changed under a running simulator can leave
/// the runner unable to set accessibility up), sets the light appearance and runs the UI tests
/// (`only`: one class or test of them), then shuts it down so the next one runs alone. Every
/// device is tested even when one fails; the line fails if any did, and says which.
fn ui_tests(remote: &Remote, devices: &[(&str, &str)], only: Option<&str>) -> String {
    let target = only.map_or_else(|| UI_TESTS.to_string(), |t| format!("{UI_TESTS}/{t}"));
    let mut steps = vec!["failed=".to_string()];
    for (label, name) in devices {
        let sim = quote(name);
        let destination = quote(&format!("platform=iOS Simulator,name={name}"));
        let defaults = ACCESSIBILITY_DEFAULTS
            .iter()
            .map(|key| {
                format!(
                    "xcrun simctl spawn {sim} defaults write com.apple.Accessibility {key} -bool true"
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let boot = format!(
            "xcrun simctl boot {sim} 2>/dev/null || true; xcrun simctl bootstatus {sim} -b >/dev/null"
        );
        steps.push(format!(
            "{boot}; {defaults}; xcrun simctl shutdown {sim}; {boot}; \
             xcrun simctl ui {sim} appearance light; echo \"UI tests on {label}\"; \
             xcodebuild test -project {PROJECT} -scheme {SCHEME} {} -destination {destination} \
             -derivedDataPath {DERIVED} || failed=\"$failed {label}\"; \
             xcrun simctl shutdown {sim} 2>/dev/null || true",
            quote(&format!("-only-testing:{target}"))
        ));
    }
    steps.push(
        "if [ -n \"$failed\" ]; then echo \"UI tests failed on:$failed\" >&2; exit 1; fi; \
         echo 'UI tests passed'"
            .to_string(),
    );
    in_dir(remote, &steps.join("; "))
}

/// The simulators `ui` tests on: both, or the one `--on` names.
fn ui_devices<'a>(remote: &'a Remote, on: Option<&str>) -> Vec<(&'static str, &'a str)> {
    shot_devices(remote)
        .into_iter()
        .filter(|(label, _)| on.is_none_or(|on| on == *label))
        .collect()
}

/// The signed app for the iPhone, installed and started. Signing
/// uses the Apple ID signed in to Xcode (`-allowProvisioningUpdates`).
/// Signing needs the login keychain, which an SSH session finds locked:
/// `security` asks for the account's password on the terminal (`ssh -t`), or
/// reads it from standard input (`keychain_password_command`), which is then
/// closed so nothing after it reads the password.
const UNLOCK: &str = "security show-keychain-info login.keychain-db >/dev/null 2>&1 || \
     security unlock-keychain login.keychain-db || exit 1; exec </dev/null; ";

/// Writes `team` into `ios/Underbite/Signing.local.xcconfig` on the Mac, keeping
/// the file's other lines (a bundle identifier override).
fn team_line(remote: &Remote) -> String {
    remote.team.as_deref().map_or_else(String::new, |team| {
        let file = "ios/Underbite/Signing.local.xcconfig";
        format!(
            "touch {file}; {{ grep -v '^DEVELOPMENT_TEAM' {file} || true; \
             printf 'DEVELOPMENT_TEAM = %s\\n' {}; }} > {file}.new; mv {file}.new {file}; ",
            quote(team)
        )
    })
}

/// The build, install and launch on `iphone`. Built for that phone, so automatic signing
/// registers a new one with the team (`-allowProvisioningDeviceRegistration`); `release`
/// optimizes the app.
fn device(remote: &Remote, iphone: &str, release: bool) -> String {
    let team = team_line(remote);
    let phone = quote(iphone);
    let destination = quote(&format!("platform=iOS,name={iphone}"));
    let configuration = if release { "Release" } else { "Debug" };
    in_dir(
        remote,
        &format!(
            "{team}{UNLOCK}xcodebuild -quiet -project {PROJECT} -scheme {SCHEME} \
             -configuration {configuration} -destination {destination} -derivedDataPath {DERIVED} \
             -allowProvisioningUpdates -allowProvisioningDeviceRegistration build; \
             xcrun devicectl device install app --device {phone} \
             {DERIVED}/Build/Products/{configuration}-iphoneos/{APP}; \
             xcrun devicectl device process launch --device {phone} {BUNDLE_ID} >/dev/null 2>&1 \
             && echo 'installed and started' \
             || echo 'installed; it did not start (is the phone locked?), so open it on the phone'"
        ),
    )
}

/// On the Mac: a Release archive of the app (`.archive/Underbite.xcarchive`). Signed (with the
/// team from the configuration, through automatic signing, like `device`) unless `unsigned`.
/// Nothing is exported or uploaded: the archive stays in the lane's directory, and its size is
/// printed.
fn archive(remote: &Remote, unsigned: bool) -> String {
    let path = format!("{ARCHIVE_DIR}/{ARCHIVE_NAME}");
    let (prefix, signing) = if unsigned {
        (String::new(), "CODE_SIGNING_ALLOWED=NO")
    } else {
        (
            format!("{}{UNLOCK}", team_line(remote)),
            "-allowProvisioningUpdates",
        )
    };
    in_dir(
        remote,
        &format!(
            "{prefix}rm -rf {ARCHIVE_DIR}; xcodebuild -quiet -project {PROJECT} -scheme {SCHEME} \
             -configuration Release -destination generic/platform=iOS -derivedDataPath {DERIVED} \
             -archivePath {path} {signing} archive || exit 1; \
             du -sh {path}; echo \"archive: $PWD/{path} ({})\"",
            if unsigned { "unsigned" } else { "signed" }
        ),
    )
}

/// Copies the screenshots in `dir` (on the Mac) back to `local`.
fn fetch_screens(remote: &Remote, dir: &str, local: &str) -> Vec<String> {
    vec![
        "-az".into(),
        "--delete".into(),
        format!("{}:{}/{dir}/", remote.host, remote.dir),
        format!("{local}/"),
    ]
}

/// Copies `path` from the Mac's directory to the same place here (`/./` marks where
/// the relative part starts).
fn fetch_path(remote: &Remote, path: &str) -> Vec<String> {
    vec![
        "-az".into(),
        "--relative".into(),
        format!("{}:{}/./{path}", remote.host, remote.dir),
        "./".into(),
    ]
}

/// What `doctor` runs on the Mac: sections that [`parse_doctor`] reads.
fn doctor_script(remote: &Remote) -> String {
    format!(
        "{}; echo '@@ macOS'; sw_vers; echo '@@ xcodebuild'; xcodebuild -version; \
         echo '@@ sdks'; xcodebuild -showsdks; echo '@@ runtimes'; xcrun simctl list runtimes; \
         echo '@@ device types'; xcrun simctl list devicetypes | grep -E '^iP(hone|ad)'; \
         echo '@@ swift'; xcrun swift --version 2>&1 | head -1; \
         echo '@@ disk'; df -Pk /Users/Shared | tail -1",
        developer_dir(remote)
    )
}

/// The newest `major` in the lines that carry `marker` followed by digits.
fn newest_major<'a>(lines: impl Iterator<Item = &'a str>, marker: &str) -> Option<u32> {
    lines
        .filter(|line| !line.contains("unavailable"))
        .filter_map(|line| {
            let rest = &line[line.find(marker)? + marker.len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u32>().ok()
        })
        .max()
}

/// What the doctor found.
#[derive(Debug, PartialEq, Eq)]
struct Findings {
    /// The newest iOS device SDK's major version.
    sdk: Option<u32>,
    /// The newest iOS Simulator SDK's major version.
    simulator_sdk: Option<u32>,
    /// The newest available iOS simulator runtime's major version.
    runtime: Option<u32>,
    /// The free space of the Mac's data volume, in KiB.
    free_kib: Option<u64>,
    /// The device types (the whole line, `name (identifier)`).
    device_types: Vec<String>,
}

/// The lines of `output` between the section `@@ name` and the next one.
fn section<'a>(output: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("@@ {name}");
    output
        .lines()
        .skip_while(|line| line.trim_end() != header)
        .skip(1)
        .take_while(|line| !line.starts_with("@@ "))
        .collect()
}

fn parse_doctor(output: &str) -> Findings {
    let sdks = section(output, "sdks");
    let runtimes = section(output, "runtimes");
    Findings {
        sdk: newest_major(sdks.iter().copied(), "-sdk iphoneos"),
        simulator_sdk: newest_major(sdks.iter().copied(), "-sdk iphonesimulator"),
        runtime: newest_major(
            runtimes
                .iter()
                .copied()
                .filter(|line| line.starts_with("iOS ")),
            "SimRuntime.iOS-",
        ),
        free_kib: section(output, "disk")
            .iter()
            .find_map(|line| line.split_whitespace().nth(3)?.parse().ok()),
        device_types: section(output, "device types")
            .iter()
            .map(|line| (*line).to_owned())
            .collect(),
    }
}

/// The device type identifier of the simulator `name` (`name (com.apple…)`).
fn device_type<'a>(findings: &'a Findings, name: &str) -> Option<&'a str> {
    findings.device_types.iter().find_map(|line| {
        let rest = line.strip_prefix(name)?.strip_prefix(" (")?;
        rest.strip_suffix(')')
            .filter(|id| id.starts_with("com.apple."))
    })
}

/// The steps that install what the doctor found missing.
const XCODE_STEPS: &str = "\
On the Mac: install Xcode 26 or later (App Store, or developer.apple.com/download); open it once
and accept the license (or `sudo xcodebuild -license accept` and `xcodebuild -runFirstLaunch`);
install the iOS simulator runtime (Xcode > Settings > Components, or `xcodebuild -downloadPlatform
iOS`: check its spelling with `xcodebuild -help`); if Xcode is not at /Applications/Xcode.app, set
`xcode = \"<path>\"` in ios/remote.local.toml; then run the doctor again.";

/// The problems (`Err`) and the warnings (`Ok`) in `findings`; `sims` are the configured iPhone and
/// iPad simulators.
fn assess(findings: &Findings, sims: [&str; 2]) -> Result<Vec<String>, String> {
    let mut missing = Vec::new();
    let enough = |version: Option<u32>| version.is_some_and(|v| v >= MIN_IOS);
    if !enough(findings.sdk) {
        missing.push(format!("an iOS {MIN_IOS} (or later) SDK"));
    }
    if !enough(findings.simulator_sdk) {
        missing.push(format!("an iOS {MIN_IOS} (or later) Simulator SDK"));
    }
    if !enough(findings.runtime) {
        missing.push(format!("an iOS {MIN_IOS} (or later) simulator runtime"));
    }
    if !missing.is_empty() {
        return Err(format!(
            "the Mac lacks {}\n{XCODE_STEPS}",
            missing.join(", ")
        ));
    }
    let mut warnings = Vec::new();
    match findings.free_kib {
        Some(kib) if kib / (1024 * 1024) < MIN_FREE_GIB => warnings.push(format!(
            "only {:.1} GiB is free on the Mac (builds and cloned simulators need {MIN_FREE_GIB} GiB or more)",
            kib as f64 / (1024.0 * 1024.0)
        )),
        Some(_) => {}
        None => warnings.push("could not read the Mac's free disk space".to_string()),
    }
    for name in sims {
        if device_type(findings, name).is_none() {
            warnings.push(format!(
                "no simulator device type is named `{name}` (set `simulator` / `ipad_simulator` in ios/remote.local.toml)"
            ));
        }
    }
    Ok(warnings)
}

/// `doctor`: runs the script on the Mac, prints what it says, and checks the toolchain.
fn doctor(remote: &Remote) -> Result<(), String> {
    let line = doctor_script(remote);
    println!("$ ssh {} {line}", remote.host);
    let out = Command::new("ssh")
        .args([remote.host.as_str(), line.as_str()])
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("failed to start `ssh`: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    print!("{text}");
    if !out.status.success() {
        return Err(format!(
            "`ssh` failed with {} (is Xcode at {}?)\n{XCODE_STEPS}",
            out.status, remote.xcode
        ));
    }
    let findings = parse_doctor(&text);
    let warnings = assess(&findings, [&remote.config_simulator, &remote.base_ipad])?;
    println!("@@ chosen simulators");
    for (kind, name) in [
        ("iPhone", &remote.config_simulator),
        ("iPad", &remote.base_ipad),
    ] {
        println!(
            "{kind}: {name} ({})",
            device_type(&findings, name).unwrap_or("no such device type")
        );
    }
    for warning in &warnings {
        println!("warning: {warning}");
    }
    println!(
        "ok: iOS {} SDK, iOS {} simulator runtime",
        findings.sdk.unwrap_or_default(),
        findings.runtime.unwrap_or_default()
    );
    Ok(())
}

/// The steps for a run, then the fetches, which run whether or not the steps
/// succeeded: `(program, arguments)`.
type Plan = (Vec<(String, Vec<String>)>, Vec<(String, Vec<String>)>);

/// The commands for `args`, after the local build (`stage`: where it is staged), with the
/// `shots` to take.
fn plan(args: &Args, remote: &Remote, stage: &str, list: &[Shot]) -> Plan {
    let ssh = |line: String| ("ssh".to_string(), vec![remote.host.clone(), line]);
    if args.step == Step::Clean {
        // `parse` checked the lane arguments.
        let which = lane::which(args.lane.as_deref(), args.all_lanes).unwrap_or(Which::All);
        // Under `sh`: the Mac's login shell (zsh) errors on a glob that matches nothing.
        let sh = |line: String| ssh(posix(&line));
        return (
            vec![
                sh(lane::remove_dirs(&remote.base_dir, &which, MARKER)),
                sh(delete_simulators(remote, &which)),
            ],
            Vec::new(),
        );
    }
    if args.step == Step::Doctor {
        return (vec![ssh(doctor_script(remote))], Vec::new());
    }
    let mut steps = vec![
        ssh(claim(remote)),
        ("rsync".to_string(), rsync_args(remote)),
    ];
    let mut fetches = Vec::new();
    if args.step.packages() {
        steps.push(("rsync".to_string(), rsync_stage(remote, stage)));
        steps.push(ssh(package(remote)));
    }
    if args.step == Step::Test {
        steps.push(ssh(in_dir(remote, &format!("cd {KIT}; swift test"))));
    }
    if matches!(args.step, Step::App | Step::Shots) {
        // The lane's own simulators, cloned on first use (and a missing one made).
        steps.push(ssh(ensure_simulators(remote)));
        steps.push(ssh(app(remote, &remote.simulator)));
    }
    if args.step == Step::StoreShots {
        steps.push(ssh(ensure_store_simulators(remote)));
        steps.push(ssh(app(remote, &remote.store)));
    }
    if args.step == Step::Archive {
        let mut line = vec![remote.host.clone(), archive(remote, args.unsigned)];
        if !args.unsigned && remote.keychain_password_command.is_none() {
            // A terminal, so the keychain can ask for the password.
            line.insert(0, "-t".into());
        }
        steps.push(("ssh".to_string(), line));
    }
    if args.step == Step::Ui {
        steps.push(ssh(ensure_simulators(remote)));
        steps.push(ssh(ui_tests(
            remote,
            &ui_devices(remote, args.on.as_deref()),
            args.only.as_deref(),
        )));
    }
    if args.step == Step::Device {
        match args.iphone.as_ref().or(remote.iphone.as_ref()) {
            // A terminal, so the keychain can ask for the password, unless
            // `run` pipes it in.
            Some(iphone) => {
                let mut line = vec![remote.host.clone(), device(remote, iphone, args.release)];
                if remote.keychain_password_command.is_none() {
                    line.insert(0, "-t".into());
                }
                steps.push(("ssh".to_string(), line));
            }
            None => steps.push(ssh(format!(
                "{}; echo 'name your iPhone in ios/remote.local.toml (iphone = \"…\"); paired devices:'; \
                 xcrun devicectl list devices; exit 2",
                developer_dir(remote)
            ))),
        }
    }
    if args.step == Step::Exec {
        steps.push(ssh(exec(remote, &args.command)));
        fetches.extend(
            args.fetch
                .iter()
                .map(|path| ("rsync".to_string(), fetch_path(remote, path))),
        );
    }
    if args.step == Step::Shots {
        steps.push(ssh(shots(remote, list)));
        let local = workspace_root().join("target/ios-screens");
        fetches.push((
            "rsync".to_string(),
            fetch_screens(remote, SCREENS, &local.to_string_lossy()),
        ));
    }
    if args.step == Step::StoreShots {
        steps.push(ssh(store_shots(remote, list)));
        let local = workspace_root().join(STORE_LOCAL);
        fetches.push((
            "rsync".to_string(),
            fetch_screens(remote, STORE_SCREENS, &local.to_string_lossy()),
        ));
    }
    (steps, fetches)
}

/// The Mac account's password, from `get` (run by `sh` here). It is never
/// printed.
fn keychain_password(get: &str) -> Result<String, String> {
    let out = Command::new("sh")
        .args(["-c", get])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("keychain_password_command: {e}"))?;
    let text = String::from_utf8(out.stdout)
        .map_err(|_| "keychain_password_command printed something that is not UTF-8".to_string())?;
    let password = text.trim_end_matches(['\n', '\r']);
    if !out.status.success() || password.is_empty() {
        return Err(format!(
            "keychain_password_command (`{get}`) gave no password ({})",
            out.status
        ));
    }
    Ok(password.to_string())
}

/// Runs `command` with `input` and a newline on its standard input.
fn with_input(command: &mut Command, input: &str) -> std::io::Result<std::process::ExitStatus> {
    let mut child = command.stdin(Stdio::piped()).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(format!("{input}\n").as_bytes())?;
    }
    child.wait()
}

/// What the build host runs to stage the core: the targets, then `cargo xtask ios build`.
fn host_build_line(release: bool) -> String {
    let debug = if release { "" } else { " --debug" };
    format!(
        "rustup target add {} >/dev/null 2>&1 || true; cargo xtask ios build{debug}",
        crate::ios::TARGETS.join(" ")
    )
}

/// The arguments of `cargo xtask remote run` that build the core on the build host and
/// copy its stage back here.
fn host_stage_args(release: bool) -> Vec<String> {
    [
        "remote",
        "run",
        "--fetch",
        "target/ios-stage",
        "--",
        "sh",
        "-c",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .chain([host_build_line(release)])
    .collect()
}

/// Builds the core on the build host (through `cargo xtask remote run`, which syncs the
/// tree and its vendored dependencies) and returns the stage copied back here.
fn stage_on_host(release: bool) -> Result<PathBuf, String> {
    let local = crate::ios::stage_dir();
    if local.exists() {
        std::fs::remove_dir_all(&local).map_err(|e| format!("{}: {e}", local.display()))?;
    }
    let exe = std::env::current_exe().map_err(|e| format!("finding xtask: {e}"))?;
    let arguments = host_stage_args(release);
    println!("$ xtask {}", arguments.join(" "));
    let status = Command::new(exe)
        .args(&arguments)
        .current_dir(workspace_root())
        .status()
        .map_err(|e| format!("failed to start xtask: {e}"))?;
    if status.success() {
        Ok(local)
    } else {
        Err(format!(
            "building the core on the build host failed with {status}"
        ))
    }
}

pub(crate) fn run(args: &[&str]) -> Result<(), String> {
    let args = parse(args)?;
    let file = local_file()?;
    if args.dry_run {
        match &file.path {
            Some(path) => println!("config: {}", path.display()),
            None => {
                println!("config: none (no ios/remote.local.toml here or in the main worktree)");
            }
        }
    }
    let remote = remote(&args, &|k| std::env::var(k).ok(), file, lane::lane())?;
    check_lane(args.step, remote.lane.as_deref())?;
    if args.dry_run {
        match &remote.lane {
            Some(lane) => println!("lane: {lane} (directory {})", remote.dir),
            None => println!("lane: none (the main checkout; directory {})", remote.dir),
        }
        if matches!(
            args.step,
            Step::App | Step::Shots | Step::StoreShots | Step::Archive | Step::Ui
        ) {
            println!("project: {PROJECT}, scheme {SCHEME}, app {APP}, bundle {BUNDLE_ID}");
            let devices = if args.step == Step::StoreShots {
                store_devices(&remote)
            } else {
                shot_devices(&remote)
            };
            for (label, name) in devices {
                println!("simulator ({label}): {name}");
            }
        }
    }
    if args.step == Step::Doctor && !args.dry_run {
        return doctor(&remote);
    }
    let list = if matches!(args.step, Step::Shots | Step::StoreShots) {
        let (dir, list) = if args.step == Step::Shots {
            (SHOTS_DIR, read_shots(&workspace_root().join(SHOTS_DIR))?)
        } else {
            (
                STORE_SHOTS_DIR,
                read_store_shots(&workspace_root().join(STORE_SHOTS_DIR))?,
            )
        };
        if list.is_empty() {
            if !args.dry_run {
                return Err(format!("no screenshots: {dir}/ has no *.args files"));
            }
            println!("shots: none ({dir}/*.args)");
        } else {
            println!(
                "shots: {}",
                list.iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        list
    } else {
        Vec::new()
    };
    let stage = if args.step.packages() && !args.dry_run {
        // The faster build host when one answers, else here.
        if !args.local && crate::remote::preferred().is_some() {
            stage_on_host(!args.debug)?
        } else {
            stage(!args.debug)?
        }
    } else {
        crate::ios::stage_dir()
    };
    let stage = path(&stage)?;
    let (steps, fetches) = plan(&args, &remote, &stage, &list);
    // Held until this function returns, whether the steps succeed or not.
    let _lock = if matches!(args.step, Step::Doctor | Step::Clean) {
        None
    } else {
        lane::lock(
            &remote.host,
            &remote.dir,
            remote.lane.as_deref(),
            args.dry_run,
        )?
    };
    if matches!(args.step, Step::Shots | Step::StoreShots) && !args.dry_run {
        let dir = workspace_root().join(if args.step == Step::Shots {
            "target/ios-screens"
        } else {
            STORE_LOCAL
        });
        std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let mut first_error = None;
    for (program, arguments) in steps {
        println!("$ {program} {}", arguments.join(" "));
        if args.dry_run {
            continue;
        }
        if let Err(e) = execute(&program, &arguments, &args, &remote) {
            first_error = Some(e);
            break;
        }
    }
    // The fetches run whether or not the steps did.
    for (program, arguments) in fetches {
        println!("$ {program} {}", arguments.join(" "));
        if args.dry_run {
            continue;
        }
        if let Err(e) = execute(&program, &arguments, &args, &remote) {
            first_error.get_or_insert(e);
        }
    }
    if args.step == Step::StoreShots && !args.dry_run && first_error.is_none() {
        let root = workspace_root().join(STORE_LOCAL);
        let problems: Vec<String> = [
            ("iphone", &IPHONE_STORE_SIZES[..]),
            ("ipad", &IPAD_STORE_SIZES[..]),
        ]
        .into_iter()
        .flat_map(|(label, sizes)| check_store_pngs(&root.join(label), sizes, list.len()))
        .collect();
        if problems.is_empty() {
            println!("store screenshots: sizes and alpha channels checked");
        } else {
            first_error = Some(problems.join("\n"));
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Runs one planned command, feeding the keychain password to the one that unlocks it.
fn execute(
    program: &str,
    arguments: &[String],
    args: &Args,
    remote: &Remote,
) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(arguments).current_dir(workspace_root());
    let password = match &remote.keychain_password_command {
        Some(get)
            if matches!(args.step, Step::Device | Step::Archive)
                && arguments
                    .last()
                    .is_some_and(|line| line.contains("unlock-keychain")) =>
        {
            Some(keychain_password(get)?)
        }
        _ => None,
    };
    let status = match password {
        Some(password) => with_input(&mut command, &password),
        None => command.status(),
    }
    .map_err(|e| format!("failed to start `{program}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program}` failed with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(step: Step) -> Args {
        Args {
            step,
            host: Some("me@mac.local".into()),
            dir: None,
            debug: false,
            dry_run: false,
            iphone: None,
            release: false,
            local: false,
            lane: None,
            all_lanes: false,
            simulator: None,
            unsigned: false,
            on: None,
            only: None,
            fetch: Vec::new(),
            command: Vec::new(),
        }
    }

    fn shot(name: &str, launch: &str, dark: bool) -> Shot {
        Shot {
            name: name.into(),
            args: launch.split_whitespace().map(str::to_owned).collect(),
            dark,
            wait: None,
        }
    }

    fn lane_remote(lane: Option<&str>) -> Remote {
        let file = LocalFile {
            host: Some("me@mac".into()),
            dir: Some("/Users/Shared/underbite-build".into()),
            ..LocalFile::default()
        };
        let a = Args {
            host: None,
            ..args(Step::App)
        };
        remote(&a, &|_| None, file, lane.map(str::to_owned)).unwrap()
    }

    fn last_line(plan: &Plan) -> &str {
        plan.0.last().unwrap().1.last().unwrap()
    }

    #[test]
    fn parses_steps_and_flags() {
        let a = parse(&[
            "test",
            "--host",
            "me@mac",
            "--dir",
            "/tmp/w",
            "--debug",
            "--dry-run",
            "--iphone",
            "Kid's iPhone",
            "--release",
            "--local",
        ])
        .unwrap();
        assert!(a.release && a.local);
        assert_eq!(a.iphone.as_deref(), Some("Kid's iPhone"));
        assert_eq!(a.step, Step::Test);
        assert_eq!(
            (a.host.as_deref(), a.dir.as_deref(), a.debug, a.dry_run),
            (Some("me@mac"), Some("/tmp/w"), true, true)
        );
        assert!(parse(&["device", "--iphone"]).is_err());
        assert!(parse(&[]).is_err());
        assert!(parse(&["build", "--fast"]).is_err());
        // The entitlement flag is not here.
        assert!(parse(&["device", "--no-weatherkit"]).is_err());
    }

    #[test]
    fn store_shots_and_archive_parse_their_own_flags() {
        let a = parse(&["store-shots", "--simulator", "iPhone 16 Pro Max"]).unwrap();
        assert_eq!(a.step, Step::StoreShots);
        assert_eq!(a.simulator.as_deref(), Some("iPhone 16 Pro Max"));
        let a = parse(&["archive", "--unsigned", "--dry-run"]).unwrap();
        assert!(a.step == Step::Archive && a.unsigned && a.dry_run);
        assert!(!parse(&["archive"]).unwrap().unsigned);
        assert!(parse(&["shots", "--unsigned"]).is_err());
        assert!(parse(&["store-shots", "--unsigned"]).is_err());
        // Nothing here exports or uploads.
        assert!(parse(&["archive", "--export"]).is_err());
        assert!(parse(&["upload"]).is_err());
    }

    #[test]
    fn store_simulators_default_to_a_6_9_inch_iphone_and_a_13_inch_ipad() {
        let r = lane_remote(None);
        assert_eq!(r.store, "iPhone 17 Pro Max");
        assert_eq!(r.store_ipad, "iPad Pro 13-inch (M5)");
        // The ordinary shots keep their simulators.
        assert_eq!(
            (r.simulator.as_str(), r.ipad.as_str()),
            ("iPhone 17", "iPad Air 11-inch (M4)")
        );
        let laned = lane_remote(Some("lane-a"));
        assert_eq!(laned.store, "iPhone 17 Pro Max (lane-a)");
        assert_eq!(laned.store_ipad, "iPad Pro 13-inch (M5) (lane-a)");
        // The configuration names them; --simulator replaces only the iPhone, and only for
        // store-shots.
        let file = || LocalFile {
            host: Some("me@mac".into()),
            store_simulator: Some("iPhone 16 Pro Max".into()),
            store_ipad_simulator: Some("iPad Pro 13-inch (M4)".into()),
            ..LocalFile::default()
        };
        let store = |step, simulator: Option<&str>| {
            let a = Args {
                simulator: simulator.map(str::to_owned),
                host: None,
                ..args(step)
            };
            remote(&a, &|_| None, file(), None).unwrap()
        };
        let r = store(Step::StoreShots, None);
        assert_eq!(
            (r.store.as_str(), r.store_ipad.as_str()),
            ("iPhone 16 Pro Max", "iPad Pro 13-inch (M4)")
        );
        let r = store(Step::StoreShots, Some("iPhone 18 Pro Max"));
        assert_eq!(
            (r.store.as_str(), r.simulator.as_str()),
            ("iPhone 18 Pro Max", "iPhone 17")
        );
        let r = store(Step::Shots, Some("iPhone 18 Pro Max"));
        assert_eq!(
            (r.store.as_str(), r.simulator.as_str()),
            ("iPhone 16 Pro Max", "iPhone 18 Pro Max")
        );
    }

    #[test]
    fn store_shots_run_light_with_the_9_41_status_bar_on_both_store_simulators() {
        let laned = lane_remote(Some("lane-a"));
        let list = [
            shot("01-now", "-seed", false),
            shot("02-tide", "-seed -x", false),
        ];
        let p = plan(&args(Step::StoreShots), &laned, "s", &list);
        let programs: Vec<&str> = p.0.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            programs,
            ["ssh", "rsync", "rsync", "ssh", "ssh", "ssh", "ssh"],
            "claim, sync, stage, package, simulators, app, shots"
        );
        let sims = &p.0[4].1[1];
        for clone in [
            "xcrun simctl clone 'iPhone 17 Pro Max' 'iPhone 17 Pro Max (lane-a)'",
            "xcrun simctl clone 'iPad Pro 13-inch (M5)' 'iPad Pro 13-inch (M5) (lane-a)'",
        ] {
            assert!(sims.contains(clone), "{sims}");
        }
        assert!(
            !sims.contains("'iPhone 17' "),
            "only the store simulators: {sims}"
        );
        assert!(
            p.0[5].1[1].contains("name=iPhone 17 Pro Max (lane-a)"),
            "{:?}",
            p.0[5]
        );
        let line = last_line(&p);
        assert!(line.contains("rm -rf .store-shots"), "{line}");
        for (dir, sim) in [
            (".store-shots/iphone", "'iPhone 17 Pro Max (lane-a)'"),
            (".store-shots/ipad", "'iPad Pro 13-inch (M5) (lane-a)'"),
        ] {
            assert!(line.contains(&format!("mkdir -p {dir}")), "{line}");
            assert!(
                line.contains(&format!(
                    "xcrun simctl status_bar {sim} override --time 9:41"
                )),
                "{line}"
            );
            assert!(
                line.contains(&format!("xcrun simctl status_bar {sim} clear")),
                "{line}"
            );
            assert!(line.contains(&format!("{dir}/01-now.png")), "{line}");
            assert!(line.contains(&format!("{dir}/02-tide.png")), "{line}");
            assert!(
                line.contains(&format!(
                    "xcrun simctl uninstall {sim} dev.savery.underbite"
                )),
                "{line}"
            );
        }
        assert!(
            line.contains("--batteryState charged --batteryLevel 100"),
            "{line}"
        );
        assert!(!line.contains("appearance dark"), "light only: {line}");
        assert!(
            line.contains("xcrun swift ios/shots/store/drop-alpha.swift .store-shots/*/*.png"),
            "{line}"
        );
        assert_eq!(p.1.len(), 1);
        assert!(
            p.1[0].1[3].ends_with("target/ios-store-shots/"),
            "{:?}",
            p.1[0]
        );
        assert!(p.1[0].1[2].ends_with("/.store-shots/"), "{:?}", p.1[0]);
    }

    #[test]
    fn store_shots_are_light_and_at_most_ten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.args"), "-seed\nwait 20\n").unwrap();
        assert_eq!(read_store_shots(dir.path()).unwrap().len(), 1);
        std::fs::write(dir.path().join("b.args"), "-seed\ndark\n").unwrap();
        assert!(
            read_store_shots(dir.path())
                .unwrap_err()
                .contains("light only")
        );
        std::fs::remove_file(dir.path().join("b.args")).unwrap();
        for i in 0..MAX_STORE_SHOTS {
            std::fs::write(dir.path().join(format!("n{i:02}.args")), "-seed\n").unwrap();
        }
        assert!(
            read_store_shots(dir.path())
                .unwrap_err()
                .contains("per device size")
        );
    }

    #[test]
    fn the_repository_store_shots_are_the_six_scenes() {
        let list = read_store_shots(&workspace_root().join(STORE_SHOTS_DIR)).unwrap();
        let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "01-conditions",
                "02-tide-detail",
                "03-map-layers",
                "04-trip-grid",
                "05-catch-log",
                "06-widgets"
            ]
        );
    }

    /// A PNG's first 26 bytes: signature, `IHDR` and the start of its data.
    fn png_head(width: u32, height: u32, color: u8) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, color]);
        out
    }

    #[test]
    fn store_pngs_are_checked_for_size_and_alpha() {
        assert_eq!(png_header(&png_head(1320, 2868, 2)), Some((1320, 2868, 2)));
        assert_eq!(png_header(b"not a png at all, but long enough"), None);
        assert!(has_alpha(6) && has_alpha(4) && !has_alpha(2) && !has_alpha(0));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), png_head(1320, 2868, 2)).unwrap();
        assert_eq!(
            check_store_pngs(dir.path(), &IPHONE_STORE_SIZES, 1),
            Vec::<String>::new()
        );
        // Wrong count, wrong size, alpha.
        assert_eq!(
            check_store_pngs(dir.path(), &IPHONE_STORE_SIZES, 2).len(),
            1
        );
        std::fs::write(dir.path().join("b.png"), png_head(1170, 2532, 6)).unwrap();
        let problems = check_store_pngs(dir.path(), &IPHONE_STORE_SIZES, 2);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("1170x2532")));
        assert!(problems.iter().any(|p| p.contains("alpha")));
        // The accepted sizes are the 6.9-inch iPhone's and the 13-inch iPad's.
        assert!(IPAD_STORE_SIZES.contains(&(2064, 2752)));
        assert!(
            !IPHONE_STORE_SIZES.contains(&(1206, 2622)),
            "that is the 6.1-inch size"
        );
        assert_eq!(
            check_store_pngs(&dir.path().join("none"), &IPAD_STORE_SIZES, 1).len(),
            1
        );
    }

    #[test]
    fn archive_is_a_release_build_that_is_neither_exported_nor_uploaded() {
        let r = Remote {
            team: Some("ABCDE12345".into()),
            ..lane_remote(Some("lane-a"))
        };
        let signed = plan(&args(Step::Archive), &r, "s", &[]);
        assert_eq!(
            signed.0.last().unwrap().1[0],
            "-t",
            "a terminal for the keychain"
        );
        let line = last_line(&signed);
        for needed in [
            "-configuration Release",
            "-destination generic/platform=iOS",
            "-archivePath .archive/Underbite.xcarchive",
            "-scheme Underbite",
            "-allowProvisioningUpdates",
            "security unlock-keychain login.keychain-db",
            "printf 'DEVELOPMENT_TEAM = %s\\n' 'ABCDE12345'",
            "du -sh .archive/Underbite.xcarchive",
        ] {
            assert!(line.contains(needed), "{needed}: {line}");
        }
        assert!(!line.contains("CODE_SIGNING_ALLOWED"), "{line}");
        let unsigned = plan(
            &Args {
                unsigned: true,
                ..args(Step::Archive)
            },
            &r,
            "s",
            &[],
        );
        assert_eq!(
            unsigned.0.last().unwrap().1[0],
            "me@mac",
            "no terminal needed"
        );
        let line = last_line(&unsigned);
        assert!(line.contains("CODE_SIGNING_ALLOWED=NO"), "{line}");
        assert!(
            !line.contains("unlock-keychain") && !line.contains("DEVELOPMENT_TEAM"),
            "{line}"
        );
        // The core is built and packaged first, and nothing is fetched or exported.
        assert_eq!(unsigned.0.len(), 5, "claim, sync, stage, package, archive");
        assert_eq!(unsigned.1, []);
        for text in [line, last_line(&signed)] {
            for forbidden in [
                "exportArchive",
                "altool",
                "notarytool",
                "upload",
                "-exportOptionsPlist",
            ] {
                assert!(!text.contains(forbidden), "{forbidden}: {text}");
            }
        }
        // The signed archive may run in a lane: it installs nothing.
        assert_eq!(check_lane(Step::Archive, Some("lane-a")), Ok(()));
    }

    #[test]
    fn exec_takes_a_command_and_fetch_paths() {
        let a = parse(&[
            "exec",
            "--fetch",
            "out/result/",
            "--fetch",
            "log.txt",
            "--",
            "xcodebuild",
            "-version",
        ])
        .unwrap();
        assert_eq!(a.step, Step::Exec);
        assert_eq!(a.fetch, ["out/result", "log.txt"]);
        assert_eq!(a.command, ["xcodebuild", "-version"]);
        assert!(parse(&["exec"]).is_err(), "a command is needed");
        assert!(parse(&["exec", "--fetch", "x"]).is_err());
        assert!(parse(&["exec", "--fetch", "../x", "--", "ls"]).is_err());
        assert!(parse(&["exec", "--fetch", "/etc", "--", "ls"]).is_err());
        assert!(
            parse(&["sync", "--", "ls"]).is_err(),
            "only exec runs a command"
        );
        assert!(parse(&["build", "--fetch", "x"]).is_err());
    }

    #[test]
    fn exec_runs_the_command_in_the_directory_with_xcode_selected_then_fetches() {
        let r = lane_remote(None);
        let a = Args {
            command: vec!["xcodebuild".into(), "-version".into(), "it's".into()],
            fetch: vec!["build/out".into()],
            ..args(Step::Exec)
        };
        let (steps, fetches) = plan(&a, &r, "s", &[]);
        let programs: Vec<&str> = steps.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            programs,
            ["ssh", "rsync", "ssh"],
            "synced, but no Rust build"
        );
        let line = steps.last().unwrap().1.last().unwrap();
        assert!(
            line.contains("export DEVELOPER_DIR='/Applications/Xcode.app/Contents/Developer'"),
            "{line}"
        );
        assert!(
            line.ends_with(
                r"cd '/Users/Shared/underbite-build'; 'xcodebuild' '-version' 'it'\''s'"
            ),
            "{line}"
        );
        assert_eq!(fetches.len(), 1);
        assert_eq!(
            fetches[0].1,
            [
                "-az",
                "--relative",
                "me@mac:/Users/Shared/underbite-build/./build/out",
                "./"
            ]
        );
    }

    #[test]
    fn the_host_comes_from_flags_then_the_environment() {
        let env = |k: &str| (k == "UNDERBITE_MAC").then(|| "env@mac".to_string());
        let mut a = args(Step::Build);
        assert_eq!(
            remote(&a, &env, LocalFile::default(), None).unwrap().host,
            "me@mac.local"
        );
        a.host = None;
        let r = remote(&a, &env, LocalFile::default(), None).unwrap();
        assert_eq!((r.host.as_str(), r.dir.as_str()), ("env@mac", DEFAULT_DIR));
        a.dir = Some("~".into());
        assert!(
            remote(&a, &env, LocalFile::default(), None).is_err(),
            "never the home directory itself"
        );
        a.dir = None;
        let file = LocalFile {
            host: Some("file@mac".into()),
            dir: Some("w".into()),
            ..LocalFile::default()
        };
        let r = remote(&a, &|_| None, file, None).unwrap();
        assert_eq!((r.host.as_str(), r.dir.as_str()), ("file@mac", "w"));
        assert_eq!(r.xcode, DEFAULT_XCODE);
        assert!(remote(&a, &|_| None, LocalFile::default(), None).is_err());
    }

    #[test]
    fn the_local_file_names_the_mac_the_simulators_and_signing() {
        let f = LocalFile::parse(
            "host = \"me@mac\"\ndir = \"src/underbite\"\nxcode = \"/Applications/Xcode-27.app\"\n\
             simulator = \"iPhone 17 Pro\"\nipad_simulator = \"iPad (A16)\"\n",
        )
        .unwrap();
        assert_eq!(f.xcode.as_deref(), Some("/Applications/Xcode-27.app"));
        assert_eq!(
            (f.host.as_deref(), f.dir.as_deref()),
            (Some("me@mac"), Some("src/underbite"))
        );
        assert_eq!(f.ipad_simulator.as_deref(), Some("iPad (A16)"));
        assert!(LocalFile::parse("hots = \"x\"").is_err());
        assert!(LocalFile::parse("host = 1").is_err());
        let example = include_str!("../../ios/remote.local.toml.example");
        assert!(LocalFile::parse(example).is_ok(), "the example parses");
    }

    #[test]
    fn a_lane_gets_its_own_directory_and_simulators_unless_a_directory_is_named() {
        let main = lane_remote(None);
        assert_eq!(main.dir, "/Users/Shared/underbite-build");
        assert_eq!(main.simulator, "iPhone 17");
        assert_eq!(main.ipad, DEFAULT_IPAD_SIMULATOR);
        assert_eq!(main.lane, None);
        let lane = lane_remote(Some("underbite-lane-a"));
        assert_eq!(lane.dir, "/Users/Shared/underbite-build-underbite-lane-a");
        assert_eq!(lane.base_dir, "/Users/Shared/underbite-build");
        assert_eq!(lane.simulator, "iPhone 17 (underbite-lane-a)");
        assert_eq!(
            lane.ipad,
            format!("{DEFAULT_IPAD_SIMULATOR} (underbite-lane-a)")
        );
        assert_eq!(lane.base_simulator, "iPhone 17");
        // --dir and UNDERBITE_MAC_DIR are used as given, but the simulators stay per lane.
        let named = Args {
            dir: Some("/Users/Shared/mine".into()),
            ..args(Step::App)
        };
        let r = remote(&named, &|_| None, LocalFile::default(), Some("a".into())).unwrap();
        assert_eq!(
            (r.dir.as_str(), r.simulator.as_str()),
            ("/Users/Shared/mine", "iPhone 17 (a)")
        );
        let env = |k: &str| (k == "UNDERBITE_MAC_DIR").then(|| "/m".to_string());
        let bare = Args {
            dir: None,
            ..args(Step::App)
        };
        assert_eq!(
            remote(&bare, &env, LocalFile::default(), Some("a".into()))
                .unwrap()
                .dir,
            "/m"
        );
        // The configured simulators are the bases.
        let file = LocalFile {
            simulator: Some("iPhone 16".into()),
            ipad_simulator: Some("iPad (A16)".into()),
            ..LocalFile::default()
        };
        let r = remote(&bare, &|_| None, file, Some("b".into())).unwrap();
        assert_eq!(r.simulator, "iPhone 16 (b)");
        assert_eq!(r.ipad, "iPad (A16) (b)");
    }

    #[test]
    fn a_lane_clones_both_simulators_for_app_and_shots_but_the_main_checkout_does_not() {
        let laned = lane_remote(Some("underbite-lane-a"));
        let line = ensure_simulators(&laned);
        assert!(line.contains("DEVELOPER_DIR="), "{line}");
        for (base, sim) in [
            ("iPhone 17", "iPhone 17 (underbite-lane-a)"),
            (
                "iPad Air 11-inch (M4)",
                "iPad Air 11-inch (M4) (underbite-lane-a)",
            ),
        ] {
            assert!(
                line.contains(&format!("grep -qF '    {sim} ('")),
                "the whole name is matched: {line}"
            );
            assert!(
                line.contains(&format!(
                    "xcrun simctl shutdown '{base}' 2>/dev/null || true"
                )),
                "{line}"
            );
            assert!(
                line.contains(&format!("xcrun simctl clone '{base}' '{sim}'")),
                "{line}"
            );
        }
        let main = ensure_simulators(&lane_remote(None));
        assert!(!main.contains("simctl clone"), "{main}");
        // A missing configured simulator is made from its device type.
        assert!(
            main.contains("xcrun simctl create 'iPhone 17' \"$t\""),
            "{main}"
        );
        assert!(
            main.contains("xcrun simctl create 'iPad Air 11-inch (M4)'"),
            "{main}"
        );
        // app and shots set the simulators up before building, and build for the lane's iPhone.
        for step in [Step::App, Step::Shots] {
            let p = plan(&args(step), &laned, "s", &[shot("a", "-x", false)]);
            let clone =
                p.0.iter()
                    .position(|(_, a)| a.last().is_some_and(|l| l.contains("simctl clone")))
                    .unwrap();
            let build =
                p.0.iter()
                    .position(|(_, a)| a.last().is_some_and(|l| l.contains("xcodebuild -quiet")))
                    .unwrap();
            assert!(clone < build, "{step:?}");
            let build = p.0[build].1.last().unwrap();
            assert!(
                build.contains(
                    "-destination 'platform=iOS Simulator,name=iPhone 17 (underbite-lane-a)'"
                ),
                "{build}"
            );
            assert!(
                build.contains(&format!("-project {PROJECT} -scheme {SCHEME}")),
                "{build}"
            );
        }
        // The tests run on the Mac itself.
        let test = plan(&args(Step::Test), &laned, "s", &[]);
        assert!(
            test.0
                .iter()
                .all(|(_, a)| !a.last().unwrap().contains("simctl"))
        );
        assert!(
            last_line(&test).ends_with("cd ios/UnderbiteKit; swift test"),
            "{}",
            last_line(&test)
        );
    }

    #[test]
    fn app_is_one_build_and_shots_capture_on_both_simulators() {
        let laned = lane_remote(Some("underbite-lane-a"));
        let list = [shot("now", "-seed -tab now", false), shot("days", "", true)];
        let shots = plan(&args(Step::Shots), &laned, "s", &list);
        let builds = shots
            .0
            .iter()
            .filter(|(_, a)| a.last().is_some_and(|l| l.contains("xcodebuild -quiet")))
            .count();
        assert_eq!(builds, 1, "built once");
        let line = last_line(&shots);
        for sim in [
            "'iPhone 17 (underbite-lane-a)'",
            "'iPad Air 11-inch (M4) (underbite-lane-a)'",
        ] {
            assert!(
                line.contains(&format!("xcrun simctl install {sim} ")),
                "{line}"
            );
            // Each run starts from a fresh install, not from an earlier build's data.
            let uninstall = format!("xcrun simctl uninstall {sim} dev.savery.underbite");
            let install = format!("xcrun simctl install {sim} ");
            assert!(
                line.find(&uninstall)
                    .is_some_and(|u| line.find(&install).is_some_and(|i| u < i)),
                "{line}"
            );
            assert!(
                line.contains(&format!(
                    "simctl launch {sim} dev.savery.underbite '-seed' '-tab' 'now'"
                )),
                "{line}"
            );
            // A shot that asks for it is also taken dark, then the appearance is restored.
            assert!(
                line.contains(&format!("xcrun simctl ui {sim} appearance dark")),
                "{line}"
            );
        }
        // Dark is set with the app stopped, read back, then launched with the light wait.
        let dark = line.find("appearance dark").unwrap();
        let readback = line[dark..].find("appearance)\" = dark").unwrap() + dark;
        let launch = line[readback..].find("simctl launch").unwrap() + readback;
        assert!(
            line[dark - 200..dark].contains("simctl terminate"),
            "{line}"
        );
        assert!(line[launch..].contains("sleep 8;"), "{line}");
        assert!(!line.contains("sleep 5;"), "{line}");
        assert!(line.contains(".screens/iphone/now.png"), "{line}");
        assert!(line.contains(".screens/ipad/days-dark.png"), "{line}");
        assert!(!line.contains("now-dark"), "{line}");
        assert!(
            line.contains(&format!(
                "{DERIVED}/Build/Products/Debug-iphonesimulator/Underbite.app"
            )),
            "{line}"
        );
        // The screenshots are copied back whether or not the capture succeeded.
        assert_eq!(shots.1.len(), 1);
        assert!(shots.1[0].1[2].ends_with("/underbite-build-underbite-lane-a/.screens/"));
        assert!(shots.1[0].1[3].ends_with("target/ios-screens/"));
        let app = plan(&args(Step::App), &laned, "s", &[]);
        assert_eq!(app.1.len(), 0);
        assert!(!last_line(&app).contains("screenshot"));
    }

    #[test]
    fn ui_takes_a_device_and_a_test() {
        let a = parse(&[
            "ui",
            "--on",
            "ipad",
            "--only",
            "AccessibilityAuditTests/testConditions",
        ])
        .unwrap();
        assert_eq!(a.step, Step::Ui);
        assert_eq!(a.on.as_deref(), Some("ipad"));
        assert_eq!(
            a.only.as_deref(),
            Some("AccessibilityAuditTests/testConditions")
        );
        assert!(parse(&["ui", "--on", "watch"]).is_err());
        assert!(parse(&["ui", "--only", "A/b/c"]).is_err());
        assert!(parse(&["ui", "--only", "x; rm -rf /"]).is_err());
        assert!(parse(&["app", "--on", "ipad"]).is_err(), "only ui takes it");
    }

    #[test]
    fn ui_switches_accessibility_on_before_testing_on_both_simulators() {
        let laned = lane_remote(Some("underbite-lane-a"));
        let ui = plan(&args(Step::Ui), &laned, "s", &[]);
        // The core is packaged and the lane's simulators made before the tests.
        assert!(
            ui.0.iter()
                .any(|(_, a)| a.last().is_some_and(|l| l.contains("echo wrote")))
        );
        assert!(ui.0[ui.0.len() - 2].1[1].contains("simctl clone"));
        let line = last_line(&ui);
        let mut from = 0;
        for sim in [
            "'iPhone 17 (underbite-lane-a)'",
            "'iPad Air 11-inch (M4) (underbite-lane-a)'",
        ] {
            let boot = line[from..]
                .find(&format!("xcrun simctl boot {sim}"))
                .unwrap()
                + from;
            let test = line[boot..].find("xcodebuild test").unwrap() + boot;
            for key in [
                "AccessibilityEnabled",
                "ApplicationAccessibilityEnabled",
                "AutomationEnabled",
            ] {
                let write = format!(
                    "xcrun simctl spawn {sim} defaults write com.apple.Accessibility {key} -bool true"
                );
                let at = line[boot..test].find(&write).map(|i| i + boot);
                assert!(at.is_some(), "{key} set before the tests: {line}");
            }
            // Then booted again, so the simulator starts with them, before the tests.
            let last_write = line[boot..test].rfind("defaults write").unwrap() + boot;
            let restart = line[last_write..test]
                .find(&format!(
                    "xcrun simctl shutdown {sim}; xcrun simctl boot {sim}"
                ))
                .map(|i| i + last_write);
            assert!(restart.is_some(), "booted again after the writes: {line}");
            assert!(
                line[restart.unwrap()..test].contains(&format!("xcrun simctl bootstatus {sim} -b")),
                "{line}"
            );
            assert!(
                line[test..].starts_with(&format!(
                    "xcodebuild test -project {PROJECT} -scheme {SCHEME} \
                     '-only-testing:UnderbiteUITests' \
                     -destination 'platform=iOS Simulator,name={}' -derivedDataPath {DERIVED}",
                    sim.trim_matches('\'')
                )),
                "{line}"
            );
            // Shut down after its tests, so the next simulator runs alone.
            assert!(
                line[test..].contains(&format!("xcrun simctl shutdown {sim}")),
                "{line}"
            );
            from = test;
        }
        // Both run even when the first fails, and the run fails if either did.
        assert!(line.contains("|| failed=\"$failed iphone\""), "{line}");
        assert!(line.contains("|| failed=\"$failed ipad\""), "{line}");
        assert!(line.contains("exit 1; fi"), "{line}");
        assert_eq!(ui.1.len(), 0);

        let one = Args {
            on: Some("ipad".into()),
            only: Some("AccessibilityAuditTests/testWelcome".into()),
            ..args(Step::Ui)
        };
        let line = last_line(&plan(&one, &laned, "s", &[])).to_owned();
        assert!(!line.contains("iPhone 17"), "{line}");
        assert!(
            line.contains("'-only-testing:UnderbiteUITests/AccessibilityAuditTests/testWelcome'"),
            "{line}"
        );
    }

    #[test]
    fn a_shot_file_has_launch_arguments_and_an_optional_dark_line() {
        assert_eq!(
            parse_shot("now", "-seed -scrollTo now\n").unwrap(),
            shot("now", "-seed -scrollTo now", false)
        );
        assert_eq!(
            parse_shot("tide-dark", "-tab tides \ndark\n").unwrap(),
            shot("tide-dark", "-tab tides", true)
        );
        assert_eq!(parse_shot("plain", "").unwrap(), shot("plain", "", false));
        assert_eq!(
            parse_shot("blank-second", "-a\n\n").unwrap(),
            shot("blank-second", "-a", false)
        );
        assert!(parse_shot("x", "-a\nlight\n").is_err());
        assert!(parse_shot("x", "-a\ndark\n-b\n").is_err());
        assert!(parse_shot("x", "-a\ndark\ndark\n").is_err());
        assert!(parse_shot("a b", "-a").is_err());
        assert!(parse_shot("a;b", "-a").is_err());
        assert!(parse_shot("", "-a").is_err());
    }

    #[test]
    fn a_shot_file_may_ask_for_a_longer_wait() {
        let one = parse_shot("m", "-a\nwait 60\n").unwrap();
        assert_eq!(one.wait, Some(60));
        assert!(!one.dark);
        let both = parse_shot("m", "-a\nwait  45 \ndark\n").unwrap();
        assert_eq!((both.wait, both.dark), (Some(45), true));
        let swapped = parse_shot("m", "-a\ndark\nwait 180\n").unwrap();
        assert_eq!((swapped.wait, swapped.dark), (Some(180), true));
        assert_eq!(parse_shot("m", "-a\n").unwrap().wait, None);
        for bad in [
            "-a\nwait\n",
            "-a\nwait 0\n",
            "-a\nwait 181\n",
            "-a\nwait -3\n",
            "-a\nwait soon\n",
            "-a\nwait 5 6\n",
            "-a\nwait 5\nwait 6\n",
        ] {
            assert!(parse_shot("m", bad).is_err(), "{bad:?}");
        }
        // A request shorter than the usual wait changes nothing; a longer one wins.
        assert_eq!(one.wait_after_launch(12), 60);
        assert_eq!(
            parse_shot("m", "-a\nwait 5\n")
                .unwrap()
                .wait_after_launch(12),
            12
        );
        assert_eq!(shot("p", "-a", false).wait_after_launch(8), 8);
    }

    #[test]
    fn shots_are_read_sorted_from_args_files_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b-days.args"), "-tab days\ndark\n").unwrap();
        std::fs::write(dir.path().join("a-now.args"), "-seed\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "not a shot").unwrap();
        assert_eq!(
            read_shots(dir.path()).unwrap(),
            [
                shot("a-now", "-seed", false),
                shot("b-days", "-tab days", true)
            ]
        );
        std::fs::write(dir.path().join("c-bad.args"), "-x\nnight\n").unwrap();
        assert!(read_shots(dir.path()).unwrap_err().contains("c-bad.args"));
        assert_eq!(
            read_shots(&dir.path().join("missing")).unwrap(),
            Vec::<Shot>::new()
        );
    }

    #[test]
    fn the_repository_shot_files_parse() {
        read_shots(&workspace_root().join(SHOTS_DIR)).unwrap();
    }

    #[test]
    fn launch_arguments_are_quoted_word_by_word() {
        let args = ["-seed".to_string(), "it's; rm -rf /".to_string()];
        let line = launch_line("'iPhone 17'", "d", "n", &args, 8);
        assert!(
            line.contains(
                r"launch 'iPhone 17' dev.savery.underbite '-seed' 'it'\''s; rm -rf /' >/dev/null"
            ),
            "{line}"
        );
        assert!(
            line.contains("until xcrun simctl launch 'iPhone 17'"),
            "{line}"
        );
    }

    #[test]
    fn device_installs_on_the_phone_named() {
        let r = Remote {
            iphone: Some("Alex's iPhone".into()),
            team: Some("ABCDE12345".into()),
            ..lane_remote(None)
        };
        let other = Args {
            iphone: Some("Sam's iPhone".into()),
            ..args(Step::Device)
        };
        let other = plan(&other, &r, "s", &[]);
        assert_eq!(
            other.0.last().unwrap().1[0],
            "-t",
            "a terminal for the keychain"
        );
        let line = last_line(&other);
        assert!(line.contains(r"--device 'Sam'\''s iPhone'"), "{line}");
        // Built for that phone, so a new one is registered with the team.
        assert!(
            line.contains(r"-destination 'platform=iOS,name=Sam'\''s iPhone'"),
            "{line}"
        );
        assert!(
            line.contains("-allowProvisioningDeviceRegistration"),
            "{line}"
        );
        assert!(
            line.contains("-configuration Debug") && line.contains("Debug-iphoneos/Underbite.app")
        );
        assert!(!line.contains("Alex"), "{line}");
        // The team is added to the local signing file without dropping its other lines.
        assert!(line.contains("grep -v '^DEVELOPMENT_TEAM' ios/Underbite/Signing.local.xcconfig"));
        assert!(
            line.contains("mv ios/Underbite/Signing.local.xcconfig.new"),
            "{line}"
        );
        assert!(
            line.contains("security unlock-keychain login.keychain-db"),
            "{line}"
        );
        let release = Args {
            release: true,
            ..args(Step::Device)
        };
        let release = plan(&release, &r, "s", &[]);
        let line = last_line(&release);
        assert!(line.contains("-configuration Release"), "{line}");
        assert!(
            line.contains(".derived/Build/Products/Release-iphoneos/Underbite.app"),
            "{line}"
        );
        // Without a phone named anywhere, it lists the paired ones and fails.
        let none = Remote { iphone: None, ..r };
        assert!(last_line(&plan(&args(Step::Device), &none, "s", &[])).contains("exit 2"));
    }

    #[test]
    fn device_is_refused_in_a_lane() {
        for step in [
            Step::Doctor,
            Step::Build,
            Step::App,
            Step::Shots,
            Step::Exec,
        ] {
            assert_eq!(
                check_lane(step, Some("underbite-lane-a")),
                Ok(()),
                "{step:?}"
            );
        }
        let e = check_lane(Step::Device, Some("underbite-lane-a")).unwrap_err();
        assert!(
            e.contains("main checkout") && e.contains("underbite-lane-a"),
            "{e}"
        );
        assert_eq!(check_lane(Step::Device, None), Ok(()));
    }

    /// Runs one shot's line in `sh`, with `xcrun`, `wc` and `sleep` stood in
    /// for: the screenshot is blank (small) for the first `blank` tries.
    fn run_shot(blank: u32) -> (u32, String) {
        let line = launch_line("'iPhone 17'", "shots", "now", &["-seed".to_string()], 8);
        let script = format!(
            "tries=0; xcrun() {{ case \"$1 $2\" in 'simctl io') tries=$((tries+1)); \
             mkdir -p shots; : > shots/now.png;; esac; }}; \
             wc() {{ if [ $tries -gt {blank} ]; then echo 1500000; else echo 70000; fi; }}; \
             sleep() {{ :; }}; {line}; echo $tries"
        );
        // The stand-in writes the screenshot file, so in a directory of its own.
        let dir =
            std::env::temp_dir().join(format!("underbite-shot-{}-{blank}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let out = Command::new("sh")
            .args(["-c", &script])
            .current_dir(&dir)
            .output()
            .expect("sh runs");
        let _ = std::fs::remove_dir_all(&dir);
        let tries = String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        (tries, String::from_utf8_lossy(&out.stderr).to_string())
    }

    #[test]
    fn a_blank_screenshot_is_taken_again_and_reported_if_it_stays_blank() {
        assert_eq!(run_shot(0), (1, String::new()), "drawn at once: one shot");
        assert_eq!(run_shot(2), (3, String::new()), "drawn on the third try");
        let (tries, err) = run_shot(9);
        assert_eq!(tries, 3, "three tries at most");
        assert!(err.contains("blank screenshot: now"), "{err}");
    }

    #[test]
    fn clean_removes_the_lanes_directories_and_both_simulators() {
        let one = parse(&["clean", "--lane", "underbite-lane-a"]).unwrap();
        let main = lane_remote(None);
        let dirs = lane::remove_dirs(
            "/Users/Shared/underbite-build",
            &Which::One("underbite-lane-a".into()),
            MARKER,
        );
        assert!(
            dirs.contains("'/Users/Shared/underbite-build-underbite-lane-a'/.underbite-remote"),
            "{dirs}"
        );
        let sims = delete_simulators(&main, &Which::One("underbite-lane-a".into()));
        for sim in [
            "iPhone 17 (underbite-lane-a)",
            "iPad Air 11-inch (M4) (underbite-lane-a)",
        ] {
            assert!(
                sims.contains(&format!("xcrun simctl delete '{sim}'")),
                "{sims}"
            );
        }
        let all = lane::remove_dirs("/Users/Shared/underbite-build", &Which::All, MARKER);
        assert!(all.contains("'/Users/Shared/underbite-build'-*"), "{all}");
        let all_sims = delete_simulators(&main, &Which::All);
        assert!(all_sims.contains("s/^    iPhone 17 (.*) ("), "{all_sims}");
        assert!(
            all_sims.contains("s/^    iPad Air 11-inch (M4) (.*) ("),
            "{all_sims}"
        );
        let p = plan(&one, &lane_remote(Some("underbite-lane-a")), "s", &[]);
        assert_eq!(p.0.len(), 2, "the directory and the simulators");
        assert!(
            p.0.iter()
                .all(|(program, a)| program == "ssh" && a[1].starts_with("sh -c "))
        );
        assert!(parse(&["clean"]).is_err());
        assert!(parse(&["clean", "--lane", "Not A Lane"]).is_err());
        assert!(parse(&["build", "--lane", "a"]).is_err());
        assert!(parse(&["clean", "--all-lanes"]).is_ok());
    }

    #[test]
    fn build_packages_the_staged_core_into_the_framework_and_bindings() {
        let r = lane_remote(None);
        let p = plan(&args(Step::Build), &r, "/stage", &[]);
        let programs: Vec<&str> = p.0.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(programs, ["ssh", "rsync", "rsync", "ssh"]);
        assert_eq!(p.0[2].1[2], "/stage/");
        let package = last_line(&p);
        assert!(
            package.contains(&format!(
                "mkdir -p {SWIFT_DEST}; cp .stage/{SWIFT} {SWIFT_DEST}/{SWIFT}"
            )),
            "{package}"
        );
        assert!(package.contains("'-create-xcframework'"), "{package}");
        assert!(
            package.contains("'.stage/aarch64-apple-ios/libunderbite_ffi.a'"),
            "{package}"
        );
        assert!(package.contains(XCFRAMEWORK), "{package}");
        assert_eq!(SWIFT, "underbite_ffi.swift");
        assert_eq!(SWIFT_DEST, "ios/UnderbiteKit/Sources/UnderbiteKit");
        assert_eq!(XCFRAMEWORK, "ios/UnderbiteKit/UnderbiteFFI.xcframework");
        // Every .gitignore applies, so secrets and build output stay here.
        let rsync = &p.0[1].1;
        assert_eq!(rsync[..2], ["-rlpz", "--checksum"]);
        assert!(rsync.contains(&"--filter=P /.underbite-remote".to_string()));
        assert!(rsync.contains(&"--filter=:- .gitignore".to_string()));
        // The framework and bindings the Mac made survive a later sync.
        assert!(
            rsync.contains(&format!("--filter=P /{XCFRAMEWORK}")),
            "{rsync:?}"
        );
        assert!(
            rsync.contains(&format!("--filter=P /{SWIFT_DEST}/{SWIFT}")),
            "{rsync:?}"
        );
        let sync = plan(&args(Step::Sync), &r, "s", &[]);
        assert_eq!(sync.0.len(), 2);
    }

    #[test]
    fn the_core_is_built_on_the_build_host_through_remote_run() {
        let release = host_stage_args(true);
        assert_eq!(
            &release[..5],
            ["remote", "run", "--fetch", "target/ios-stage", "--"]
        );
        let line = release.last().unwrap();
        assert!(
            line.contains(
                "rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin"
            ),
            "{line}"
        );
        assert!(line.ends_with("cargo xtask ios build"), "{line}");
        assert!(
            host_stage_args(false)
                .last()
                .unwrap()
                .ends_with("cargo xtask ios build --debug")
        );
    }

    const DOCTOR_OUTPUT: &str = "\
@@ macOS
ProductName:\t\tmacOS
ProductVersion:\t\t27.2
@@ xcodebuild
Xcode 27.0
Build version 27A266a
@@ sdks
iOS SDKs:
\tiOS 27.0                      \t-sdk iphoneos27.0

iOS Simulator SDKs:
\tSimulator - iOS 27.0          \t-sdk iphonesimulator27.0

macOS SDKs:
\tmacOS 27.0                    \t-sdk macosx27.0
@@ runtimes
== Runtimes ==
iOS 27.0 (27.0 - 24A434) - com.apple.CoreSimulator.SimRuntime.iOS-27-0
@@ device types
iPhone 17 (com.apple.CoreSimulator.SimDeviceType.iPhone-17)
iPad Air 11-inch (M4) (com.apple.CoreSimulator.SimDeviceType.iPad-Air-11-inch-M4)
iPad Pro 13-inch (M5) (16GB) (com.apple.CoreSimulator.SimDeviceType.iPad-Pro-13-inch-M5-16GB)
@@ swift
Apple Swift version 6.3
@@ disk
/dev/disk3s5   239362496 196663680 8611924    96%    /System/Volumes/Data
";

    #[test]
    fn the_doctor_reads_sdks_runtimes_disk_and_device_types() {
        let f = parse_doctor(DOCTOR_OUTPUT);
        assert_eq!(f.sdk, Some(27));
        assert_eq!(f.simulator_sdk, Some(27));
        assert_eq!(f.runtime, Some(27));
        assert_eq!(f.free_kib, Some(8_611_924));
        assert_eq!(f.device_types.len(), 3);
        assert_eq!(
            device_type(&f, "iPhone 17"),
            Some("com.apple.CoreSimulator.SimDeviceType.iPhone-17")
        );
        assert_eq!(
            device_type(&f, "iPad Pro 13-inch (M5)"),
            None,
            "the whole name, not a prefix"
        );
        assert_eq!(
            device_type(&f, "iPad Air 11-inch (M4)"),
            Some("com.apple.CoreSimulator.SimDeviceType.iPad-Air-11-inch-M4")
        );
    }

    #[test]
    fn the_doctor_accepts_ios_26_or_later_and_warns_about_disk() {
        let f = parse_doctor(DOCTOR_OUTPUT);
        let warnings = assess(&f, ["iPhone 17", "iPad Air 11-inch (M4)"]).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("8.2 GiB") && warnings[0].contains("15 GiB"),
            "{warnings:?}"
        );
        let roomy = Findings {
            free_kib: Some(40 * 1024 * 1024),
            ..parse_doctor(DOCTOR_OUTPUT)
        };
        assert_eq!(
            assess(&roomy, ["iPhone 17", "iPad Air 11-inch (M4)"]),
            Ok(vec![])
        );
        let odd = assess(&roomy, ["iPhone 99", "iPad Air 11-inch (M4)"]).unwrap();
        assert!(odd[0].contains("`iPhone 99`"), "{odd:?}");
        // An exact 26 is enough: the app's minimum, not the SDK the Mac happens to have.
        let old = DOCTOR_OUTPUT
            .replace("iphoneos27.0", "iphoneos26.0")
            .replace("iphonesimulator27.0", "iphonesimulator26.0")
            .replace("iOS-27-0", "iOS-26-0");
        assert_eq!(parse_doctor(&old).sdk, Some(26));
        assert!(assess(&parse_doctor(&old), ["iPhone 17", "x"]).is_ok());
    }

    #[test]
    fn the_doctor_fails_without_an_ios_26_sdk_or_runtime_and_says_what_to_install() {
        let too_old = DOCTOR_OUTPUT
            .replace("iphoneos27.0", "iphoneos18.4")
            .replace("iphonesimulator27.0", "iphonesimulator18.4")
            .replace("iOS-27-0", "iOS-18-4");
        let e = assess(&parse_doctor(&too_old), ["iPhone 17", "x"]).unwrap_err();
        for wanted in [
            "iOS 26 (or later) SDK",
            "Simulator SDK",
            "simulator runtime",
            "Xcode",
            "xcodebuild -runFirstLaunch",
            "ios/remote.local.toml",
        ] {
            assert!(e.contains(wanted), "{wanted}: {e}");
        }
        let no_runtime = DOCTOR_OUTPUT.replace(
            "iOS 27.0 (27.0 - 24A434) - com.apple.CoreSimulator.SimRuntime.iOS-27-0",
            "iOS 26.0 (26.0 - 23A1) - com.apple.CoreSimulator.SimRuntime.iOS-26-0 (unavailable, runtime profile not found)",
        );
        let e = assess(&parse_doctor(&no_runtime), ["iPhone 17", "x"]).unwrap_err();
        assert!(
            e.starts_with("the Mac lacks an iOS 26 (or later) simulator runtime"),
            "{e}"
        );
        let nothing = assess(&parse_doctor(""), ["a", "b"]).unwrap_err();
        assert!(
            nothing.contains("SDK") && nothing.contains("runtime"),
            "{nothing}"
        );
        // A tvOS runtime is no iOS runtime.
        let tv =
            "@@ runtimes\ntvOS 27.0 (27.0 - 1) - com.apple.CoreSimulator.SimRuntime.tvOS-27-0\n";
        assert_eq!(parse_doctor(tv).runtime, None);
    }

    #[test]
    fn doctor_runs_one_script_with_every_section() {
        let r = lane_remote(None);
        let p = plan(&args(Step::Doctor), &r, "s", &[]);
        assert_eq!(p.0.len(), 1);
        let script = last_line(&p);
        for wanted in [
            "DEVELOPER_DIR",
            "sw_vers",
            "xcodebuild -version",
            "xcodebuild -showsdks",
            "xcrun simctl list runtimes",
            "xcrun simctl list devicetypes",
            "df -Pk",
        ] {
            assert!(script.contains(wanted), "{wanted}: {script}");
        }
    }
}
