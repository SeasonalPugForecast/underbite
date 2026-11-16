# Screenshots

`cargo xtask ios remote shots` builds the app for the simulator, runs it in an iPhone and an iPad
simulator and takes one screenshot per `*.args` file in this directory. Which screens exist is
decided here, not in the tool: add a file to add a screenshot.

## File format

One file per screenshot, `<name>.args`:

```text
-seed -tab tides
dark
```

- **Line 1** is the launch arguments, split on whitespace (blank for none). They are passed to
  `xcrun simctl launch` as they stand; the app reads them only in debug builds.
- **Line 2** is optional: `dark` also takes a dark-appearance screenshot of the same launch.
- Nothing else is allowed. A name may hold letters, digits, `-`, `_` and `.`; files are taken in
  sorted order, so a numeric prefix (`10-now.args`) fixes the order.

## Output

`target/ios-screens/iphone/<name>.png` and `target/ios-screens/ipad/<name>.png`, with
`<name>-dark.png` beside those that asked for a dark one. The directory is replaced on every run.
A screenshot under 120 KB is a screen that had not been drawn yet: it is taken again (three tries)
and a blank that stays blank is reported.

The simulators are `simulator` and `ipad_simulator` in `ios/remote.local.toml` (defaults `iPhone 17`
and `iPad Air 11-inch (M4)`), cloned per lane as `<name> (<lane>)`.
