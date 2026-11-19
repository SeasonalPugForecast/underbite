# Open-Meteo Marine API

Verified on 2026-11-08 from the official documentation and live requests. Recorded responses are in
`tests/fixtures/raw/open-meteo-marine/` (index: `README.md` there). Anything not checked is marked
**unverified**.

Documentation pages used:

- API reference: <https://open-meteo.com/en/docs/marine-weather-api>
- Terms and rate limits: <https://open-meteo.com/en/terms>
- Licence and attribution: <https://open-meteo.com/en/licence>

## Summary

| Item | Finding |
|---|---|
| Base URL | `https://marine-api.open-meteo.com/v1/marine` (HTTPS, GET, no version header; the path carries `v1`) |
| Authentication | none for the free, non-commercial API; `apikey` parameter (and a `customer-` host prefix) only for paid commercial use |
| Format | JSON, the same envelope as Open-Meteo's forecast API (`latitude`, `longitude`, `elevation`, `*_units`, column arrays). CSV and XLSX also exist (**unverified**, not requested) |
| Missing values | JSON `null` inside the column arrays; HTTP 200 |
| Time | `timezone=GMT` by default, ISO 8601 without offset (`2026-11-08T14:00`), or `timeformat=unixtime`; with a named `timezone` the response carries `utc_offset_seconds` and local times |
| Rate limits (free) | "Less than 10'000 API calls per day, 5'000 per hour and 600 per minute" (terms page; the same page's comparison table also lists 300,000 per month). Limits are per IP address and per application. No rate-limit headers were returned (response headers inspected on several requests: only `Date`, `Content-Type`, `Transfer-Encoding`, `Connection`) |
| Terms | Free API for non-commercial use only; "Websites or apps that have subscriptions or display advertisements" count as commercial (terms page). Misuse may be blocked without notice |
| Licence of the data | CC BY 4.0 |
| User-Agent | not required; a descriptive one was sent on every request |

## Coastal-accuracy caveat

The API page states, next to the tide and current variables: "Accuracy at coastal areas is limited" and
"This is not suitable for coastal navigation and does not replace your nautical almanac."
Source: <https://open-meteo.com/en/docs/marine-weather-api> (section on `sea_level_height_msl` and ocean
currents). Tides and currents are computed at 0.08° (about 8 km). Any display of these values should
say they are modelled and not for navigation.

Observed on 2026-11-08 against NOAA data, as an illustration of that caveat:

- CO-OPS 8518750 (The Battery) predicted lows of -0.77 m (05:59 UTC) and -0.77 m (18:34 UTC) and a high of
  +0.93 m (12:07 UTC) relative to MSL. The model grid cell nearest the harbour (40.625, -74.04166) put its
  lows at 04:00 and 17:00 UTC (-1.31, -1.42 m) and its high at 10:00 UTC (+0.22 m): earlier by one to two
  hours, a smaller range, and a mean of about -0.6 m over 48 h. `sea_level_height_msl` is therefore not
  interchangeable with a local-MSL tide curve.
- NDBC 44025 reported 1.9 m significant wave height and 15.9 °C water temperature at 13:10 UTC; the model
  cell near it (40.2917, -73.125) gave 1.50 m and 16.0 °C at 14:15 UTC.

## Requests the app needs

All examples use `B=https://marine-api.open-meteo.com/v1/marine`.

1. Hourly forecast (waves, SST, currents, sea level) for a point:

   ```text
   B?latitude=40.70&longitude=-74.02&hourly=wave_height,wave_direction,wave_period,wind_wave_height,wind_wave_direction,wind_wave_period,swell_wave_height,swell_wave_direction,swell_wave_period,secondary_swell_wave_height,secondary_swell_wave_period,secondary_swell_wave_direction,sea_surface_temperature,ocean_current_velocity,ocean_current_direction,sea_level_height_msl&forecast_days=2&timezone=UTC&wind_speed_unit=ms
   ```

2. 15-minute currents and sea level: `minutely_15=ocean_current_velocity,ocean_current_direction,sea_level_height_msl`
   (plus `forecast_days`/`past_days`, or `forecast_minutely_15`/`past_minutely_15`).
3. Current block (latest 15-minute step): `current=wave_height,wave_period,wave_direction,sea_surface_temperature,ocean_current_velocity,ocean_current_direction,sea_level_height_msl`.
4. Daily summary (needs `timezone`): `daily=wave_height_max,wave_direction_dominant,wave_period_max,swell_wave_height_max`.
5. History: `start_date=YYYY-MM-DD&end_date=YYYY-MM-DD` on the same endpoint; `models=era5_ocean` for the long record.

Coordinates are rounded by the app before sending; the response echoes the grid cell actually used.

### Parameters (observed)

| Parameter | Observed behaviour |
|---|---|
| `latitude`, `longitude` | required. Comma-separated lists for several places are documented (**unverified**). Omitting them returned HTTP 200 with an **empty body**, not a JSON error, so a parser must treat an empty body as a failure |
| `hourly`, `daily`, `current`, `minutely_15` | comma-separated variable names; an unknown name returns HTTP 400 `{"error":true,"reason":"Invalid value: Cannot initialize ... from invalid String value bogus"}` (`marine-error-invalid-variable.json`) |
| `forecast_days` | 0 to 16 accepted (the error text says "Allowed range 0 to 16"; the docs page lists 0 to 8 and a default of 5 or 7, its own text conflicts). 16 returned 384 hourly steps; 20 returned HTTP 400 (`marine-error-forecast-days.json`). The effective length depends on the model; best match returned 16 days of wave variables at New York (other variables were not checked at 16 days) |
| `past_days` | docs say 0 to 92. 92 and 93 and 200 were all accepted and returned the full number of hours (2208, 2232, 4800). Treat the documented 92 as the contract |
| `start_date`, `end_date` | allowed range reported by the server on 2026-11-08: `1940-01-01` to `2026-11-23` (today plus 15 days). A date beyond the range returns HTTP 400 `{"reason":"Parameter 'end_date' is out of allowed range from 1940-01-01 to 2026-11-23","error":true}` |
| `forecast_hours`, `past_hours`, `start_hour`, `end_hour` | documented; **unverified** (not requested) |
| `timezone` | `UTC`/`GMT` default; `America/New_York` returned `utc_offset_seconds: -18000` (EST on 2026-11-08) and local times. `auto` is documented; **unverified** |
| `timeformat` | `unixtime` returned epoch seconds (`1794114000` for local midnight with `timezone=America/New_York`) |
| `length_unit` | `metric` (default) or `imperial`; `imperial` turns `wave_height` to `ft` (`5.315` for 1.62 m) |
| `temperature_unit` | `fahrenheit` turns `sea_surface_temperature` to `°F`. Not listed on the marine docs page, but accepted |
| `wind_speed_unit` | the parameter that sets the **ocean current** unit: `kmh` (default), `ms`, `mph`, `kn`. The response unit strings are `km/h`, `m/s`, `mp/h` (sic), `kn`. The documentation's "velocity unit" name `velocity_unit` is **not** honoured (ignored, stayed `km/h`) |
| `cell_selection` | `sea` is the effective default (a land-side request was moved to a sea cell); `nearest` and `land` at 40.70, -74.02 both snapped to the land-side cell 40.708, -74.042 and returned all nulls. The docs page contradicts itself on the default |
| `models` | see Models below. An unknown value returns HTTP 400 `Cannot initialize MultiDomains from invalid String value` |

### Grid snapping at the test places

| Requested | Cell returned (`latitude`, `longitude`) | Note |
|---|---|---|
| 40.70, -74.02 (The Battery area) | 40.625, -74.04166 | about 8 km south in the lower bay near Sandy Hook; the default `sea` selection avoids the land cell at 40.708, -74.042, which holds only nulls for waves |
| 40.25, -73.16 (near NDBC 44025) | 40.291664, -73.12499 (default best match); 40.25, -73.25 (`ncep_gfswave025`); 40.246044, -73.1882 (`ecmwf_wam`); 40.5, -73.0 (`era5_ocean`) | different models use different grids |
| 39.0, -98.0 (Kansas, land) | 39.041664, -98.04166, `elevation` 402 | HTTP 200, all values null; no error |
| 42.0, -81.5 (open Lake Erie) | 42.041664, -81.54166, `elevation` 170 | waves present; SST, current and sea level null |

## Variables

Hourly (and `current`) variables, from the docs and confirmed in the 2026-11-08 responses for the default
(best match) model. "Filled" is for the two US test points.

| Variable | Unit (`*_units`) | Filled by best match? | Notes |
|---|---|---|---|
| `wave_height` | `m` (`ft` with imperial) | yes | combined sea state (the docs group it under "mean waves") |
| `wave_direction` | `°` | yes | direction the waves come **from** (0 = north) |
| `wave_period` | `s` | yes | mean period |
| `wave_peak_period` | `undefined` | **no, always null** | the unit string is the literal `undefined`; only some models fill it (`ecmwf_wam025` returned `s` and values) |
| `wind_wave_height`, `wind_wave_direction`, `wind_wave_period` | `m`, `°`, `s` | yes | |
| `wind_wave_peak_period` | `undefined` | no, null | |
| `swell_wave_height`, `swell_wave_direction`, `swell_wave_period` | `m`, `°`, `s` | yes | primary swell train |
| `swell_wave_peak_period` | `undefined` | no, null | |
| `secondary_swell_wave_height`, `_period`, `_direction` | `m`, `s`, `°` | yes | secondary train |
| `tertiary_swell_wave_height`, `_period`, `_direction` | `undefined` | **no, always null** | docs: only the GFS wave models fill the third train. With `models=ncep_gfswave025` the fields are present (`m`, `s`, `°`) but were `0` at the offshore point (no third train) |
| `sea_surface_temperature` | `°C` (`°F`) | yes (not Great Lakes) | |
| `ocean_current_velocity` | `km/h` by default (see `wind_speed_unit`) | yes (not Great Lakes) | speed of the SMOC surface current. Values at the test places were small: 0.2 to 0.7 km/h in the lower bay, 1.0 to 1.5 km/h offshore |
| `ocean_current_direction` | `°` | yes (not Great Lakes) | direction the current flows **toward** |
| `sea_level_height_msl` | `m` | yes (not Great Lakes) | includes tides, relative to global mean sea level (not a local datum) |
| `invert_barometer_height` | `m` | yes | documented as a separate sea-level correction; returned `0.00` to `0.01` m in the first hours of the offshore series |

`undefined` in `hourly_units` means "this model does not provide the variable here"; the values are all null.
A parser should read the unit string of each column and treat `undefined` plus nulls as absent.

Daily variables confirmed: `wave_height_max`, `wave_direction_dominant`, `wave_period_max`,
`swell_wave_height_max` (units `m`, `°`, `s`, `m`). The docs also list `wind_wave_*_max`,
`*_dominant` and `*_peak_period_max` forms (**unverified**).

### Time resolution

- The docs list 15-minute data (`minutely_15`) only for `ocean_current_velocity`,
  `ocean_current_direction` and `sea_level_height_msl`, native in North America and Central Europe and
  interpolated from hourly elsewhere. For the New York point, `sea_level_height_msl` changed smoothly every
  15 minutes (`-0.15`, `-0.21`, `-0.27`, `-0.33`, ... m) while velocity and direction changed only about
  once an hour (values repeated across four steps). 96 steps came back for `forecast_days=1`.
- `current` returns one 15-minute step with `interval: 900` (for example `time: 2026-11-08T14:15`).
- Waves, SST and everything else are hourly; the docs list the source cadence as 3-hourly for MFWAM
  waves and 6-hourly for SST, interpolated to hourly (not checked in the output).

## Models and coverage

Default is `best_match`. Names seen to work: `meteofrance_wave`, `meteofrance_currents`, `ecmwf_wam025`,
`ecmwf_wam`, `ncep_gfswave025`, `era5_ocean`, `best_match`. The docs page also lists a `seamless` option and the DWD
and GFS 0.16° models; their exact identifiers were not requested (**unverified**).

| Data set (docs) | Region | Resolution | Temporal res. | Forecast length | History start | Update |
|---|---|---|---|---|---|---|
| MeteoFrance MFWAM | global | 0.08° | 3-hourly | 10 days | Oct 2021 | 12 h |
| MeteoFrance SMOC currents and tides | global | 0.08° | hourly | 10 days | Jan 2022 | 24 h |
| MeteoFrance SST | global | 0.08° | 6-hourly | 10 days | Jan 2022 | 24 h |
| ECMWF WAM | global | 9 km | hourly | 15 days | Nov 2025 | 6 h |
| ECMWF WAM 0.25 | global | 0.25° | 3-hourly | 15 days | Mar 2024 | 6 h |
| NCEP GFS Wave 0.25° | global | 0.25° | hourly | 16 days | Jun 2024 | 6 h |
| NCEP GFS Wave 0.16° | 52.5° N to 15° S | 0.16° | hourly | 16 days | Oct 2024 | 6 h |
| DWD EWAM | Europe | 0.05° | hourly | 78 h | Aug 2022 | 12 h |
| DWD GWAM | global | 0.25° | hourly | 174 h | Aug 2022 | 12 h |
| ERA5-Ocean | global | 0.5° | hourly | none | 1940 to present, 5-day delay | daily |

What best match gave at New York: `meteofrance_wave` for waves (same numbers as the best-match waves) and
`meteofrance_currents` for SST, currents and sea level (same numbers as best match). Single-model requests
return `undefined` and nulls for the variables the model lacks (`meteofrance_wave` has no SST, current or
sea level; `meteofrance_currents` has no waves).

Observed data availability by date at 40.25, -73.16 (best match):

| Date requested | `wave_height` | `ocean_current_velocity` | `sea_surface_temperature` | `sea_level_height_msl` |
|---|---|---|---|---|
| 2021-12-15 | values | not requested | null | null |
| 2022-02-01 | values | values | null | null |
| 2022-06-01 | not requested | values | null | null |
| 2022-12-01 | not requested | values | values | values |
| 2024-07-01 | values | values | values | values |

So SST and sea level start between June and December 2022 (not bisected further); currents start by
February 2022 (the docs say January 2022). `models=era5_ocean` returned waves back to 1960 (checked
1960-07-01) with 0.5° cells, but `sea_surface_temperature`, current and sea level were null in that model
(checked 1960 and 2024). The 1940 start is the documented limit of the date parameter (the server
rejects earlier dates); a 1940 request was not made (**unverified**).

### Out-of-coverage and land behaviour

- A land cell returns HTTP 200 with the usual envelope, `elevation` greater than 0, and every requested
  value `null`. There is no error and no warning, so "no data" must be derived from all-null columns.
- With the default `sea` cell selection the API moves a coastal request to a nearby sea cell (New York:
  40.625, -74.04166). Requests that fall on a land-only area farther inland (Kansas) are not moved far and
  yield nulls.
- Great Lakes (open Lake Erie, 42.0, -81.5): wave columns are filled (about 1 m, 4 s on 2026-11-08), while
  `sea_surface_temperature`, `ocean_current_*` and `sea_level_height_msl` are null (the ocean products evidently do not cover lakes; the response gives no explanation).
- Dates before a variable's start return null columns, not errors (see the table above).
- Outside the supported date range, HTTP 400 as shown above. Error bodies have the shape
  `{"error": true, "reason": "<text>"}`.

## Response shape

```text
{
  "latitude": 40.625, "longitude": -74.04166, "generationtime_ms": 0.4,
  "utc_offset_seconds": 0, "timezone": "GMT", "timezone_abbreviation": "GMT", "elevation": 0.0,
  "hourly_units": { "time": "iso8601", "wave_height": "m", ... },
  "hourly": { "time": ["2026-11-08T00:00", ...], "wave_height": [0.26, ...], ... }
}
```

`minutely_15`/`minutely_15_units` and `daily`/`daily_units` have the same layout; `current` is an object of
scalars plus `time` and `interval` (seconds). Metric values have at most two decimals (imperial wave heights show three, e.g. `5.315`); direction
values are integers. The two-day New York response is about 7 KB; the 16-day, three-variable request
was a few kB.

## Licence, attribution and terms

- Data licence: CC BY 4.0 (<https://open-meteo.com/en/licence>, <https://open-meteo.com/en/terms>).
- The licence page asks for a visible credit linked to Open-Meteo next to any place where the data is shown:
  "Weather data by Open-Meteo.com" linking to <https://open-meteo.com/>. The credit must not imply
  endorsement.
- The API page additionally requires crediting the model providers actually used. For the marine data
  those are Copernicus Marine Service (MeteoFrance MFWAM, SMOC currents and tides, SST), ECMWF (WAM),
  NOAA NCEP (GFS Wave), DWD (EWAM, GWAM) and the Copernicus Climate Change Service (ERA5-Ocean). The
  exact wording per provider is on the licence page; the licence page also lists each provider's own
  licence.
- Free API: non-commercial use only. This covers an app without subscriptions or advertising; paid
  tiers apply otherwise. Because the licence is CC BY 4.0 but the free endpoint is non-commercial, a
  commercial fork must buy a subscription.
- Terms and rate limits can change; re-check the terms page before release.
- No deprecation or outage notices were found on the documentation pages on 2026-11-08 (**unverified**
  beyond that: the pages and the open-meteo.com status feed were not searched for incident history).

Re-checked 2026-11-18: licence and non-commercial terms unchanged; details read the same day:

- `https://open-meteo.com/en/licence`: API data under CC BY 4.0; "You must include a link next to any location Open-Meteo data are displayed", for example "Weather data by Open-Meteo.com". The page lists each data provider with a link to that provider's licence (DWD, ECMWF, Norwegian MI and others CC BY; UK Met Office CC BY-SA); it gives no per-provider credit wording. The per-model credit wording is on the API documentation page, not the licence page as written above.
- `https://open-meteo.com/en/docs/marine-weather-api`: "MeteoFrance MFWAM waves, SMOC currents and tides, and sea surface temperature: generated using E.U. Copernicus Marine Service Information (https://doi.org/10.48670/moi-00017, https://doi.org/10.48670/moi-00016)"; ECMWF WAM wave forecasts from ECMWF; GFS Wave forecasts from NOAA NCEP; EWAM and GWAM from DWD; "ERA5-Ocean wave reanalysis: generated using Copernicus Climate Change Service information (https://doi.org/10.24381/cds.adbb2d47)". "All users of Open-Meteo data must provide a clear attribution to the providers of the models they use as well as a reference to Open-Meteo."
- `https://open-meteo.com/en/terms`: free use is non-commercial, "Less than 10'000 API calls per day, 5'000 per hour and 600 per minute" (the comparison table adds 300,000 per month), limits per IP address and application, blocking without notice for misuse. Qualifying examples: "Private or non-profit websites or apps that do not have subscriptions or advertising"; voluntary donations and tip jars are permitted. Commercial: apps with subscriptions or advertisements, commercial products. Changes to the terms take effect when posted.
- Privacy section of the terms: web server logs "may contain sensitive information such as geographical coordinates", are not shared with third parties and are deleted after 90 days.

## Using the result

Suitable as a global fallback for waves, SST and a tide-like sea level where no CO-OPS or NDBC source
exists. Not suitable as the primary inshore source: the grid is 8 km, coastal accuracy is documented as
limited, tidal timing and range differ from CO-OPS (see above), and the same Great Lakes limits apply.
