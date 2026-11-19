# iOS app

The iPhone and iPad app is a SwiftUI shell over the Rust core (`crates/underbite-ffi`, through
UniFFI). The project is `ios/Underbite/Underbite.xcodeproj` (scheme `Underbite`, bundle identifier
`dev.savery.underbite`); the Swift package that wraps the core is `ios/UnderbiteKit`. The app's
minimum is iOS 26; Xcode 26 or later builds it.

## On the Mac

1. Install Xcode and Rust, then the iOS targets:
   `rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin`
2. From the repository root: `cargo xtask ios build`. It writes
   - `ios/UnderbiteKit/UnderbiteFFI.xcframework` (iPhone, simulator and Mac slices),
   - `ios/UnderbiteKit/Sources/UnderbiteKit/underbite_ffi.swift` (the generated bindings).
   Both are build output and not committed.
3. `cd ios/UnderbiteKit && swift test` runs the package tests on the Mac.

On Linux the same command builds the iOS static libraries and the bindings (useful as a check);
only the XCFramework needs `xcodebuild`.

## Building on the Mac from Linux (over SSH)

Name the Mac once, in `ios/remote.local.toml` (gitignored, like every `*.local.toml`; copy
`ios/remote.local.toml.example`):

```toml
host = "you@your-mac.local"             # anything `ssh` accepts, including a Host alias
dir = "/Users/Shared/underbite-build"   # on the Mac; relative paths are under its home
# simulator = "iPhone 17"               # the iPhone simulator (the default)
# ipad_simulator = "iPad Air 11-inch (M4)"   # the iPad simulator (the default)
# store_simulator = "iPhone 17 Pro Max"         # store-shots: a 6.9-inch iPhone (the default)
# store_ipad_simulator = "iPad Pro 13-inch (M5)"   # store-shots: a 13-inch iPad (the default)
```

or pass `--host`, or set `UNDERBITE_MAC`. Then, from the repository root:

| Command | Does |
|---|---|
| `cargo xtask ios remote doctor` | shows macOS, Xcode, the SDKs, simulator runtimes, device types and free disk; fails unless an iOS 26 (or later) SDK and simulator runtime are installed, warns under 15 GB free |
| `cargo xtask ios remote sync` | copies the working tree to the Mac |
| `cargo xtask ios remote build` | builds the Rust core (on the build host), syncs, makes the XCFramework on the Mac |
| `cargo xtask ios remote test` | build, then `swift test` in `ios/UnderbiteKit` |
| `cargo xtask ios remote app` | build, then the app for the iPhone simulator, once |
| `cargo xtask ios remote shots` | app, then run it in the iPhone and the iPad simulator and take a screenshot for each `ios/shots/*.args` file; they land in `target/ios-screens/iphone/` and `target/ios-screens/ipad/` (see `ios/shots/README.md`; a `wait <seconds>` line in an `.args` file, up to 180, waits longer before that shot, and `-seedLimits` adds a sample limits profile) |
| `cargo xtask ios remote store-shots [--simulator name]` | app, then the App Store screenshots: one per `ios/shots/store/*.args` file (six: conditions, tide detail, map with layers, trip grid, catch log, widgets), light, with the status bar at 9:41 (full battery and signal), on a 6.9-inch iPhone simulator (`store_simulator`, default `iPhone 17 Pro Max`, 1320x2868) and a 13-inch iPad simulator (`store_ipad_simulator`, default `iPad Pro 13-inch (M5)`, 2064x2752, which App Store Connect requires for an iPad app); they land in `target/ios-store-shots/iphone/` and `target/ios-store-shots/ipad/`, and the run fails when one is not an accepted size or has an alpha channel. `--simulator` replaces the iPhone. The `.args` format is that of `shots`, without `dark` |
| `cargo xtask ios remote archive [--unsigned]` | build the Rust core, then a Release archive on the Mac, `.archive/Underbite.xcarchive` in the lane's directory (bundle identifier `dev.savery.underbite`), signed with `team` from `ios/remote.local.toml` like `device` (it asks for the Mac account's password to unlock the keychain unless `keychain_password_command` is set), or unsigned with `--unsigned`. It never exports or uploads: open the archive in Xcode's Organizer and submit it by hand. It is large: `clean` deletes it with the lane |
| `cargo xtask ios remote ui [--on iphone\|ipad] [--only Class/test]` | build, then the `UnderbiteUITests` target (accessibility audits and what VoiceOver reads) on the iPhone and the iPad simulator one after the other; it switches the simulator's accessibility on and boots it again first (writing the keys to a running simulator is not enough); `--on` picks one device, `--only` one class or test; fails naming the devices that failed |
| `cargo xtask ios remote exec [--fetch path]… -- command…` | sync, then run the command in the lane's directory on the Mac with Xcode selected; `--fetch` copies a path (relative to the repository) back |
| `cargo xtask ios remote device [--iphone name] [--release]` | build, sign and install the app on your iPhone (or the named one), then start it (main checkout only) |
| `cargo xtask ios remote clean --lane name` / `--all-lanes` | delete a lane's directory and simulators on the Mac |
| `cargo xtask ios licenses` | write `ios/Underbite/Underbite/Resources/Acknowledgements.json` from the lock file (the app's open-source notices); a test fails when `Cargo.lock` changed and it was not rerun |

Add `--dry-run` to see the commands without running them. The copy honours `.gitignore`, so build
output, `.env` and the local config files never leave this machine; `.git` is not copied either.
The directory on the Mac carries a `.underbite-remote` marker, and a directory without one is never
overwritten.

The Rust core is built on the build host of `cargo xtask remote` (`remote.local.toml`) when one
answers, else here; `--local` always builds here. The Mac only wraps the staged libraries with
`xcodebuild` and runs Swift, so it needs Remote Login (System Settings, General, Sharing), your
SSH key and Xcode. Xcode is used through `DEVELOPER_DIR` (`xcode = "/Applications/Xcode.app"` in
`ios/remote.local.toml` to change it), so the Mac's `xcode-select` setting does not matter.

### Parallel lanes (several worktrees at once)

Each linked git worktree is a **lane**, so several can build on the same Mac and build host without
wiping each other's trees. The lane is the worktree directory's name, lower-cased and reduced to
`[a-z0-9-]`; `UNDERBITE_LANE=name` overrides it and `UNDERBITE_LANE=` (empty) switches it off. The
main checkout has no lane.

| | main checkout | lane `underbite-lane-a` |
|---|---|---|
| Mac directory | `/Users/Shared/underbite-build` | `/Users/Shared/underbite-build-underbite-lane-a` |
| build host directory | `underbite-build` | `underbite-build-underbite-lane-a` |
| iPhone simulator | `iPhone 17` | `iPhone 17 (underbite-lane-a)` |
| iPad simulator | `iPad Air 11-inch (M4)` | `iPad Air 11-inch (M4) (underbite-lane-a)` |
| lock | `<dir>.lock` | `<dir>.lock` |

- **Directories.** The suffix goes onto a directory from the config files or the default. `--dir`,
  `UNDERBITE_MAC_DIR` and `UNDERBITE_BUILD_DIR` are used as given. `.derived` (DerivedData), `.stage`
  and `.screens` live inside the directory, so lanes do not share them.
- **Config.** The gitignored `remote.local.toml` and `ios/remote.local.toml` are read from the
  worktree, else from the main worktree. `--dry-run` prints which file it used.
- **Lock.** Every run holds `<dir>.lock` until it ends, even on failure. A second run in the same
  directory waits and retries; a stale lock is taken over with a warning.
- **Simulators.** `app` and `shots` make a missing configured simulator from its device type, and
  clone both simulators the first time a lane needs them, as does `store-shots` for its two
  (`xcrun simctl clone`, which requires the
  source to be shut down, so creating a lane's simulators shuts the configured ones down once).
  `test` and `exec` need no simulator.
- **The phone stays in the main checkout.** `device` refuses to run in a lane: it signs with the
  Mac's login keychain and installs on your phone.
- **Clean up.** `cargo xtask remote clean --lane underbite-lane-a` and
  `cargo xtask ios remote clean --lane underbite-lane-a` delete the lane's directory (and lock) on
  the build host and on the Mac, and the lane's simulators (`xcrun simctl delete`). `--all-lanes`
  does every lane. Only directories with the marker are removed, never the main one.

A fresh worktree needs nothing by hand: the framework and the generated Swift are gitignored, and
`ios remote build|test|app|shots` stage the Rust core themselves. The first run in a lane is a cold
build, and the iOS SDK is large: keep an eye on the Mac's free disk, which `doctor` reports.

## Building your own copy

If you cloned the repository and want the app on your own iPhone, you need a Mac with Xcode (no SSH
setup: `cargo xtask ios remote` is only for building from Linux).

1. `cargo xtask ios build` on the Mac stages the Rust core and makes the framework.
2. Create `ios/Underbite/Signing.local.xcconfig` (gitignored) with your team and, since
   `dev.savery.underbite` belongs to the author's account, your own bundle identifier.
3. Open `ios/Underbite/Underbite.xcodeproj` in Xcode, pick your iPhone and run. A free Apple ID
   works for seven days at a time.
