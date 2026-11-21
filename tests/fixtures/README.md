# Test fixtures

Recorded responses from the data sources, used by tests that must never touch the network.

## Indexes

Each source has its own directory under `raw/` and an index that lists every file with the exact
request URL and the recording date:

- [coops](raw/coops/README.md)
- [ndbc](raw/ndbc/README.md)
- [usgs](raw/usgs/README.md)
- [usno](raw/usno/README.md)
- [nwps](raw/nwps/README.md)
- [open-meteo-marine](raw/open-meteo-marine/README.md)
- [nws-marine](raw/nws-marine/README.md)
- [erddap](raw/erddap/README.md)
- [gibs](raw/gibs/README.md)
- [glerl](raw/glerl/README.md)
- [ncds](raw/ncds/README.md)
- [bathymetry](raw/bathymetry/README.md)
- [copernicus](raw/copernicus/README.md)
- [tier2](raw/tier2/README.md)
- [hydro](raw/hydro/README.md)
- [weather](raw/weather/README.md)
- [regulations](raw/regulations/README.md)
- [state-maps](raw/state-maps/README.md)
- [fishbase](raw/fishbase/README.md)
- [dfo-iwls](raw/dfo-iwls/README.md)
- [eccc](raw/eccc/README.md)
- [cwms](raw/cwms/README.md)
- [rise](raw/rise/README.md)
- [cyan](raw/cyan/README.md)
- [hab](raw/hab/README.md)
- [overpass](raw/overpass/README.md)
- [padus](raw/padus/README.md)
- [gbif](raw/gbif/README.md)
- [inaturalist](raw/inaturalist/README.md)
- [nas](raw/nas/README.md)
- [stocking-ny](raw/stocking-ny/README.md)
- [gridded-weather](raw/gridded-weather/README.md)
- [tidal-limit](raw/tidal-limit/README.md) (recorded 2026-11-20; NHD, NWI, VDatum and CO-OPS requests, with the exact URL of each file in the index)
- [swimming](raw/swimming/README.md) (recorded 2026-11-20; NWS surf zone and beach forecasts, NWPS rip current stations, beach water quality from Chicago, New York City, New York State and the Water Quality Portal, OSM lifeguard tags, UV index, with the exact URL of each file in the index)

## Conventions

- Files are the raw bytes of the response, in the source's own format (JSON, CSV, XML, text, zip and
  so on). Where a file was reduced, its index line says how.
- Hand-made files are named `*.synthetic.*` and are never presented as recordings.
- Every file is listed in its directory's README with the request URL and the recording date. A
  fixture without an index line does not belong here.
- No API keys, tokens or personal data in any file, URL or name. Requests are recorded without keys.
- The biodiversity record fixtures (GBIF, iNaturalist) are synthetic: invented records in the sources' shapes.
- Test places are New York Harbor (The Battery, CO-OPS 8518750; NDBC 44025), the lower Hudson and a
  Croton River USGS site.
- Live checks against the real services are `#[ignore]` tests named `live_*`, run by hand only.
- Tests load fixtures with `crates/underbite-providers/src/testutil.rs`, which resolves
  `tests/fixtures/raw/<source>/<name>` from the workspace root.
