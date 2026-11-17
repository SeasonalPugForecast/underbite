# Gridded wind, pressure and wave layers (candidates)

Checked on 2026-11-17 (UTC, about 10:30 to 11:00) with `curl` against the live services and each service's own metadata. Fixtures: `tests/fixtures/raw/gridded-weather/` (index in its `README.md`). Only the New York Bight (39.0 to 41.5 N, 75.0 to 72.5 W) and the nine points 39.75, 40.25, 40.75 N by 73.75, 73.25, 72.75 W were requested, about 40 requests in all, at least 2 s apart (5 to 8 s on ERDDAP servers). `User-Agent: Underbite-scout/0.1 (data source check)` was sent; no key, account or cookie was used.

The existing map pipeline has three paths: tiles into `MKTileOverlay` (GIBS, NCDS, Copernicus WMTS), grids fetched from an ERDDAP server and rendered on the device (`erddap.md`), and bounding-box WMS. See `erddap.md` and `gibs.md` for the first two.

## Summary

| Candidate | What it offers near New York | Path | Fits the existing pipeline | Verdict (2026-11-17) |
|---|---|---|---|---|
| NOAA nowCOAST map services (`nowcoast.noaa.gov/geoserver`, `mapservices.weather.noaa.gov`) | no wind, pressure or wave layer | WMS / ArcGIS REST | not applicable | **rejected**: the layers do not exist |
| GFS 10 m wind and sea-level pressure on the PacIOOS ERDDAP (`ncep_global_lon180`) | 0.5 degree, 3-hourly, 8-day forecast | griddap, grid rendered on the device | yes, with one new host entry | **usable** (coarse): recommended for wind arrows and pressure |
| WaveWatch III on the PacIOOS ERDDAP (`ww3_global_lon180`) | 0.5 degree, hourly, 7-day forecast; height, period, direction | griddap, grid | yes, with one new host entry and the depth axis | **usable** (coarse): recommended for wave direction and the inspector |
| Copernicus Marine WMTS waves (`GLOBAL_ANALYSISFORECAST_WAV_001_027`) | 0.083 degree, 3-hourly, 10-day forecast; height, direction, period, swell and wind-sea parts | WMTS tiles | tile path yes (`TileSource` needs a Copernicus variant); behind the Copernicus opt-in | **usable**: recommended for the wave-height raster |
| Copernicus Marine WMTS wind (`WIND_GLO_PHY_L4_NRT_012_004`) | 0.125 degree satellite wind analysis, hourly, with arrows drawn into the tile | WMTS tiles | tile path yes | **partly usable**: observation only, newest tile about 35 hours old |
| Open-Meteo multi-point forecast and marine requests as a coarse grid | any point list; model grids from 0.083 degree (marine) and about 0.25 degree (forecast) | JSON per point list | no grid type; needs a point-list decoder | **works, not recommended** for a pan-driven layer; fine for one spot |
| GFS and WaveWatch III GRIB from NOMADS / NCEP | native resolution | GRIB2 | no | **rejected**: GRIB only, too heavy for the phone (not requested) |
| NOAA CoastWatch pfeg ids `NCEP_Global_Best`, `NWW3_Global_Best` | same data as the two PacIOOS datasets | griddap | redirected | use the PacIOOS ids directly |

## nowCOAST (rejected)

- The old `https://nowcoast.noaa.gov/arcgis/rest/services/...` and `layerinfo` paths are blocked since 2023-04-19: the CDN answers HTTP 403 with the text "as of 4/19/2023 nowCOAST no longer accepts requests to \"arcgis\" or \"layerinfo\" as map services have changed" and links to the National Weather Service service change notice `https://www.weather.gov/media/notification/pdf_2023_24/scn23-12_nowcoast_aab.pdf` (not read). The same page says requests can also be blocked "due to rate limiting or incorrect URI contents". The contact is `nowcoast.team@noaa.gov`.
- The current service is one GeoServer WMS: `https://nowcoast.noaa.gov/geoserver/ows?service=wms&request=GetCapabilities` (WMS 1.3.0, 230 KB, 64 layers, `Fees` and `AccessConstraints` both `NONE`). It was read in full. The workspaces are: alerts, bluetopo, boundaries, cbofs, dbofs, geopotential, gomofs, grtofs, leofs, lightning_detection, lmhofs, loofs, lsofs, marine_pathogen, ngofs, nyofs, s100, satellite, sea_surface_temperature, sfbofs, stofs2d, stofs3d, tbofs, tropical_cyclones, tropical_storm_surge, vertical_land_motion, wcofs, weather_radar. No layer is wind, pressure, wave or atmospheric-model guidance. Wind appears only as tropical cyclone wind radii and the observed wind swath.
- `https://mapservices.weather.noaa.gov/` (NWS ArcGIS Server 11.3) lists the folders `raster` (air_quality, climate, hazards, NDFD, obs, outlooks, precip, snow), `eventdriven`, `vector` and `static`. `raster/NDFD` holds only `NDFD_temp`. No wind, pressure or wave service was found in the folders listed; sub-folders of `vector` and `static` other than the names above were not opened (**unverified**).
- Fixture: none (nothing usable).

## GFS wind and pressure on PacIOOS ERDDAP

- Server: `https://pae-paha.pacioos.hawaii.edu/erddap` (ERDDAP 2.29.0, Apache, `access-control-allow-origin: *`). The NOAA CoastWatch id `NCEP_Global_Best` on `coastwatch.pfeg.noaa.gov` is the same data: its `.das` answers 200 there, and data requests answer `302` to `https://pae-paha.pacioos.hawaii.edu/erddap/griddap/ncep_global.json?...`. Use the PacIOOS host directly.
- Dataset ids: `ncep_global` (longitude 0 to 359.5) and `ncep_global_lon180` (longitude -180 to 179.5, checked: same values for the same place). The core clips boxes to -180..180, so use `ncep_global_lon180`.
- Title "NOAA/NCEP Global Forecast System (GFS) Atmospheric Model"; "8-day, 3-hourly forecast for the globe at approximately 50-km or 0.5-deg resolution". Grid 361 x 720 (latitude -90 to 90). Time axis 11,634 steps from 2022-12-01T12:00Z, step `PT3H`, last time 2026-11-24T15:00Z when read (7.2 days ahead of the check). The axis is a "best" series that stitches the newest forecasts; the model run of each step was not examined (**unverified**), and neither was the update cadence (GFS itself runs four times a day).
- Variables used: `ugrd10m`, `vgrd10m` (10 m wind components, `m s-1`), `prmslmsl` (mean sea level pressure, **`Pa`**, `_FillValue` -9.99e+8; the JSON carries NaN-free values over sea and land). Also `tmpsfc`, `tmp2m`, `pratesfc`, `rh2m` and four radiation fluxes. There is no altitude axis: the array order is `[time][latitude][longitude]`.
- Latitude rows come back north to south in JSON for this dataset (first row 41.5), the opposite of `ww3_global`. The grid code sorts the axes, so this is harmless.
- Request (percent-encode the brackets), one time, the Bight box, three variables; 36 rows, 3,074 bytes (fixture):

```text
https://pae-paha.pacioos.hawaii.edu/erddap/griddap/ncep_global_lon180.json
  ?ugrd10m[(2026-11-17T12:00:00Z)][(39.0):(41.5)][(-75.0):(-72.5)],vgrd10m[...same...],prmslmsl[...same...]
```

- Size: about 85 bytes per row for three variables; a 6 x 6 box is 3 KB, and 35 to 43 N by 80 to 68 W (17 x 25 cells, 425 rows) would be about 36 KB. One request per time step; a time slider over 24 hours is eight requests.
- Values at 2026-11-17T12Z in the cell at 40.5 N, 74.0 W (just off the harbour mouth): wind u 3.1 and v -4.5 m s-1 (north-west), pressure 103,209 Pa (1032 hPa); Open-Meteo gave 1031 hPa at the nearest sampled points the same morning.
- Fit: griddap, same JSON table as `erddap.md`; two components give arrows (`GridDraw::Arrows`), `prmslmsl` gives a raster. The pipeline needs a new `GridHost` for `pae-paha.pacioos.hawaii.edu` (base URL and pacing); the `GridSpec` fields (`variables`, `altitude_axis: false`, `native_step_deg: 0.5`) already cover it. Pressure is in Pa: the core converts to hPa for the canonical unit.
- At 0.5 degree (about 55 km) a harbour or a bay falls inside one or two cells. The layer shows weather systems and gradients, not local wind; the spot card keeps using the point sources.

## WaveWatch III on PacIOOS ERDDAP

- Same server. Ids `ww3_global` (0 to 359.5) and `ww3_global_lon180` (the id exists and returned the same rows; fixture). pfeg id `NWW3_Global_Best` redirects to `ww3_global` in the same way. Further regional ids on the server (`ww3_hawaii`, `ww3_samoa`) cover other oceans.
- Title "WaveWatch III (WW3) Global Wave Model"; creator Kwok Fai Cheung (University of Hawaii) with NOAA/NCEP and NWS Honolulu. "7-day model with a 5-day hourly forecast at approximately 50-km or 0.5-deg resolution". Grid 311 latitudes (-77.5 to 77.5) x 720 longitudes with a `depth` axis of length 1. Time axis 85,948 steps from 2017-01-01T21:00Z (`time_coverage_start`), step `PT1H`, last time 2026-11-23T18:00Z when read (6.3 days ahead).
- Variables: `Thgt` (significant wave height, `meters`), `Tper` (peak period, `second`), `Tdir` (peak direction, `degrees`, standard name `sea_surface_wave_from_direction_at_variance_spectral_density_maximum`, so the direction the waves come from), plus the swell parts `shgt`, `sper`, `sdir` and the wind-sea parts `whgt`, `wper`, `wdir` (same units; ids read from the `.dds`, values not fetched). `_FillValue` NaN: land and cells the model does not resolve are `null`.
- Array order `[time][depth][latitude][longitude]`; the depth value is `0` (`altitude_axis: true` in `GridSpec` produces `[(0.0)]`). The table parser in `crates/underbite-providers/src/erddap.rs` looks columns up by name (`time`, `latitude`, `longitude` and the variables), so the extra `depth` column should be ignored; this was read from the code, not run (**unverified**).
- Request, 36 rows, 2,695 bytes (fixture): `.../griddap/ww3_global_lon180.json?Thgt[(2026-11-17T12:00:00Z)][(0.0)][(39.0):(41.5)][(-75.0):(-72.5)],Tdir[...],Tper[...]`.
- Content at 2026-11-17T12Z: 0.28 to 0.77 m, direction 146 to 156 degrees (south-east swell, period 8.1 to 8.3 s) over the shelf; the cells over or next to land (the western edge of the box and the Long Island and Jersey shore cells) are `null`. The Battery, New York Harbor and every river or lake are outside the model.
- Fit: griddap, same as above; arrows from height and direction need a vector conversion (direction to components) that the arrow drawer, which takes eastward and northward components, does not yet have. A scalar raster (height) fits as is.

## Copernicus Marine WMTS waves

- Endpoint and rules as in `copernicus.md`: `https://wmts.marine.copernicus.eu/teroWmts`, key-value WMTS 1.0.0, EPSG:3857 256 px PNG, no account, anonymous access unconfirmed by the service's own text, behind the opt-in.
- Product capabilities: `https://wmts.marine.copernicus.eu/teroWmts/GLOBAL_ANALYSISFORECAST_WAV_001_027?SERVICE=WMTS&REQUEST=GetCapabilities` (HTTP 200, 91 KB). Dataset `cmems_mod_glo_wav_anfc_0.083deg_PT3H-i_202411` (the `_202411` suffix is the reprocessing version; read it from the capabilities as `copernicus.md` describes). A second dataset `cmems_mod_wav_anfc_0.083deg_static_202211--ext--bathy` holds the model's bathymetry.
- Layers (variable ids, all `queryable`): `VHM0` (significant height, m), `VMDR` (mean direction the waves come from), the swell and wind-sea parts `VHM0_SW1`, `VHM0_SW2`, `VHM0_WW`, `VMDR_SW1`, `VMDR_SW2`, `VMDR_WW`, and `VCMX`, `VMXL`, `VPED`, `VSDX`. No wave-period layer (`VTPK`, `VTM10`) is in the list; peak period is therefore not a tile layer.
- `VHM0`: styles `cmap:amp` (default) and `cmap:amp,logScale`; time dimension `2022-11-01T03:00:00Z/2026-11-27T00:00:00Z/PT10800S` (3-hourly, ending 9.5 days ahead), default `2026-11-17T12:00:00.000Z`. Direction layers have their own colour styles (`cmap:cyclic`); no arrow style is offered for waves.
- Tile (fixture, zoom 5, column 9, row 12, 8.7 KB): the wave field over the Bight and the shelf is a smooth pale pink; land and coast are transparent. Legend JSON (fixture, 8.6 KB) gives the 256-step colour table with `valueMin` 0.41 and `valueMax` 6.87 (fixed per layer, `clamp` true): any height below 0.41 m is the lightest colour, so calm days look blank. The app can use the table with its own ramp only for the legend, since the tile is already coloured.
- Updates: `admp_updated` and the time axis are refreshed during the day (the time range had advanced to 2026-11-27 by the check); the model run behind each step was not checked (**unverified**).
- Tiles carry `cache-control: max-age=2592000` as noted in `copernicus.md`. A `TIME` outside the range answers 400 with the valid range.
- Fit: the tile path. `underbite-core::map::TileSource` has GIBS, NCDS, GEBCO and radar variants and no Copernicus one; the Copernicus layer needs a URL builder (the template is in `copernicus.md`), the style in the URL, and a `TIME` value from the capabilities. Values for the inspector are not in the tile: `GetFeatureInfo` (not exercised) or the PacIOOS grid above.

## Copernicus Marine WMTS wind

- Product `WIND_GLO_PHY_L4_NRT_012_004`, dataset `cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H_202207` (capabilities 124 KB, 29 layers). Layer `.../wind` (speed, `m s-1`, colour range 0 to 37.05) plus components (`eastward_wind`, `northward_wind`), stress, curl, divergence, bias and standard deviation layers.
- Styles for `wind`: `cmap:speed,vectorStyle:solid`, `...:solidAndVector`, `...:vector`, each also with `logScale`. The `vector` style draws arrows into the tile (65 KB at zoom 5, fixture), `solid` is a colour field only (11 KB, fixture). The arrows are drawn by the server at its own spacing and cannot be restyled or hidden per zoom.
- Capabilities state `time` as `2024-06-13T04:00:00Z/2026-11-16T23:00:00Z/PT1H` with default `2026-11-16T23:00:00.000Z`, but tiles answered HTTP 500 (`Input contains an empty iterator.`, an `ExceptionReport`) for every hour tried from `2026-11-16T00:00:00Z`, with the default time and with `2026-11-16T12:00:00Z`; `2026-11-15T23:00:00Z` returned 200 (92 KB), `2026-11-10T12Z` to `2026-11-15T12Z` also 200. So at 10:40 UTC on 2026-11-17 the newest tile was about 35 hours old, and the capabilities range overstates it by about a day. A client must not trust the range's end and must fall back on a 500 of this kind.
- Content: a satellite scatterometer and radiometer analysis (observation, not a forecast; the product page was not read: **unverified**), so it has no future steps.
- Verdict: usable as a "last analysis" overlay for the open sea; not a current-wind layer.

## Open-Meteo as a coarse grid

- Both endpoints accept comma-separated `latitude` and `longitude` lists and answer with one JSON object per point inside a top-level array (fixtures, nine points each; both HTTP 200, 3.9 KB and 3.7 KB).
  - `https://api.open-meteo.com/v1/forecast?...&hourly=wind_speed_10m,wind_direction_10m,pressure_msl&wind_speed_unit=ms` returns the requested point snapped to the model grid (39.7401, -73.7425 for 39.75, -73.75: about 0.25 degree or finer, mixed models), `wind_speed_10m` in `m/s` as asked, `wind_direction_10m` in degrees, `pressure_msl` in `hPa`.
  - `https://marine-api.open-meteo.com/v1/marine?...&hourly=wave_height,wave_direction,wave_period` snaps to a 0.083 degree grid (40.2917, -73.7917), `m`, degrees, `s`. A point on or beside land (40.75 N, 73.75 W is on Long Island) is snapped to the nearest sea cell (40.625 N); the answer does not say so except by the returned coordinates.
- Rate limit, read from the response of the nine-point request: `X-RateLimit-Limit: 600`, `X-RateLimit-Window: 60`, `X-RateLimit-Cost: 9.0`, `X-RateLimit-Remaining: 591`, `X-RateLimit-Reset` (epoch seconds). The cost equals the number of locations (for two hourly steps and three variables). `open-meteo-marine.md` recorded no rate-limit headers on 2026-11-08; they are present now on the forecast endpoint. Whether the marine endpoint sends them was not read (**unverified**). The documented free limits are 10,000 per day, 5,000 per hour and 600 per minute, per IP and per application: a 5 x 5 grid costs 25, so one full grid per pan limits the app to 400 pans a day per IP, and 24 grids a minute.
- Fit: no grid type exists for a list of points; the core would need a decoder that turns a point list into a regular grid. The licence (CC BY 4.0, non-commercial free tier, visible credit) is in `open-meteo-marine.md`.
- Verdict: reasonable for one spot or a small, cached fan around the spot (`pressure_msl` trend, wind at the spot), not for a map layer that follows the viewport. If used, build a fixed 0.25 degree lattice, key the cache by lattice cell and hour, fetch only what is missing, and keep the grid out of the daily budget that point forecasts share.

## GRIB sources (rejected)

GFS and WaveWatch III reach the public only as GRIB2 on NOMADS (`nomads.ncep.noaa.gov`, with a grid-subset filter) and as AWS open-data buckets; the data packs wind, pressure and waves in compressed GRIB messages that need a GRIB decoder on the phone, and a filtered NOMADS request was not tried. Rejected for that reason without a request; the PacIOOS ERDDAP above serves the same fields as JSON subsets.

## Licence, credit and terms

- PacIOOS datasets (`.das` `license` text, both datasets): "The data may be used and redistributed for free but is not intended for legal use, since it may contain inaccuracies. Neither the data Contributor, University of Hawaii, PacIOOS, NOAA, State of Hawaii nor the United States Government, nor any of their employees or contractors, makes any warranty ...". `acknowledgement`: "The Pacific Islands Ocean Observing System (PacIOOS) is funded through the National Oceanic and Atmospheric Administration (NOAA) as a Regional Association within the U.S. Integrated Ocean Observing System (IOOS). PacIOOS is coordinated by the University of Hawaii School of Ocean and Earth Science and Technology (SOEST)." Suggested credit: "NOAA/NCEP GFS via PacIOOS" and "WaveWatch III, University of Hawaii / NOAA NCEP via PacIOOS". Publisher contact `info@pacioos.org`. PacIOOS's own data-use page was not read (**unverified**); no rate limit or rate-limit header was seen in nine requests, and none is published in the dataset metadata.
- Copernicus Marine layers: licence, credit text and DOIs in `copernicus.md` (the wave product DOI and credit line were not looked up here: **unverified**; "E.U. Copernicus Marine Service Information" is the general form). The anonymous-access question stays open.
- Open-Meteo: CC BY 4.0, free tier non-commercial, see `open-meteo-marine.md`.
- nowCOAST (not used): NOAA public domain data, `Fees` and `AccessConstraints` `NONE` in the capabilities.
- Not for navigation: charts and current layers carry the note in the app; wind, pressure and wave grids are model output on a 0.083 to 0.5 degree grid and read as forecasts, not as conditions at a spot.

## Recommendation for a later layer task

1. **Wind arrows and pressure**: grid path from `ncep_global_lon180` on the PacIOOS ERDDAP. Add `GridHost` for `pae-paha.pacioos.hawaii.edu`; layers `wind` (`GridDraw::Arrows`, variables `ugrd10m`, `vgrd10m`) and `pressure` (`GridDraw::Raster`, `prmslmsl`, Pa to hPa in the core, optional isobars from the existing contour code). Time slider in 3-hour steps read from the time axis (`time[(today):1:(last)]`); never request a time past the axis. Label the layer "model forecast, about 55 km".
2. **Waves**: grid path from `ww3_global_lon180` for the inspector and direction (height raster plus direction arrows), and, where the Copernicus opt-in is on, WMTS `VHM0` tiles for a finer height raster (0.083 degree). Do not mix the two without saying which one a value comes from.
3. **Observed wind**: Copernicus `wind` tiles only as an optional layer with the age shown, handling the 500 for missing hours.
4. **Open-Meteo** stays a point source; no grid fan-out per pan.
5. Re-check at implementation: the table parser with a `depth` column, whether `ncep_global` has a faster-updating twin, and the Copernicus wave DOI and credit.

## Unverified

- Model run times and update cadence of both PacIOOS datasets, and the values of the WaveWatch III swell and wind-sea variables (2026-11-17).
- The table parser's handling of the `depth` column (read from the code, not run).
- The PacIOOS usage policy page, and whether a limit exists (none seen).
- `GetFeatureInfo` on the Copernicus wave and wind layers; the wave and wind product DOIs; whether the wind product's missing last day is permanent or a delay (checked 2026-11-17 only).
- The Open-Meteo marine rate-limit headers.
- The NWS `vector` and `static` ArcGIS folders beyond their names, the nowCOAST service change notice, and any future nowCOAST wind layer.
- GRIB sources were not requested.
