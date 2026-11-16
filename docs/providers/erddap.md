# NOAA CoastWatch ERDDAP (SST, chlorophyll, Kd490, HF radar currents)

Verified on 2026-11-08 and re-checked on 2026-11-16 (UTC; the Kd490 and limits sections below) against live servers with `curl`, from the ERDDAP server documentation and from each dataset's own metadata (`.das`). Fixtures: `tests/fixtures/raw/erddap/` (index in its `README.md`).

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
| VIIRS Kd490, NOAA-21, near real time / science quality, NIR-SWIR algorithm, US sector US00, 750 m | coastwatch.noaa.gov | `noaacwN21VIIRSkd490NSWUSDaily` | `kd_490` (m^-1); also `l2_flags`, `qa_score` | 0.0075 degree (750 m), lat 0.00375..59.99625, lon -142.38375..-52.39125, with an `altitude` axis of length 1 | daily composite, centred time about 19:00 to 19:45Z (not 12:00Z), days missing from the axis | 2026-11-14T19:22:22Z (2 days before the 2026-11-16 check) | `null` / `NaN`; raw `_FillValue` -32.768. See "Kd490 re-check, 2026-11-16" |
| HF radar surface currents, US East Coast and Gulf of Mexico, 6 km | pfeg | `ucsdHfrE6` | `water_u`, `water_v` (m s-1, eastward and northward); also `DOPx`, `DOPy`, `hdop`, `number_of_sites`, `number_of_radials` | about 0.054 degree latitude by 0.058 degree longitude (about 6 km), lat 21.74..46.49, lon -97.88..-57.23 | hourly, on the hour | 2026-11-08T08:00Z (about 6 h before the check) | `null` in JSON; raw `_FillValue` -327.67 |

Other datasets seen and not chosen:

- `noaacwNPPN20VIIRSkd490Daily` ("merged S-NPP NOAA-20 Kd490, to 2026-11-02" in its title) ends at 2021-09-01 in its time axis; do not use it.
- Gap-filled DINEOF chlorophyll and Kd490 (`noaacwNPPN20S3AkdSCIDINEOF2kmDaily`, 2 km, ends 2026-10-28; `noaacwNPPN20VIIRSDINEOFDaily`, 9 km, to 2026-11-03) fill cloud gaps by interpolation. Candidate for a "no clouds" chlorophyll layer; interpolated values are not observations.
- East Coast 750 m products on `coastwatch.noaa.gov` (`noaacwecnNPPVIIRSSQchlaEastCoastAnnual`, `...Seasonal`) are annual or seasonal composites only. A daily 750 m "sector" family exists (`noaacwNPPVIIRSchlaSectorXXDaily` and the science-quality `noaacwNPPVIIRSSCIchlaSector..Daily`, plus Sentinel-3 OLCI `noaacwS3AOLCIchlaSector..Daily`); **unverified**: which sector letters cover New York, and whether the S-NPP sectors continue.
- `noaacwecnMURdaily` (East Coast MUR) ends in 2021. Use `jplMURSST41`.
- `jplMURSST42` is the 0.25 degree anomaly product, not SST.

Observations that affect clients:

- **S-NPP ends.** The dataset titles on `coastwatch.noaa.gov` say "S-NPP end of mission 2026-11-02" and GIBS stops S-NPP imagery on 2026-11-02. NOAA-20 (and NOAA-21 in GIBS) carry on. The 2026-11-08 search missed the NOAA-20 and NOAA-21 Kd490 datasets of the 750 m US sector; the 2026-11-16 re-check (below) found them. Use `noaacwN21VIIRSkd490NSWUSDaily` for Kd490.
- Chlorophyll coverage on a given day is patchy: for the test box (stride 3) the NOAA-20 series from 2026-10-15 to 2026-11-05 had 63 to 66 valid cells of 88 on 12 days and 0 to 4 on 8 days; the newest day (2026-11-05) was empty. The app must step back to the latest day with data (date picker per §6.7).
- Latitude order differs: `jplMURSST41` and `ucsdHfrE6` list latitude ascending; the 4 km VIIRS datasets list it descending (the first row of the subset is 40.78125 N). A parser must read the coordinates from each row, never assume the order.
- Longitude is -180..180 on the unsuffixed ids. The `_Lon0360` variants exist for the datasets listed and are not needed.
- MUR revises history: "the data for the most recent 7 days is usually revised every day" (dataset `history`), with a nominal 4-day latency for the final version replacing a 1-day near-real-time version. Re-fetch rather than cache the latest week indefinitely.

## Kd490 re-check, 2026-11-16

Searched on 2026-11-16 (about 23:45 UTC) with `tabledap/allDatasets.csv` on both CoastWatch nodes (title matches for `Kd`, attenuation, turbidity, PACE, OLCI, Secchi, reflectance, SPM; fixtures `coastwatch-noaa-alldatasets-kd-search-2026-11-16.csv` and `pfeg-alldatasets-kd-search-2026-11-16.csv`) and on GIBS (see `gibs.md`).

**A current Kd490 product exists: `noaacwN21VIIRSkd490NSWUSDaily`** (coastwatch.noaa.gov). The 2026-11-08 note was wrong to say none was found; the dataset was not in the search then.

| Item | Value |
|---|---|
| Title | "Kd490 (NIR-SWIR), NOAA-21 VIIRS, NRT/Science Quality, 750m Sector US00 (United States), 2025-present, Daily" |
| Variable | `kd_490`, `m^-1` (`.das`), long name "Diffuse Attenuation Coefficient at 490 nm", reference Wang et al. (2009); `l2_flags` and `qa_score` are also served and are not needed |
| Axes | `[time][altitude][latitude][longitude]`, altitude 0.0 |
| Grid | 0.0075 degrees (750 m); latitude 0.00375 to 59.99625; longitude -142.38375 to -52.39125 (covers New York Harbor and the Great Lakes; not Alaska or Hawaii); latitude listed descending in the data (first row 40.79625) |
| Time axis | 2025-01-01T19:10:29Z to 2026-11-14T19:22:22Z; one composite per day centred on the pass (19:xx Z), never 12:00Z; days absent from the axis between 2026-10-20 and 2026-11-14: 10-22, 10-31, 11-01, 11-02, 11-05 |
| Latest at check | 2026-11-14T19:22:22Z (`.das` `time_coverage_end`, and `time[(last)]`) |
| Lag | about 2 days on the check date; the day-of-year in the processing history (`V2026318`) is 2026-11-14 |
| Missing | `null` in JSON, `NaN` in CSV; raw `_FillValue` and `missing_value` -32.768 |
| Values seen | New York box 40.0 to 40.8 N, 73.0 to 74.2 W at stride 6 on 2026-11-11: 313 of 486 cells with a value, 0.08 to 1.48 m^-1; western Lake Erie box 41.4 to 42.0 N, 82.5 to 83.5 W at stride 6 on 2026-11-10: 94 of 322, 0.28 to 2.94 m^-1 |
| Cloud | cloudy days are mostly `null`: New York had 0 cells on 2026-11-13 and 2026-11-14 and 3 on 2026-11-12; Lake Erie 0 on 2026-11-07 and 2026-11-11 and 43 to 94 on 2026-11-08 to 2026-11-10. The app must step back to the newest day with data, as for chlorophyll. |
| Algorithm | NOAA MSL12 v1.8+, "Daily Merge kd490 from NOAA-21 VIIRS"; license text is the usual "may be used and redistributed for free but is not intended for legal use" (see License) |

Request (fixtures `noaacwn21viirskd490nswusdaily-kd490-ny-subset-2026-11-11.json` and `...-erie-subset-2026-11-10.json`):

```
https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?kd_490%5B(2026-11-11T19:27:55Z)%5D%5B(0.0)%5D%5B(40.0):6:(40.8)%5D%5B(-74.2):6:(-73.0)%5D
# time axis, to choose a day
https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?time%5B(2026-10-20):1:(last)%5D
```

Because the time of day varies, a client cannot build the time value from a date. It requests the time axis for the last 10 to 14 days (about 700 bytes), then asks for `kd_490` at one of the listed times (`(last)` also works, for the newest). The time axis of the same days for the other datasets is in the fixtures.

Other Kd490 datasets seen on 2026-11-16 (newest time in the dataset list):

| Dataset id | Newest time | Verdict |
|---|---|---|
| `noaacwN20VIIRSkd490NSWUSDaily` (NOAA-20, same grid and algorithm) | 2026-11-10T19:52:32Z | works, 6 days behind NOAA-21; second choice |
| `noaacwN21VIIRSkd490SectorUSDaily` / `noaacwN20VIIRSkd490SectorUSDaily` (standard algorithm, same sector) | 2026-11-11T19:27:55Z / 2026-11-11T19:34:18Z | work, 3 days behind the NIR-SWIR variant for NOAA-21 |
| `noaacwNPPVIIRSkd490Daily` (S-NPP, 4 km, global; pfeg `nesdisVHNkd490Daily`) | 2026-11-12T12:00:00Z | still grew after the mission end (the 2026-11-08 note saw 2026-11-06); window 2025-11-16 to 2026-11-12, "EXPERIMENTAL" in its summary; on 2026-11-12 only 5 of 726 New York cells had a value (fixture); do not rely on it |
| `noaacwNPPVIIRSkd490Sector??Daily` (S-NPP 750 m sectors) | to 2026-11-12 | S-NPP; the science-quality sectors stopped 2025-08-19 |
| `erdMH1kd4901day_R2022NRT` (pfeg; Aqua MODIS L3SMI, global 4 km, variable `Kd_490`, unit `m-1`, latitude descending, `NaN` missing, daily 12:00Z) | 2026-11-15T12:00:00Z (1 day lag) | time axis current, but coverage is poor: 0 valid cells in a 5 degree box around New York on 2026-11-11 and 2026-11-14 (and 0 in the 0.8 x 1.2 degree box on 11-11 to 11-15), 74 of 2,989 on 2026-10-20 and 895 on 2026-09-20; cause not established. Third choice. |
| `erdMWk4901day` (West US only) | 2026-11-15 | does not cover New York or Lake Erie |
| `noaacwNPPN20VIIRSkd490Daily` | 2021-09-01 | ended; the title is misleading |
| `noaacwNPPN20S3AkdSCIDINEOF2kmDaily`, `...DINEOFDaily` | 2026-11-05 | gap-filled (interpolated) Kd490 from VIIRS and OLCI; 2 km and 9 km global; not observations |

Not found: no PACE OCI or Sentinel-3 OLCI Kd490 grid on either node (OLCI appears only as chlorophyll sectors and the Long Island Sound 300 m colour product `noaacwappsS3ABcolorLISDaily`, which ends 2025-02-28). GIBS has no Kd490 layer (`gibs.md`).

Recommendation recorded for the catalogue: Kd490 can use `noaacwN21VIIRSkd490NSWUSDaily`, with `noaacwN20VIIRSkd490NSWUSDaily` as a fallback. Both depend on a US-sector product labelled "2025-present"; their title says NRT/Science Quality, and the dataset has no stated end date.

### Request limits (2026-11-16)

- The ERDDAP griddap documentation (`https://coastwatch.pfeg.noaa.gov/erddap/griddap/documentation.html`) states no numeric limit on request count or size: "griddap is designed to handle requests of any size"; it warns that "requests for a long time range" (more than about 30 time points) "are prone to time out failures, which often appear as Proxy Errors" and advises several smaller requests; it asks users to "be kind to other ERDDAP users: run just one script or curl command at a time". The page also exposes no quota, `Retry-After` or rate-limit header description; the part after character 100,000 of the page was not read.
- Observed: about 45 requests to three nodes (`coastwatch.noaa.gov` about 25, `coastwatch.pfeg.noaa.gov` about 15, `apps.glerl.noaa.gov` 4) between 23:42 and 00:05 UTC at one request every 6 to 8 seconds (under 10 per minute per host) drew no `429` and no `503`. Response headers carried no rate-limit fields (`Server: Apache`, `Erddap-Server: 2.31.1`).
- The `429` and its headers were not provoked and remain **unverified**. The 2026-11-08 observation stands: about 12 requests in a minute to pfeg answered `429`.
- Size: a 4-D subset costs one JSON row per cell; the 486-row New York subset is 39 KB, the 726-row 4 km one 53 KB. A request of the whole 0.8 x 1.2 degree box at 750 m would be 107 x 161 = 17,227 rows (about 1.2 MB); use stride 4 to 6 for map layers or draw from fewer cells.

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
- VIIRS chlorophyll and Kd490 (the 4 km datasets): `license` points to NASA's data and information policy (`https://science.nasa.gov/earth-science/earth-science-data/data-information-policy/`): open, no restrictions. Credit line: "Ocean colour: NOAA CoastWatch / NOAA STAR VIIRS".
- `ucsdHfrE6`: `license` "The data may be used and redistributed for free but is not intended for legal use, since it may contain inaccuracies. Neither the data Contributor, ERD, NOAA, nor the United States Government ... makes any warranty". Creator: "NOAA IOOS Surface Currents Program" (HFRNet, Scripps/UCSD). Credit line: "Surface currents: NOAA IOOS HF Radar Network".
- Not for navigation: none of these datasets carries the wording; the app adds it to current and SST layers (HF radar surface currents are estimates of the upper 2.4 m and are not for navigation).
- The ERDDAP server citation requested on its information page: "Simons, R.A., and Chris John. 2022. ERDDAP. https://coastwatch.pfeg.noaa.gov/erddap . Monterey, CA: NOAA/NMFS/SWFSC/ERD."

- `noaacwN21VIIRSkd490NSWUSDaily`: `license` is the "may be used and redistributed for free but is not intended for legal use" text with the no-warranty clause; creator "NOAA CoastWatch" (`https://coastwatch.noaa.gov`), institution "NOAA NESDIS STAR". Credit line: "Water clarity: NOAA CoastWatch / NOAA STAR VIIRS".
- `erdMH1kd4901day_R2022NRT`: `license` points to NASA's data and information policy; data from NASA OBPG (`oceancolor.gsfc.nasa.gov`).

## What was not verified

- Rate limit thresholds and the `429` response headers (no `429` occurred on 2026-11-16 while keeping under 10 requests a minute per host).
- The numeric maximum size of a griddap request.
- `.transparentPng` drawing parameters and binary formats.
- Which S-NPP VIIRS 750 m sector covers New York (the NOAA-20 and NOAA-21 Kd490 datasets above need no sector letter).
- Holdings of `upwell.pfeg.noaa.gov`.
