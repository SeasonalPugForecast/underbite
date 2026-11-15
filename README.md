# Underbite

A free, ad-free, account-free fishing companion for iPhone. It gathers what an angler checks before
and during a trip, for salt and fresh water alike: tides and tidal currents, river flow and gage
height, water temperature and clarity, waves, wind and weather, sun and moon, depth and structure,
access, stocking and seasons. Every data source, card, unit, limit and map layer is configurable.

Data comes only from free public sources (NOAA, USGS, NASA, Open-Meteo and others), and every value
shows where it came from, how far away it was measured and how old it is.

**Not for navigation.** Charts, depths and currents are for planning only. Fishing regulations are
linked from their official sources, never stated by the app.

Status: early development.

## Layout

A Rust core (data model, astronomy, view model) and sans-IO data providers are shared by the iOS app
(through UniFFI) and a host command-line tool. Providers build requests and parse bytes; network I/O
lives only in the command-line runtime and in the app. The weather side builds on the crates of the
whether project.

- `crates/underbite-core`, `crates/underbite-providers`, `crates/underbite-config`: the libraries
- `apps/underbite`: the command-line tool
- `xtask`: build, check and remote-build automation
- `docs/providers`: notes on each data source
- `tests/fixtures`: recorded responses used by the tests (see `tests/fixtures/README.md`)

## Build and check

```text
cargo xtask ci              # fmt, clippy -D warnings, tests, cargo-deny
cargo xtask remote doctor   # check the configured build hosts
cargo xtask remote ci       # the same gate on a build host
cargo xtask remote run --fetch <path> -- <command>
```

Build hosts are listed in `remote.local.toml`, which is not tracked; copy
`remote.local.toml.example` and edit it. Tests never touch the network; live checks are `#[ignore]`
tests named `live_*` and run by hand.

## Data sources

- **Tides, water levels, currents, water temperature and air observations:** NOAA/NOS CO-OPS.
  "Tide, water-level and current data: NOAA/NOS CO-OPS."
- **Buoy observations (waves, wind, water temperature):** "Buoy observations: NOAA National Data Buoy
  Center."
- **River flow, gage height and water temperature:** U.S. Geological Survey. Values marked
  provisional are subject to revision and must not be relied on for safety decisions.
- **River forecasts and stage:** NOAA National Water Prediction Service (NWPS), with observations
  courtesy of the U.S. Geological Survey and NOAA National Ocean Service.
- **Marine forecasts and alerts:** NOAA National Weather Service.
- **Marine and weather model data:** "Weather data by Open-Meteo.com" (<https://open-meteo.com/>),
  CC BY 4.0, with the model providers it names: Copernicus Marine Service, ECMWF, NOAA NCEP, DWD and
  the Copernicus Climate Change Service.
- **Sea surface temperature and ocean colour:** "SST: NASA JPL MUR (MEaSUREs), via NOAA CoastWatch
  ERDDAP"; "Ocean colour: NOAA CoastWatch / NOAA STAR VIIRS".
- **Satellite imagery:** "We acknowledge the use of imagery provided by services from NASA's Global
  Imagery Browse Services (GIBS), part of NASA's Earth Science Data and Information System (ESDIS)."
- **Nautical charts:** NOAA Office of Coast Survey, NOAA Chart Display Service (ENC). Not for
  navigation.
- **Bathymetry:** NOAA NCEI (DEM global mosaic, Coastal Relief Model, CUDEM) and the GEBCO Grid,
  GEBCO Compilation Group. Not for navigation or any purpose relating to safety at sea.
- **Access, stocking, seasons and other regional sources:** surveyed; each is credited in the app
  once it is used.

## License

MIT.
