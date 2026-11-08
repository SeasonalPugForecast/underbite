# NOAA CoastWatch ERDDAP (SST, chlorophyll, Kd490, HF radar currents)

Verified on 2026-11-08 (UTC) against live servers with `curl`, from the ERDDAP server documentation and from each dataset's own metadata (`.das`). Fixtures: `tests/fixtures/raw/erddap/` (index in its `README.md`).

## Servers

| Node | Base URL | Notes |
|---|---|---|
| NOAA CoastWatch West Coast Node (NMFS SWFSC ERD) | `https://coastwatch.pfeg.noaa.gov/erddap` | ERDDAP 2.31.1. Hosts MUR SST and HF radar. |
| NOAA CoastWatch (NESDIS STAR) | `https://coastwatch.noaa.gov/erddap` | ERDDAP 2.31.1, dataset ids start with `noaacw`. Hosts VIIRS ocean colour. |
| `https://upwell.pfeg.noaa.gov/erddap` | exists (answers 302 to its own `/erddap/info/index.json?page=1&itemsPerPage=1`) | **unverified**: holdings not examined; nothing needed was missing from the two nodes above. |

Both nodes serve the same ERDDAP interface. All requests need only a descriptive `User-Agent`; no key or account.

Redirect to know about: on the pfeg node, data requests (`.json`, `.csv`, ...) for the datasets whose ids start with `nesdisVHN` answer `302` to the same query on `coastwatch.noaa.gov` under the matching `noaacw...` id (`nesdisVHNchlaDaily` to `noaacwNPPVIIRSchlaDaily`, `nesdisVHNkd490Daily` to `noaacwNPPVIIRSkd490Daily`). The `.das` and `.dds` documents of these ids were served directly by pfeg at 14:18 UTC; the data requests answered 302 from 14:21 UTC on. Whether the `.das` requests redirect now was not re-tested. A client should use the `coastwatch.noaa.gov` ids directly and follow redirects as a fallback.

## Datasets

| Layer | Node | Dataset id | Variable(s) | Grid | Time step | Latest time at check | Fill / missing |
|---|---|---|---|---|---|---|---|
| MUR SST, v4.1, global | pfeg | `jplMURSST41` | `analysed_sst` (degree_C); also `analysis_error`, `mask`, `sea_ice_fraction` | 0.01 degree (about 1.1 km), lat -89.99..89.99, lon -179.99..180 | daily, 09:00Z | 2026-11-07T09:00Z (dataset `time_coverage_end`), about 29 h before the check | land and ice-masked cells are `null` in JSON, `NaN` in CSV; raw `_FillValue` -7.768 |
| VIIRS chlorophyll-a, NOAA-20, near real time, global 4 km | coastwatch.noaa.gov | `noaacwN20VIIRSchlaDaily` | `chlor_a` (mg m^-3, OC3) | 0.0375 degree (4 km), 4788 x 9602, with an `altitude` axis of length 1 | daily, 12:00Z, irregular (days without a swath are missing from the time axis) | 2026-11-05T12:00Z (3 days before the check) | clouds and no-data are `null` / `NaN`; raw `_FillValue` NaN; `valid_min` 0.001, `valid_max` 1000 |
| VIIRS chlorophyll-a, S-NPP, near real time, global 4 km | coastwatch.noaa.gov (also under pfeg id `nesdisVHNchlaDaily`, which redirects) | `noaacwNPPVIIRSchlaDaily` | `chlor_a` | same grid | daily 12:00Z | 2026-11-06T12:00Z | as above |
| VIIRS Kd490, S-NPP, near real time, global 4 km | coastwatch.noaa.gov (pfeg id `nesdisVHNkd490Daily` redirects) | `noaacwNPPVIIRSkd490Daily` | `kd_490` (m^-1) | same grid | daily 12:00Z | 2026-11-06T12:00Z | `null` / `NaN`; raw `_FillValue` -6.5534 |
| HF radar surface currents, US East Coast and Gulf of Mexico, 6 km | pfeg | `ucsdHfrE6` | `water_u`, `water_v` (m s-1, eastward and northward); also `DOPx`, `DOPy`, `hdop`, `number_of_sites`, `number_of_radials` | about 0.054 degree latitude by 0.058 degree longitude (about 6 km), lat 21.74..46.49, lon -97.88..-57.23 | hourly, on the hour | 2026-11-08T08:00Z (about 6 h before the check) | `null` in JSON; raw `_FillValue` -327.67 |

Other datasets seen and not chosen:

- `noaacwNPPN20VIIRSkd490Daily` ("merged S-NPP NOAA-20 Kd490, to 2026-11-02" in its title) ends at 2021-09-01 in its time axis; do not use it.
- Gap-filled DINEOF chlorophyll and Kd490 (`noaacwNPPN20S3AkdSCIDINEOF2kmDaily`, 2 km, ends 2026-10-28; `noaacwNPPN20VIIRSDINEOFDaily`, 9 km, to 2026-11-03) fill cloud gaps by interpolation. Candidate for a "no clouds" chlorophyll layer; interpolated values are not observations.
- East Coast 750 m products on `coastwatch.noaa.gov` (`noaacwecnNPPVIIRSSQchlaEastCoastAnnual`, `...Seasonal`) are annual or seasonal composites only. A daily 750 m "sector" family exists (`noaacwNPPVIIRSchlaSectorXXDaily` and the science-quality `noaacwNPPVIIRSSCIchlaSector..Daily`, plus Sentinel-3 OLCI `noaacwS3AOLCIchlaSector..Daily`); **unverified**: which sector letters cover New York, and whether the S-NPP sectors continue.
- `noaacwecnMURdaily` (East Coast MUR) ends in 2021. Use `jplMURSST41`.
- `jplMURSST42` is the 0.25 degree anomaly product, not SST.

Observations that affect clients:

- **S-NPP ends.** The dataset titles on `coastwatch.noaa.gov` say "S-NPP end of mission 2026-11-02" and GIBS stops S-NPP imagery on 2026-11-02. NOAA-20 (and NOAA-21 in GIBS) carry on. No NOAA-20 or NOAA-21 near-real-time Kd490 dataset was found; the S-NPP Kd490 series is likely to stop growing. Treat the Kd490 layer as at risk and re-check before relying on it.
- Chlorophyll coverage on a given day is patchy: for the test box (stride 3) the NOAA-20 series from 2026-10-15 to 2026-11-05 had 63 to 66 valid cells of 88 on 12 days and 0 to 4 on 8 days; the newest day (2026-11-05) was empty. The app must step back to the latest day with data (date picker per §6.7).
- Latitude order differs: `jplMURSST41` and `ucsdHfrE6` list latitude ascending; the 4 km VIIRS datasets list it descending (the first row of the subset is 40.78125 N). A parser must read the coordinates from each row, never assume the order.
- Longitude is -180..180 on the unsuffixed ids. The `_Lon0360` variants exist for the datasets listed and are not needed.
- MUR revises history: "the data for the most recent 7 days is usually revised every day" (dataset `history`), with a nominal 4-day latency for the final version replacing a 1-day near-real-time version. Re-fetch rather than cache the latest week indefinitely.

## griddap request syntax (verified)

```
{base}/griddap/{datasetId}.{format}?{var}[{time}][{altitude}][{lat}][{lon}]{,{var2}[...]}
```

- Formats used: `.json` (table: `columnNames`, `columnTypes`, `columnUnits`, `rows`; missing as `null`), `.csv` (a header row and a units row, then data; missing as `NaN`), `.das`, `.dds`, `.transparentPng`. Binary options (`.nc`, `.nc4`, `.mat`) exist; **unverified** here.
- Each axis constraint is `[start:stride:stop]`; start and stop are either an index (`[0:1:99]`) or a value in parentheses (`[(40.0):(40.8)]`, `[(2026-11-07T09:00:00Z)]`). `stride` is optional and counts grid steps. `(last)` and `last-N` refer to the newest time, for example `[(last)]` or `[last-3:1:last]`.
- Axis order of the datasets above: MUR and HF radar `[time][latitude][longitude]`; the 4 km VIIRS datasets `[time][altitude][latitude][longitude]` with altitude `(0.0)`.
- Values in parentheses are matched to the nearest axis value inside the axis range. A time one step past the end of the axis returned `503 Service Unavailable` on 2026-11-08 (a request for `2026-11-08T09:00:00Z` on MUR); a time far past the end returns `404 Not Found` with the text `Your query produced no matching results ... "Start" is greater than the axis maximum=...`. Use `(last)`.
- Time is UTC, ISO 8601 with `Z`; JSON time values are strings, the `.das` `actual_range` is in seconds since 1970-01-01T00:00:00Z.
- A request returns one row per grid cell, so the size is the cell count times the row size: the whole test box on MUR at native resolution is 81 x 121 = 9,801 cells (roughly 0.6 MB as JSON); `(40.0):5:(40.8)` and `(-74.2):5:(-73.0)` give 17 x 25 cells (24 KB, the fixture).

### Example requests

Test box 40.0 to 40.8 N, 73.0 to 74.2 W (all recorded; see fixtures):

```
# MUR SST, newest day, every 5th cell
https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.json?analysed_sst%5B(last)%5D%5B(40.0):5:(40.8)%5D%5B(-74.2):5:(-73.0)%5D

# HF radar currents, newest hour, both components (6 km, full resolution)
https://coastwatch.pfeg.noaa.gov/erddap/griddap/ucsdHfrE6.json?water_u%5B(last)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D,water_v%5B(last)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D

# VIIRS NOAA-20 chlorophyll, a given day, full resolution
https://coastwatch.noaa.gov/erddap/griddap/noaacwN20VIIRSchlaDaily.json?chlor_a%5B(2026-11-04T12:00:00Z)%5D%5B(0.0)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D

# VIIRS Kd490, a given day
https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPVIIRSkd490Daily.json?kd_490%5B(2026-11-04T12:00:00Z)%5D%5B(0.0)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D

# Time axis only (find the days that exist)
https://coastwatch.noaa.gov/erddap/griddap/noaacwN20VIIRSchlaDaily.json?time%5B(2026-10-15):1:(last)%5D
```

URL template: `{base}/griddap/{id}.json?{var}%5B({t})%5D%5B({lat0}):{step}:({lat1})%5D%5B({lon0}):{step}:({lon1})%5D` (brackets percent-encoded as `%5B` and `%5D`; ERDDAP also accepts them raw in a browser but strict URL libraries want them encoded).

### Field table

| Dataset | JSON column | Type | Unit | Meaning | Missing |
|---|---|---|---|---|---|
| all | `time` | string | UTC ISO 8601 | nominal time of the field | n/a |
| all | `latitude`, `longitude` | number | degrees north, degrees east | cell centre | n/a |
| 4 km VIIRS | `altitude` | number | m | always 0 | n/a |
| `jplMURSST41` | `analysed_sst` | number | degree_C | foundation SST analysis | `null` over land |
| `ucsdHfrE6` | `water_u` | number | m s-1 | eastward surface velocity (upper 2.4 m) | `null` outside radar coverage (190 of the 315 test-box cells had a value) |
| `ucsdHfrE6` | `water_v` | number | m s-1 | northward surface velocity | `null` |
| `noaacwN20VIIRSchlaDaily` | `chlor_a` | number | mg m^-3 | chlorophyll-a, OC3 | `null` (cloud, glint, land) |
| `noaacwNPPVIIRSkd490Daily` | `kd_490` | number | m^-1 | diffuse attenuation at 490 nm; clarity grows as the value falls | `null` |

Units come from the response's own `columnUnits`. The core can read them rather than hard-coding.

## Map tiles from ERDDAP (WMS, `.transparentPng`)

- WMS: `{base}/wms/{datasetId}/request?service=WMS&version=1.3.0&request=GetMap&layers={datasetId}:{variable}&crs=EPSG:4326&bbox={lat0},{lon0},{lat1},{lon1}&width={w}&height={h}&format=image/png&transparent=true&time={time}&styles=`. ERDDAP's WMS documentation lists three versions (1.1.0, 1.1.1, 1.3.0), `image/png` as the only format and `CRS:84` and `EPSG:4326` as the only coordinate systems; one time value per request; `time=current` means the newest; the only style is the default (`styles=` empty). Verified: a 384 x 256 `EPSG:4326` GetMap for the test box returned 200 with a PNG (fixture); the same request with `crs=EPSG:3857` returned `503` on two tries, so Web Mercator is **not** supported. Capabilities advertise `MaxWidth` and `MaxHeight` of 4096.
- Colour range is fixed by the dataset's `colorBarMinimum` and `colorBarMaximum` (MUR: 0 to 32 degree_C). The test image comes out as an almost uniform green; a useful SST ramp needs the grid values and on-device rendering.
- `.transparentPng`: `{base}/griddap/jplMURSST41.transparentPng?analysed_sst%5B(2026-11-07T09:00:00Z)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D` returns an RGBA PNG with one pixel per grid cell (121 x 81 for the test box). It is the dataset's own palette, again with the fixed colour range. Drawing options (`.draw`, `.colorBar`, `.size`) are described in the ERDDAP docs; **unverified** here.
- Consequence for `MKTileOverlay`: ERDDAP cannot serve XYZ or EPSG:3857 tiles. For colour imagery in Web Mercator use NASA GIBS (`docs/providers/gibs.md`). For ERDDAP-sourced data, fetch the griddap subset and draw it on device (also needed for the SST and chlorophyll edge detector).

## Authentication, limits, availability

- No authentication.
- **Rate limiting observed.** About 12 requests in roughly one minute (several metadata queries, a `.das` request and a time-axis query) to `coastwatch.pfeg.noaa.gov` produced `429 Too Many Requests` (an Apache error page) at 14:19 UTC. The headers sent with the 429 were not captured; no `Retry-After` or rate-limit header was seen on the 200 responses (`erddap-server: 2.31.1`, standard security headers only). The limit values are **unverified**.
- **Intermittent 503.** From 14:27 to 14:32 UTC, about one in four requests to the pfeg node answered `503 Service Unavailable` with the body `There was a (temporary?) problem.  Wait a minute, then try again.`, including repeats of requests that had succeeded. Plan for retry with back-off, and a cache.
- ERDDAP's own documentation does not state a request quota. `https://coastwatch.pfeg.noaa.gov/erddap/information.html` says an ERDDAP "runs as an entirely public server with no login system and no restrictions" unless the administrator configures them. Keep requests to a few small subsets per layer refresh, cache by (dataset, time, box), and send a `User-Agent` that names the app.
- Maximum range per request: no fixed limit is stated in the documentation fetched; a request that exceeds server memory is refused with an error. **Unverified**: the numeric ceiling.
- Latency and cadence: see the dataset table. HF radar is hourly with a few hours of lag; MUR is daily with one day of lag; VIIRS is daily with 2 to 3 days of lag and gaps.

## License and attribution

- Each dataset states its terms in its `license` attribute (`.das`). ERDDAP's `legal.html` adds: "The data from some datasets may be subject to usage limitations or restrictions", and that the service and data are provided "as is" with no warranty.
- `jplMURSST41`: "These data are available free of charge under the JPL PO.DAAC data policy. The data may be used and redistributed for free but is not intended for legal use, since it may contain inaccuracies." Requested acknowledgement (`acknowledgement` attribute): "These data were provided by JPL under support by NASA MEaSUREs program." Credit line suggested for the app: "SST: NASA JPL MUR (MEaSUREs), via NOAA CoastWatch ERDDAP".
- VIIRS chlorophyll and Kd490: `license` points to NASA's data and information policy (`https://science.nasa.gov/earth-science/earth-science-data/data-information-policy/`): open, no restrictions. Credit line: "Ocean colour: NOAA CoastWatch / NOAA STAR VIIRS".
- `ucsdHfrE6`: `license` "The data may be used and redistributed for free but is not intended for legal use, since it may contain inaccuracies. Neither the data Contributor, ERD, NOAA, nor the United States Government ... makes any warranty". Creator: "NOAA IOOS Surface Currents Program" (HFRNet, Scripps/UCSD). Credit line: "Surface currents: NOAA IOOS HF Radar Network".
- Not for navigation: none of these datasets carries the wording; the app adds it to current and SST layers (HF radar surface currents are estimates of the upper 2.4 m and are not for navigation).
- The ERDDAP server citation requested on its information page: "Simons, R.A., and Chris John. 2022. ERDDAP. https://coastwatch.pfeg.noaa.gov/erddap . Monterey, CA: NOAA/NMFS/SWFSC/ERD."

## What was not verified

- Rate limit thresholds and the `429` response headers.
- The numeric maximum size of a griddap request.
- `.transparentPng` drawing parameters and binary formats.
- Which VIIRS 750 m sector covers New York and whether it is fed after S-NPP's end of mission.
- Holdings of `upwell.pfeg.noaa.gov`.
