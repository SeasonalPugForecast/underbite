# NOAA CO-OPS: Data API and Metadata API

Verified on 2026-11-08 against the live services and the official documentation:

- Data API documentation: <https://api.tidesandcurrents.noaa.gov/api/prod/> and the response field help
  <https://api.tidesandcurrents.noaa.gov/api/prod/responseHelp.html>
- Metadata API documentation (v1.0): <https://api.tidesandcurrents.noaa.gov/mdapi/prod/>
- Disclaimers: <https://tidesandcurrents.noaa.gov/disclaimers.html>

Recorded responses are in `tests/fixtures/raw/coops/` (index: `tests/fixtures/raw/coops/README.md`). Test
places: The Battery (8518750), Tarrytown (8518919, subordinate tide station), Hudson River Entrance (NYH1927,
harmonic current station), Statue of Liberty east (ACT3646, subordinate current station).

## 1. Summary of what to implement

| Need | Request | Notes |
|---|---|---|
| High and low tide times | `predictions`, `interval=hilo` | works for harmonic and subordinate stations |
| Tide curve | `predictions`, `interval=6` (or 1, 5, 10, 15, 30, 60, `h`) | harmonic (reference) stations only |
| Observed water level | `water_level` | needs `datum`; The Battery is preliminary 6-minute data |
| Water temperature, air temperature, pressure, wind | `water_temperature`, `air_temperature`, `air_pressure`, `wind` | only where the station has the sensor; The Battery has no wind |
| Current max/slack | `currents_predictions`, `interval=max_slack` | harmonic and subordinate stations; pass `bin` |
| Current curve | `currents_predictions`, `interval=6` etc. | harmonic stations only |
| Observed current | `currents` | PORTS meters only; availability varies (see 4.5) |
| Station discovery | mdapi `stations.json?type=…` | very large lists; filter client-side |
| Datums, harmonic constituents, offsets | mdapi `stations/{id}/datums.json`, `harcon.json`, `tidepredoffsets.json`, `?expand=currentpredictionoffsets` | |

## 2. Data API

### 2.1 Base URL, version and parameters

Base: `https://api.tidesandcurrents.noaa.gov/api/prod/datagetter` (the `prod` path segment is the only
version marker; there is no version header). GET only. HTTP/2, gzip accepted (`Accept-Encoding: gzip` returns
`content-encoding: gzip`).

Parameters observed to be **required** by the live service (each returned HTTP 400 with an `error` message when
omitted):

| Parameter | Required | Values used | Notes |
|---|---|---|---|
| `product` | yes | `predictions`, `water_level`, `water_temperature`, `air_temperature`, `air_pressure`, `wind`, `currents`, `currents_predictions`, `hourly_height` | missing: plain text `Wrong Product : Product cannot be null or empty` |
| `station` | yes | 7-digit id (`8518750`) or current-station id (`NYH1927`, `n06010`) | case as listed by mdapi; unknown: HTTP 400 `The station is not a valid station or there is system error.` |
| `begin_date` + `end_date`, or `date`, or `range` combinations | yes | `yyyyMMdd` | `end_date` is inclusive: the whole last day is returned (the 6-minute series ends at 23:54). `date=latest`, `today`, `recent` are accepted for water level, meteorological data and predictions |
| `datum` | for `predictions`, `water_level`, `hourly_height` | `MLLW`, `MSL`, `STND`, `MTL`, … | omitted: HTTP 400 `Wrong Datum: Datum cannot be null or empty`. Subordinate stations must use `MLLW` per the documentation. A repeated `datum` parameter: the first one wins |
| `units` | **yes** | `metric`, `english` | omitted: HTTP 400 `Wrong Unit:Unit cannot be null or empty` (the documentation does not say it is mandatory) |
| `time_zone` | **yes** | `gmt`, `lst`, `lst_ldt` | omitted: HTTP 400 `Wrong Time zone: Time zone cannot be null or empty` |
| `format` | **yes** | `json` (also `xml`, `csv`) | omitted: plain text `Wrong Format: Format cannot be null or empty` |
| `interval` | for predictions | `hilo`, `1`, `5`, `6`, `10`, `15`, `30`, `60`, `h`; currents: `max_slack`, `1`, `6`, `10`, `30`, `60`, `h` | |
| `bin` | for current stations | integer | see section 4 |
| `vel_type` | optional | `speed_dir` or `default` | harmonic current predictions only |
| `application` | optional | `underbite` | identifies the caller in the provider's logs |

Authentication: none. `User-Agent`: not required (a request with an empty `User-Agent` was served), and no
`X-` rate limit headers are returned. The API documentation says only that CO-OPS throttles heavy users and
recommends pausing between calls and downloading only what is needed; no numeric limit is published
(**unverified**: the actual throttle threshold, which was not reached with about 110 requests spaced about one second apart).
Responses to `datagetter` carry `cache-control: no-cache, no-store, must-revalidate`.

Always send `time_zone=gmt`, `units=metric`, `format=json` and `application=underbite`.

### 2.2 Example requests

Seven-day high/low predictions at The Battery (fixture `coops-8518750-hilo-2026-11-08.json`):

```text
https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=predictions&application=underbite&begin_date=20261108&end_date=20261114&datum=MLLW&station=8518750&time_zone=gmt&units=metric&interval=hilo&format=json
```

The same week at 6-minute spacing (fixture `coops-8518750-predictions-6min-2026-11-08.json`): replace
`interval=hilo` with `interval=6`.

Current max/slack at Hudson River Entrance for bin 13 (fixture `coops-NYH1927-bin13-max-slack-2026-11-08.json`):

```text
https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=currents_predictions&application=underbite&begin_date=20261108&end_date=20261114&station=NYH1927&bin=13&interval=max_slack&time_zone=gmt&units=metric&format=json
```

Observed water level for one day (fixture `coops-8518750-water-level-2026-11-07.json`):

```text
https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=water_level&application=underbite&datum=MLLW&begin_date=20261107&end_date=20261107&station=8518750&time_zone=gmt&units=metric&format=json
```

The latest single water-level point: `…&product=water_level&datum=MLLW&date=latest&…`.

### 2.3 Time zone and time format

With `time_zone=gmt` every `t` and `Time` value is UTC in the form `YYYY-MM-DD HH:MM` (no seconds, no zone
designator, no `T`). `lst` is local standard time (fixed offset, no daylight saving), `lst_ldt` is local civil
time. Example: with `gmt` the Battery low of `2026-11-08 05:59` appears as `2026-11-08 00:59` with `lst_ldt`
(EST, UTC-5, after the 2026-11-01 change). `begin_date` and `end_date` are interpreted in the same zone as the
output. The daily products `daily_mean` and `monthly_mean` are restricted to local standard time and are not
used.

### 2.4 Response shapes

All values are JSON strings, even numbers.

**Predictions** (`product=predictions`). The body is exactly `{ "predictions" : [ … ] }` (no `metadata`):

| Field | Meaning | Unit (`units=metric` / `english`) | Missing marker |
|---|---|---|---|
| `t` | time | UTC with `gmt` | none seen |
| `v` | predicted height above the requested datum | metres / feet, 3 decimals | none seen |
| `type` | only with `interval=hilo`: `H` high, `L` low | | |

Sample (fixture): `{"t":"2026-11-08 12:07", "v":"1.711", "type":"H"}`. The seven days contain 27 events and 1680
six-minute points. `hilo` returns alternating `H`/`L`; `HH`/`LL` are not used by predictions.

**Observations** (`water_level`, `water_temperature`, `air_temperature`, `air_pressure`, `wind`,
`hourly_height`) return `{"metadata":{"id","name","lat","lon"}, "data":[…]}` (metadata values are strings):

| Product | Fields in `data[]` | Units (metric) | Notes |
|---|---|---|---|
| `water_level` | `t`, `v`, `s`, `f`, `q` | `v` metres above the datum; `s` sigma, metres | `f` is `O,F,R,L` (count of 1 s samples outside 3 sigma, flat, rate-of-change and limit flags); `q` is `p` preliminary or `v` verified |
| `hourly_height` (verified) | `t`, `v`, `s`, `f` | metres | `f` is `I,L`; no `q`. October 2026 returned "No data was found" on 2026-11-08 because verification trails by about a month; January 2020 returned data |
| `water_temperature`, `air_temperature` | `t`, `v`, `f` | °C | `f` is `X,N,R` |
| `air_pressure` | `t`, `v`, `f` | hPa (millibar) | `f` is `X,N,R` |
| `wind` | `t`, `s`, `d`, `dr`, `g`, `f` | `s` and `g` m/s (knots with `english`), `d` degrees true, `dr` compass text (the documentation says only "wind direction in degrees"; the meteorological convention, the direction the wind comes from, is assumed and **unverified**) | `f` is `X,R`; The Battery has no wind sensor |
| `currents` (PORTS meters) | `t`, `s`, `d`, `b` | `s` cm/s (knots with `english`), `d` degrees true (the documentation says only "current direction in degrees"; whether it is the direction toward which the water flows is **unverified** for observations), `b` bin | |

Missing values: no `null` or empty `v` occurred in the recorded days. When there is nothing to return the
service answers HTTP 200 with `{"error": {"message":"No data was found. This product may not be offered at this
station at the requested time."}}`. Parsers must therefore check for an `error` key before `data` or
`predictions`. Flags are not a missing marker: a reading with a raised flag still has a value.

`units=english` was spot-checked (predictions in feet: `5.614`; wind and water temperature in knots and
°F). **Unverified**: temperature unit text under `english` for every product, `humidity`, `visibility`,
`conductivity`, `salinity`, `high_low` (observed), `daily_max_min`, `one_minute_water_level`, `air_gap`,
`ofs_water_level`, `expand=detailed` for currents, and Great Lakes stations (the documentation says predictions
do not exist there).

### 2.5 Datums and the prediction datum

Use `datum=MLLW`. Predictions at 8518750 are exact shifts of one another between datums: on
2026-11-08 the 05:59 low is `0.017` (MLLW), `-0.766` (MSL) and `1.019` (STND); these differ by 0.783 m and
1.002 m, which are `MSL - MLLW` and `STND`-to-`MLLW` in the station's datum table. Predictions are therefore the
sum of a constant (the datum offset) and the harmonic series (section 3.3).

### 2.6 Maximum range per request

Documentation versus what the service enforced on 2026-11-08:

| Request | Documented | Observed |
|---|---|---|
| `predictions` `hilo` | 10 years | message `size limit … is 3655 days`; an 11-year request got HTTP 400. A 3-year request succeeded but took 7.8 s, a 5-year and a 10-year request ended with HTTP 504 after about 10 s. Use at most 1 year per request |
| `predictions` other intervals | 1 year | not enforced up to 800 days at `interval=6` (7.3 MB) and 3 years at `interval=60`; stay at 1 month per request for `interval=6` (about 7,200 points, 280 KB) |
| `currents_predictions` `max_slack` | 1 year | message `size limit … is 366 days`; 365 days returned 2,822 events (351 KB); 400 days returned HTTP 400 |
| `currents_predictions` other intervals | 1 month | 61 days at `interval=60` accepted; use at most 1 month |
| `water_level` (6-minute) | 1 month | message `size limit … is 31 days`; 2026-10-01 to 2026-11-01 accepted, to 2026-11-03 rejected |
| `hourly_height`, `high_low` | 1 year | not tested (**unverified**) |
| `currents` with `bin=0` | 7 days | not tested (**unverified**) |

### 2.7 Error shapes

Fixtures: `coops-error-*.json|txt` and `coops-8518750-wind-2026-11-07-error-nodata.json`.

| Case | HTTP | Body |
|---|---|---|
| no data / product not offered at the station / subordinate station asked for `interval=6` | 200 | `{"error": {"message":"No data was found. This product may not be offered at this station at the requested time."}}` or, for a subordinate tide station, `{"error": {"message":"No Predictions data was found. Please make sure the Datum input is valid."}}` |
| missing datum, unit, time zone, or end before begin, range too long, bad bin, unknown station | 400 | `{"error": {"message":" … "}}` with leading and trailing blank lines and leading space in the message |
| missing `product` or `format` | 400 | plain text (not JSON, though the content type says JSON) such as `\n Wrong Product : Product cannot be null or empty \n\n\n` |
| gateway time-out on very long requests | 504 | HTML page (`text/html`) |

Parse by trimming whitespace, then try JSON, and treat anything else as an error.

## 3. Tide predictions and harmonic constituents

### 3.1 Metadata API base and request shapes

Base: `https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi` ("v1.0" in the documentation; no version in the
path). JSON by extension (`.json`) or `Accept: application/json`; XML otherwise. No authentication, no
`User-Agent` requirement, no rate-limit headers. Responses carry `access-control-allow-origin: *`.

| Resource | URL |
|---|---|
| station record | `/stations/{id}.json` (`?expand=details,sensors,products,datums,harcon,tidepredoffsets,currentpredictionoffsets,bins,deployments`) |
| station list | `/stations.json?type={waterlevels,tidepredictions,currentpredictions,currents,harcon,datums,met,…}` |
| datums | `/stations/{id}/datums.json[?units=metric]` |
| harmonic constituents | `/stations/{id}/harcon.json[?units=metric][&bin=N]` |
| tide offsets | `/stations/{id}/tidepredoffsets.json` |
| current offsets | `/stations/{id}.json?expand=currentpredictionoffsets`, or `/stations/{id}_{bin}/currentpredictionoffsets.json` (the `{id}` path without the bin suffix returns 404 although the documentation lists it) |
| bins | `/stations/{id}/bins.json[?units=metric]` |

### 3.2 Station lists

Differences from the documentation: the JSON list is under the key `stations` (the documentation says
`stationList`); `count` is the full count, `units` is `null`. Sizes on 2026-11-08: `waterlevels` 302 stations
(777 KB, also returned by `stations.json` with no `type`), `tidepredictions` 3502 (2.0 MB), `currentpredictions`
4499 (3.8 MB), `currents` 85 (104 KB), `harcon` 1374 (3.5 MB). Download once, cache for days, and filter by
bounding box locally. Fixtures are trimmed to New York except `currents`.

Tide-prediction station fields: `id`, `name`, `lat`, `lng`, `state`, `type` (`R` reference/harmonic, `S`
subordinate), `reference_id` (subordinate only), `timemeridian`, `timezonecorr`, `tidepredoffsets.self`.
Counts: 1260 `R`, 2242 `S`. Current-prediction station fields: `id`, `name`, `lat`, `lng`, `type` (`H` harmonic,
`S` subordinate, `W` weak and variable), `currbin`, `depth` (**feet**, `null` when unknown), `depthType`
(`U` unknown, `B` below chart datum, `S` below surface), `timezone_offset`. A station with several bins is
**listed once per bin** (NYH1927 appears with `currbin` 13, 8 and 2), and the `id` repeats.

### 3.3 Harmonic constituents (`harcon`) of a tide station

Request: `GET /stations/8518750/harcon.json` (fixture `coops-mdapi-8518750-harcon.json`) or `?units=metric`
(fixture `coops-mdapi-8518750-harcon-metric.json`).

Response: `{"units": "feet" | "meters", "HarmonicConstituents": [ … ], "self": "…"}`. Each element:

| Field | Type | Meaning, unit |
|---|---|---|
| `number` | integer | constituent number (1 to 37 at The Battery) |
| `name` | string | constituent name (list below) |
| `description` | string | text |
| `amplitude` | number | **feet by default**; metres with `units=metric`. Three decimals in either unit, so the metric amplitudes are rounded to 1 mm |
| `phase_GMT` | number | degrees, 0 to 360, **phase lag against the equilibrium argument at Greenwich, referenced to UTC**. Independent of `units` |
| `phase_local` | number | degrees, the same lag referenced to the station's local standard time. Independent of `units` |
| `speed` | number | degrees per hour (360 divided by the period in hours). Independent of `units` |
| `comments` | string | source note: "Vector Average of 5 years 2018-2022. SA and SSA from an average of 16 years 2002-2022." |

Important points for offline prediction:

- **Units.** The default is feet, not metres. `units=metric` multiplies the amplitude by 0.3048 and rounds to
  three decimals; phases and speeds do not change. M2 is `2.201` ft or `0.671` m. Request the default (feet) for
  the least rounding error (0.001 ft is 0.3 mm), or use metric directly; the difference is below 0.5 mm.
- **Phase reference.** Use `phase_GMT` with arguments computed from UTC. `phase_local` equals
  `phase_GMT - speed * H` modulo 360, with `H` the hours by which local standard time is behind UTC (5 for
  The Battery; checked against all 37 constituents there and against San Francisco with `H` = 8, the only
  exceptions being constituents with zero amplitude and phase 0). Using `phase_local` with UTC arguments gives
  errors of about 1 m.
- **Constituent names** at 8518750, in the order returned (37): `M2, S2, N2, K1, M4, O1, M6, MK3, S4, MN4, NU2,
  S6, MU2, 2N2, OO1, LAM2, S1, M1, J1, MM, SSA, SA, MSF, MF, RHO, Q1, T2, R2, 2Q1, P1, 2SM2, M3, L2, 2MK3, K2,
  M8, MS4`. `MM`, `MSF` and `MF` are present with amplitude 0.0 and phase 0.0 (the comment says Mm and Msf are
  unstable and not used). Spellings are as above (`LAM2`, `2SM2`, `RHO`).
- **No mean-level constituent.** The list has no `Z0`. The constant is derived from the datum table
  (`/stations/8518750/datums.json`, fixtures `coops-mdapi-8518750-datums.json` in feet and
  `coops-mdapi-8518750-datums-metric.json` in metres): for predictions at datum `D`, `Z0(D) = MSL - D`
  measured in the datum table. For MLLW the value is `MSL - MLLW = 1.785 - 1.002 = 0.783 m` (5.86 - 3.29 =
  2.57 ft). A least-squares fit of the constant to the week of 6-minute predictions gave 0.7830 m. `MTL - MLLW`
  (0.753 m) fits clearly worse. The datum epoch is 1983-2001, accepted 2012-11-19; the constituents come from
  2018-2022.
- **Subordinate stations have no constituents** (`HarmonicConstituents: []` at Tarrytown, fixture
  `coops-mdapi-8518919-harcon.json`) and their datum table is empty (all fields null).

Independent check, performed with a short script outside this repository (Schureman node factors and
Greenwich equilibrium arguments at UTC; the lunar-node, perigee and mean longitudes from Meeus): the model

```text
h(t) = Z0 + sum over constituents of f * A * cos( V0(t) + u - phase_GMT )
```

with `A` the amplitude in metres, `f` and `u` the node factor and phase correction of the year, and `V0` the
equilibrium argument at UTC, reproduced the 1680 six-minute fixture values with an RMS of 7 mm and a maximum
of 20 mm, and the 27 high and low times to within 0 to 2 minutes early (the model extremes differing from the
fixture values by -11 to +12 mm). The remainder comes from the simplified treatment of `L2`, `M1` and the
compound constituents in that script, so the implementation should be able to do better. The convention itself
(cosine of equilibrium argument minus `phase_GMT`, with `phase_GMT` and not `phase_local`) is confirmed.

### 3.4 Subordinate tide station offsets

Request: `GET /stations/8518919/tidepredoffsets.json` (Tarrytown; fixture
`coops-mdapi-8518919-tidepredoffsets.json`):

| Field | Value at Tarrytown | Meaning |
|---|---|---|
| `refStationId` | `8518750` | reference (harmonic) station |
| `type` | `S` | subordinate |
| `timeOffsetHighTide` | 109 | minutes added to the reference high-tide time |
| `timeOffsetLowTide` | 117 | minutes added to the reference low-tide time |
| `heightOffsetHighTide` | 0.7 | high tide height adjustment |
| `heightOffsetLowTide` | 0.7 | low tide height adjustment |
| `heightAdjustedType` | `R` | `R` means the height offsets are **ratios** (multiply the reference height); the documentation does not define `F` (fixed addend) and it was not observed (**unverified**) |

At a reference station the same endpoint returns `type: "R"`, empty `refStationId` and all offsets `null`.

Verified against the fixtures: Battery high `2026-11-08 12:07` / 1.711 m gives Tarrytown high `13:56` / 1.198 m
(+109 min, x0.7), Battery low `05:59` / 0.017 m gives Tarrytown low `07:56` / 0.012 m (+117 min, x0.7). The
heights stay relative to MLLW. Subordinate stations support only `interval=hilo` and `datum=MLLW`.

## 4. Currents

### 4.1 Stations and bins

Harmonic station used: **NYH1927 Hudson River Entrance** (40.7076, -74.0253). It is in the
`currentpredictions` list as type `H` with `currbin` 13, 8 and 2 (nominal depths 7, 23 and 43 ft below chart
datum); a 2019 survey, so predictions only (no real-time data). Its `bins.json` lists 14 bins with depths in
metres (13.99 m for bin 1 to 0.98 m for bin 14; bin 13 is 1.98 m); `stations/NYH1927.json` includes `deployments`.

Real-time PORTS meters near New York in the active `currents` list: `n03020` The Narrows, `n06010` Kill Van Kull,
`n07010` Newark Bay entrance. `n01010` Bergen Point and `n02010` Bayonne Bridge are prediction stations only.

### 4.2 Predictions: `currents_predictions`

`bin` is optional for tidal current prediction stations: without it the bin nearest the surface is used (for
NYH1927 that is bin 13, depth 2.1 m). An invalid `bin` returns HTTP 400 with the valid ones:
`{"error": {"message":"Available bin number of NYH1927: 13, 8, 2, "}}`. Different bins give different phases and
amplitudes, so record the bin with the spot.

`interval=max_slack` (fixtures `coops-NYH1927-bin13-max-slack-2026-11-08.json`,
`coops-ACT3646-bin1-max-slack-2026-11-08.json`), body `{"current_predictions":{"units":"meters, cm/s","cp":[ … ]}}`:

| Field | Type | Meaning |
|---|---|---|
| `Type` | string | `flood`, `ebb` (maximum) or `slack` |
| `Time` | string | `YYYY-MM-DD HH:MM`, UTC with `gmt` |
| `Velocity_Major` | number | speed along the major axis, **cm/s with `units=metric`, knots with `english`**; positive toward `meanFloodDir`, negative toward `meanEbbDir`; slack events are near zero (-0.2 to 0.2) |
| `meanFloodDir`, `meanEbbDir` | integer | degrees true, direction the water flows toward in flood and ebb |
| `Bin` | string | bin number |
| `Depth` | string or `null` | bin depth, metres with `metric` (`"2.1"`), feet with `english` (`"7"`); `null` at subordinate stations |

A day at NYH1927 bin 13 contains 8 events (`flood`, `slack`, `ebb`, `slack`, repeated twice). Sample: `{"Type":"ebb",
"meanFloodDir":11,"Bin":"13","meanEbbDir":183,"Time":"2026-11-08 06:30","Depth":"2.1","Velocity_Major":-117.5}`.

`interval=6` (also 1, 10, 30, 60, `h`) at a harmonic station: the same object without `Type`, one point per
step (fixture `coops-NYH1927-bin13-currents-predictions-6min-2026-11-08.json`, 1680 points for the week).

`vel_type=speed_dir` with `interval` 1, 6, 10, 30 or 60 returns `{"Speed":"60.056","Bin":"13","Time":"…",
"Direction":11,"Depth":"2.1"}` (speed as a string, cm/s or knots; direction in degrees true the water flows
toward) (fixture `coops-NYH1927-bin13-speed-dir-6min-2026-11-08.json`). With `interval=max_slack` it returns HTTP
400: `vel_type= speed_dir only supports current prediction intervals of 1, 6, 10, 30, 60.   Set vel_type = default
for interval = max_slack`. Once, an otherwise identical request returned the default shape (no `Speed`), then the
speed/direction shape on a repeat; check which of `Velocity_Major` and `Speed` is present rather than assuming.

**Harmonic and subordinate stations differ.** A subordinate station (ACT3646, type `S`) has no constituents and
answers `max_slack` only: a request with `interval=6` silently returned the `max_slack` events (the response is
identical to the `max_slack` response; no error). Its `Depth` is `null` and `currbin` is 1.

### 4.3 Current harmonic constituents

`GET /stations/NYH1927/harcon.json[?bin=13][&units=english]` (fixtures `coops-mdapi-NYH1927-harcon.json`, all
bins, and `coops-mdapi-NYH1927-harcon-bin13-english.json`). Different from tide stations:

- The default units are `"meters, centimeters/second"` (amplitudes in cm/s); `units=english` gives
  `"feet, knots"`. All bins are returned unless `bin` is given (75 elements for three bins).
- Fields per element: `binNbr`, `binDepth` (metres or feet, 1.98 m for bin 13, which differs from the `Depth`
  `2.1` in predictions), `constNum`, `constituentName`, `description`, `majorAmplitude`, `majorPhase` (local),
  `majorPhaseGMT`, `minorAmplitude`, `minorPhase`, `minorPhaseGMT`, `majorMeanSpeed`, `minorMeanSpeed`, `azi`
  (degrees true of the major axis).
- 25 constituents (no `NU2`, `MU2`, `LAM2`, `S1`, …): `M2, S2, N2, K1, M4, O1, M6, MK3, S4, MN4, S6, 2N2, OO1,
  M1, J1, Q1, 2Q1, P1, 2SM2, M3, L2, 2MK3, K2, M8, MS4`. There is no `speed` field; take the angular speed from
  the constituent name.
- `majorMeanSpeed` (-20.37 cm/s for bin 13) is the non-tidal mean along the major axis and is the equivalent of
  `Z0`. It is the same for every constituent of a bin.

The check script reproduced one day of 6-minute `Velocity_Major` for bin 13 with
`v = majorMeanSpeed + sum(f * majorAmplitude * cos(V0 + u - majorPhaseGMT))` to an RMS of 3.7 cm/s (maximum 5.9)
against peak speeds of about 130 cm/s, positive in the flood direction. Using `majorPhase` instead gives errors
of about 1.4 m/s. The minor axis was not checked (**unverified**); the published predictions use the major axis
(`Velocity_Major`) with the mean flood and ebb directions, and how `Speed` and `Direction` are
derived from the major and minor axes was not checked (**unverified**).

### 4.4 Subordinate current offsets

`GET /stations/ACT3646.json?expand=currentpredictionoffsets` (fixture `coops-mdapi-ACT3646-station.json`):

| Field | Value | Meaning |
|---|---|---|
| `refStationId`, `refStationBin` | `n03020`, 7 | reference harmonic station and bin |
| `meanFloodDir`, `meanEbbDir` | 31.0, 205.0 | degrees true at the subordinate station |
| `mfcTimeAdjMin`, `mecTimeAdjMin` | 58, 60 | minutes added to the reference max flood, max ebb |
| `sbfTimeAdjMin`, `sbeTimeAdjMin` | 80, 53 | minutes added to the reference slack before flood, slack before ebb |
| `mfcAmpAdj`, `mecAmpAdj` | 1.05, 1.19 | ratios applied to the reference max flood and max ebb speeds |

Verified against `n03020` bin 7 on 2026-11-08: reference max flood `11:34` / 88.2 cm/s gives `12:32` / 92.6;
reference max ebb `17:01` / -88.0 gives `18:01` / -104.7; reference slack before ebb `13:47` gives `14:40`;
reference slack before flood `07:56` gives `09:16`.

### 4.5 Observed currents (`currents`)

`n06010` and `n07010` returned data (fixture `coops-n06010-currents-2026-11-07.json`: fields `t`, `s` cm/s, `d`
degrees, `b` bin; the meter reports a fixed bin). `n03020`, `n02010` and `n01010` returned "No data was found" for
the last hours and for 2026-11-07, although `n03020` is in the active `currents` list; treat observed currents as
frequently absent. `bin` is optional at PORTS meters (a predefined bin is used). Unknown station ids return
HTTP 400.

## 5. Units to convert

| Datum | Where | Units to convert |
|---|---|---|
| tide height | predictions `v`, observations `v` | metres above datum, metric; the core's canonical unit |
| water temperature | `water_temperature` `v` | °C |
| pressure | `air_pressure` `v` | hPa |
| wind | `wind` `s`, `g`, `d` | m/s, m/s, degrees true from |
| current speed | `Velocity_Major`, `Speed`, `s` | **cm/s**, divide by 100 for m/s |
| current direction | `meanFloodDir`, `meanEbbDir`, `Direction`, `d` | degrees true toward |
| harmonic amplitude (tide) | `amplitude` | feet unless `units=metric` |
| harmonic amplitude (current) | `majorAmplitude` | cm/s unless `units=english` |
| datum values | `datums[].value` | feet unless `units=metric` |

## 6. Licence, terms and attribution

- Data on government servers is public domain "unless specifically annotated otherwise and may be used freely
  by the public" (disclaimers page). The page asks that **attribution be given whenever NOS material is
  reproduced and re-disseminated**, and that information not be modified and then presented as official
  government material. It also says to pay attention to the date and time of the data, and that NOS provides
  data "as is" with no warranty.
- Raw observations are described as preliminary: "released for limited public use as preliminary data to be used
  only with appropriate caution". Suggested credit text: "Tide, water-level and current data: NOAA/NOS CO-OPS."
- The API documentation asks users to space requests and to set `application`; it states no numeric limit.
- The documentation page for web services says data services moved to the cloud and asks callers to use the
  updated URLs (`api.tidesandcurrents.noaa.gov`); the older `tidesandcurrents.noaa.gov/api/datagetter` host
  was not tested (**unverified**).
- Tide and current predictions are not for navigation; display "not for navigation" next to charts and currents.

## 7. Outages, changes and deprecations

- The `datums` product no longer exists on the Data API; use mdapi (documentation).
- A service change notice dated 2026-09-11 retires the legacy sea-level trend product on 2026-10-01 in favour of
  the Derived Product APIs (`api.tidesandcurrents.noaa.gov/dpapi/prod/`); it does not concern `datagetter`.
- A 504 gateway time-out occurred on 5-year and 10-year `hilo` requests (2026-11-08); no other outage was
  seen. No dated deprecation of `datagetter` or `mdapi` was found.
- The documentation example for `harcon` of a current station shows `feet, knots`; the live default is
  `meters, centimeters/second`.

## 8. Addendum 2026-11-15: far-future predictions, a second current bin set and a diurnal station

Recorded with the `COMMON` query of section 2.2 (`application=underbite&time_zone=gmt&units=metric&format=json`).
The files are listed in `tests/fixtures/raw/coops/README.md`.

**How far ahead predictions are served.** Predictions for 8518750 (`datum=MLLW`, `interval=hilo`, one day
each) were returned for 2034-06-01, 2035-06-01, 2040-06-01, 2050-06-01 and 2100-06-01. A request for 2101-06-01
returned `{"error": {"message":"No Predictions data was found. Please make sure the Datum input is valid."}}`.
So predictions are served to the end of 2100 (the 2101 request was the only one probed past it). Fixtures use
2034-06-01..07, about half a lunar-node cycle (18.6 years) after 2026:

| Fixture | Content |
|---|---|
| `coops-8518750-hilo-2034-06-01.json` | 28 events, first `2034-06-01 00:13` H 1.63 m, last `2034-06-07 22:38` L 0.288 m |
| `coops-8518750-predictions-6min-2034-06-01.json` | 1680 points `2034-06-01 00:00` 1.626 to `2034-06-07 23:54` 0.381 |

Heights are metres above MLLW, times GMT, in the shapes of section 2.4. A one-week `hilo` and a one-week
`interval=6` request were accepted (the 5-year and 10-year `hilo` 504 of section 7 concerns range, not horizon).

**Reference current station of ACT3646: `n03020`, bin 7.** `product=currents_predictions&station=n03020&bin=7&
interval=max_slack&begin_date=20261108&end_date=20261114` returned 54 events with `"units":"meters, cm/s"`;
each carries `Type` (`slack`, `flood` or `ebb`), `Time` (`YYYY-MM-DD HH:MM` GMT), `Depth` (`3.9` m),
`Velocity_Major` (cm/s, signed: negative on the ebb), `meanFloodDir` 325, `meanEbbDir` 143 and `Bin` `"7"` as in
section 4.2. First event: slack `2026-11-08 01:10` -0.2; then ebb `04:16` -103.2 cm/s. The harmonic
constituents are `GET /stations/n03020/harcon.json?bin=7` (fixture `coops-mdapi-n03020-harcon-bin7.json`):
`{"units":"meters, centimeters/second","HarmonicConstituents":[...]}` with 29 constituents, each carrying
`binNbr` 7, `binDepth` 3.93, `constNum`, `constituentName`, `majorAmplitude`, `majorPhase`, `majorPhaseGMT`,
`minorAmplitude`, `minorPhase`, `minorPhaseGMT`, `majorMeanSpeed`, `minorMeanSpeed` and `azi`; M2 has major
amplitude 79.94 cm/s, GMT phase 344.6 and `azi` 323.6. The bin selects the depth at which the constituents are
valid.

**Pensacola FL (8729840), a diurnal harmonic station.** `stations/8729840/tidepredoffsets.json` returns
`"type": "R"` (reference, that is harmonic) with zero offsets, and the station record (`stations/8729840.json`)
has `tideType: Diurnal`, `timezone` CST, `timezonecorr` -6, `lat` 30.4044, `lng` -87.2112, `shefcode` PCLF1.
Fixtures:

- `harcon.json?units=metric`: `{"units":"meters","HarmonicConstituents":[...]}` with 37 constituents, amplitude in
  metres and phase in degrees (`phase_GMT`, `phase_local`). The dominant constituents are diurnal: K1 amplitude
  0.125 m (GMT phase 58.2), O1 0.122 (47.9), P1 0.037, Q1 0.026, against M2 0.017 (172.1) and S2 0.005; the
  `comments` field of M2 reads "Vector average of 5 years (2018-2022)".
- `datums.json?units=metric`: epoch 1983-2001, accepted `Apr 15 2004`, `OrthometricDatum` NAVD88,
  `units` meters; datums are relative to `STND` (`MHHW` 2.952, `MHW` 2.943, `MSL` 2.757).
- `predictions` with `datum=MLLW`, `hilo`, 2026-11-08..14: 14 events, one high and one low a day (first
  `2026-11-08 03:09` H 0.464 m, last `2026-11-14 18:53` L -0.002 m); the 6-minute series has 1680 points
  (`2026-11-08 00:00` 0.396 m). Values can be slightly negative against MLLW.
