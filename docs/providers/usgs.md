# USGS Water Data APIs

Verified on 2026-11-08 against the live service, with `curl` and without an API key. Recorded
responses are in `tests/fixtures/raw/usgs/` (index: `tests/fixtures/raw/usgs/README.md`). Anything not
checked is marked **unverified**.

## Summary

- Base URL `https://api.waterdata.usgs.gov/ogcapi/v1`; the response header `api-version` was `1.11.2`.
  The path segment `v1` is the API version; USGS documents a versioning policy at
  <https://api.waterdata.usgs.gov/docs/ogcapi/versioning/>.
- OGC API - Features. Responses are GeoJSON `FeatureCollection` objects (`application/json`, the default
  without `f=json` or an `Accept` header). Gzip is supported (`Accept-Encoding: gzip`).
- No account is needed. A free key raises the rate limit (see "Authentication and limits").
- The legacy `waterservices.usgs.gov` is being switched off; this app must use only the API above.
- A separate **Statistics API (beta)** exists and returns per-day-of-year percentiles. The assumption
  that the new platform has no statistics endpoint is wrong (see "Statistics API").
- Time series are mostly 15-minute instantaneous values; timestamps are UTC, daily values are dated
  in the site's standard time.

## Legacy retirement (dates confirmed 2026-11-08)

Source: USGS Water Data blog, "Intentional degradations to WaterServices ahead of final
decommissioning" (posted 2026-09-18, updated 2026-10-27),
<https://waterdata.usgs.gov/blog/wdfn-waterservices-degradation/>, and "WaterServices APIs will be
decommissioned early 2027" (posted 2026-01-07, updated 2026-10-17),
<https://waterdata.usgs.gov/blog/api-waterservices-decom/>.

| Date | Change to `waterservices.usgs.gov` |
|---|---|
| 2026-11-16 | 2 second delay added to every request |
| 2027-01-11 | delay raised to 10 seconds |
| 2027-01-25 | delay raised to 30 seconds |
| 2027-01-27 | all requests redirected to the blog post for about 24 hours; service restored 2027-01-28 |
| 2027-02-01 | delay raised to 60 seconds |
| 2027-02-15 | all requests redirected for about 48 hours; service restored 2027-02-17 |
| 2027-02-22 | decommissioned; requests redirected to the earlier blog post |

USGS states the timing and length of delays may be adjusted, the redirects will not start earlier than
listed, and it does not expect to move the 2027-02-22 date. Legacy NWISWeb pages are being retired in the
same campaign (November 2026 to February 2027). Data for parameter codes released after June 2026 is
not published on WaterServices at all (search summary of the blog; **unverified** on the page itself).

## Requests the app needs

All examples use `f=json` for clarity; it is optional. Quote URLs in a shell (`?`, `&`).

| Need | Request |
|---|---|
| Find sites in a box | `GET /collections/monitoring-locations/items?bbox=<west>,<south>,<east>,<north>&site_type_code=ST&limit=50&properties=id,monitoring_location_name,...` |
| One site | `GET /collections/monitoring-locations/items/USGS-01375000` |
| What a site measures | `GET /collections/time-series-metadata/items?monitoring_location_id=USGS-01375000&limit=100&skipGeometry=true` |
| Newest value of every series | `GET /collections/latest-continuous/items?monitoring_location_id=USGS-01375000,USGS-01323500&parameter_code=00060,00065,62614&skipGeometry=true` |
| 7-day series | `GET /collections/continuous/items?monitoring_location_id=USGS-01375000&parameter_code=00060&datetime=2026-11-01T00:00:00Z/2026-11-08T00:00:00Z&skipGeometry=true&sortby=time&limit=2000&properties=time_series_id,time,value,approval_status,qualifier` |
| Relative window | `datetime=P7D` (ISO 8601 duration counted back from now) is accepted by the documentation; **unverified** here (fixtures use fixed windows) |
| One year of daily values | `GET /collections/daily/items?monitoring_location_id=USGS-01375000&parameter_code=00060&statistic_id=00003&datetime=2025-11-08/2026-11-07&skipGeometry=true&sortby=time&limit=1000` |
| Percentile for the date | `GET https://api.waterdata.usgs.gov/statistics/v0/observationNormals?monitoring_location_id=USGS-01375000&parameter_code=00060&normal_type=DOY&computation_type=percentile&start_date=11-08&end_date=11-08` |
| Parameter definitions | `GET /collections/parameter-codes/items?id=00060,00065,...&skipGeometry=true` |

Behaviour of the query parameters, all observed:

- `bbox=west,south,east,north` (longitude first) works on `monitoring-locations` and
  `time-series-metadata`; it also works on `daily`.
- `monitoring_location_id` always carries the agency prefix: `USGS-01375000`.
- Comma-separated lists work for `monitoring_location_id`, `parameter_code` (on `latest-continuous`) and
  `id` (on `parameter-codes`). A comma list for `site_type_code` (`ST,ES`) matched nothing on
  `monitoring-locations`, so send one site type per request.
- `properties=a,b,c` limits the returned fields; an unknown name gives HTTP 400
  `"unknown properties specified"`. `skipGeometry=true` returns `"geometry": null` and shrinks the body.
- `datetime` accepts a timestamp, a `start/end` interval (both ends inclusive: a 7-day window returned
  673 points), half-open `start/..`, and durations. For `daily` a plain date interval works.
- `sortby=time` sorts ascending (`-time` descending; **unverified**). Without it the order is not
  defined (daily values came back unsorted) and the versioning policy says order is not guaranteed.
  Always sort, or sort on the client.
- Filtering on the `qualifier` property returned HTTP 400 `InvalidQuery`; filter on the client.
- Missing values are `null`, not a sentinel.

## Paging

- `limit` defaults to 10 and is at most 50000 (`limit=100000` returns HTTP 400
  `{"code":"InvalidParameterValue","type":"InvalidParameterValue","description":"Limit of 50000 exceeded"}`,
  fixture `usgs-error-limit-exceeded.json`).
- `numberReturned` is always present. `numberMatched` was **absent in every response** recorded,
  although the documentation examples show it. Do not rely on it.
- More results are signalled by a link with `rel: "next"`. All collections tested (`continuous`,
  `daily`, `monitoring-locations`, `time-series-metadata`) produced an opaque `cursor=` parameter
  (not `offset`, which the documentation example still shows). Follow the `href` as given; the versioning
  policy lists "changing how `next` links are generated" as a non-breaking change. No `next` link means
  the last page. Fixtures `usgs-continuous-croton-...-paging-limit3.json` and
  `usgs-daily-croton-...-paging-limit3.json` hold a first page with the link.
- `continuous` accepts at most three years per query (migration guide,
  <https://api.waterdata.usgs.gov/docs/ogcapi/migration/>). Without `datetime` it returned data from
  the previous year onward.
- A 7-day, 15-minute series is 673 points; with `properties` and `skipGeometry` a response is about
  280 KB uncompressed. One request with `limit=2000` suffices.

## Authentication and limits

- Anonymous requests worked. About 110 requests to the API hosts in one hour, none rate-limited.
- A key comes from <https://api.waterdata.usgs.gov/signup/> (free, sent by email). It is passed as the
  query parameter `api_key=<key>` or the header `X-Api-Key: <key>`
  (<https://api.waterdata.usgs.gov/docs/ogcapi/keys/>). Keys are managed by api.data.gov.
- The key documentation says each response carries `X-RateLimit-Limit` and `X-RateLimit-Remaining`
  (its example shows `1000` and `998`) and that exceeding the limit returns HTTP 429. **In this
  recording no `X-RateLimit-*` header appeared on any anonymous response** (landing page, collections,
  data endpoints; all were CloudFront `Miss` responses with `x-api-umbrella-request-id`). The anonymous
  hourly limit is therefore **unverified**: the migration guide only says "more than a few queries per
  hour" need a key. A 429 response was not provoked, so its body and any `Retry-After` header are
  **unverified**. The client should treat 429 as retry-later and read the headers when present.
- Response headers seen: `api-version`, `x-build-version`, `cache-control` (`monitoring-locations` and `daily`
  responses `public, max-age=3600`; `latest-continuous` and `continuous` `no-cache, no-store`),
  `access-control-allow-origin: *`, `content-crs: <http://www.opengis.net/def/crs/OGC/1.3/CRS84>`.
- Efficiency advice from USGS: use `properties`, narrow time ranges, split large queries
  (<https://api.waterdata.usgs.gov/docs/ogcapi/efficiency/>).

## Terms and credit

USGS-authored data are in the U.S. public domain
(<https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits>). USGS asks for
credit such as "U.S. Geological Survey" when its information is used; no further attribution wording is
required. Data with `approval_status` `Provisional` are covered by the provisional data statement
(<https://waterdata.usgs.gov/provisional-data-statement/>): subject to revision, not to be relied on
for safety decisions. A display of live values should say "provisional" for those values. The USGS
identifier (logo) is trademarked and needs separate permission; do not use it. Contact for questions:
`wdfn@usgs.gov`.

## Fields

### monitoring-locations (`collections/monitoring-locations/items`)

Feature `id` and `properties.id` are the monitoring location id (`USGS-01375000`); geometry is a
`Point` `[longitude, latitude]` in WGS84 (CRS84).

| Field | Type | Example | Notes |
|---|---|---|---|
| `id` | string | `USGS-01375000` | agency code + `-` + number |
| `monitoring_location_number` | string | `01375000` | 8 to 15 digits, leading zeros kept |
| `monitoring_location_name` | string | `CROTON R AT NEW CROTON DAM NR CROTON-ON-HUDSON NY` | mixed case or capitals, as entered |
| `site_type_code`, `site_type` | string | `ST`, `Stream` | seen: `ST` stream, `ES` estuary, `LK` lake or reservoir, `ST-TS` tidal stream, `GW` well |
| `state_code`, `state_name`, `county_code`, `county_name` | string | `36`, `New York`, `119`, `Westchester County` | |
| `hydrologic_unit_code` | string | `020301010307` | 12 digits; supports prefix matching |
| `drainage_area` | number or null | `378.0` | square miles (unit not stated in the response; **unverified**) |
| `altitude`, `vertical_datum` | number, string | `43.12`, `NAVD88` | feet (**unverified**); `vertical_datum` is `null` for many lakes |
| `time_zone_abbreviation`, `uses_daylight_savings` | string | `EST`, `N` | `N` on both New York sites tested, even though the area observes daylight time; this describes the site's logging convention, see "Time" |
| `agency_code`, `agency_name`, `district_code`, `country_code`, `basin_code`, `altitude_*`, `horizontal_*`, `original_horizontal_datum*`, `contributing_drainage_area`, well and aquifer fields, `revision_*` | | | present in the full record; not needed |

### time-series-metadata (`collections/time-series-metadata/items`)

| Field | Type | Example | Notes |
|---|---|---|---|
| `id` | string | `e32aab5ccc774c11b172102b97abc0d8` | 32 hex characters; equals `time_series_id` in the value collections |
| `monitoring_location_id` | string | `USGS-01375000` | |
| `parameter_code`, `parameter_name`, `parameter_description` | string | `00060`, `Discharge` | |
| `unit_of_measure` | string | `ft^3/s` | see units table |
| `statistic_id` | string or null | `00011` | `00011` instantaneous point, `00003` mean, `00001` max, `00002` min; also `00021` to `00024` (tidal high, higher-low, lower-high, low) and `30800` (selected value); `null` on event series |
| `computation_identifier` | string | `Instantaneous`, `Mean`, `Max At Event Time`, `TidalHigh`, `SelectedValue` | |
| `computation_period_identifier` | string | `Points`, `Daily`, `Water Year` | |
| `sublocation_identifier` | string or null | `Unfiltered` | distinguishes parallel series for the same parameter |
| `primary` | string or null | `Primary` | `null` marks a non-primary, provisional-only series that USGS keeps for 120 days |
| `begin`, `end` | string or null | `2026-11-08T05:45:00+00:00` | UTC with offset; `end` is refreshed at most daily and may lag the data |
| `last_modified` | string | `2026-11-08T05:49:31.766620` | no offset in this collection (UTC presumed, **unverified**) |
| `thresholds` | array | `[]` | on some series, objects with `Name`, `Periods[].ReferenceValue`, e.g. `National Weather Service Flood Stage` 4.12 ft |
| `parent_time_series_id` | string or null | | links a daily series to the instantaneous series it is computed from |
| `data_gap_interval` | string or null | `PT1H12M` | ISO 8601 duration used to detect gaps |
| `web_description`, `statistics_begin` | string or null | `HRECOS` | free text; `statistics_begin` limits the years used for USGS statistics |

### latest-continuous and continuous

`latest-continuous` returns one feature per time series (the most recent value, even if years old).
`continuous` returns all values of the selected series in the window.

| Field | Type | Example | Notes |
|---|---|---|---|
| `time_series_id` | string | `e32aab5c...` | |
| `monitoring_location_id` | string | `USGS-01375000` | |
| `parameter_code` | string | `00060` | |
| `statistic_id` | string | `00011` | |
| `time` | string | `2026-11-08T13:45:00+00:00` | UTC with `+00:00`; 15-minute cadence on the sites tested |
| `value` | string or null | `"12.4"` | **a string** (to keep precision); `null` when missing (qualifier then usually explains why) |
| `unit_of_measure` | string | `ft^3/s` | |
| `approval_status` | string | `Provisional` or `Approved` | |
| `qualifier` | array of strings or null | `["ICE"]` | see below |
| `last_modified` | string | `2026-11-08T13:50:07.916850+00:00` | |
| `method_category` | string or null | `STNRD`, `UNKWN` | |

### daily and latest-daily

Same fields, with `time` a calendar date (`"2026-09-07"`), no `method_category`, and `statistic_id`
needed in the query: the daily mean of discharge is `statistic_id=00003`. Daily series exist for
discharge, gage height, temperature, lake elevation and others; they are derived from the instantaneous
series (see `parent_time_series_id`).

### Qualifiers and approval

`approval_status` is `Provisional` (still subject to revision) or `Approved`. `qualifier` carries the
remaining codes, always as an array, `null` when none. Seen in the recordings: `ESTIMATED`, `ICE`
(both together as `["ESTIMATED","ICE"]`), `EQUIP` (equipment malfunction; the value is often `null`),
`MAINT`, `DISCONTINUED`. The documentation example also shows `SEASONAL`. A description of each code is
"not currently available" from the API (migration guide). Counts on 2026-01-20 for 293 upstate New York
sites: 182 `ESTIMATED`, 8 `ICE`, 3 `["ESTIMATED","ICE"]`, 1 `["EQUIP","ESTIMATED"]`, 99 none. Display
rule for the app: show `ICE` and `EQUIP` as "unreliable", `ESTIMATED` as "estimated".

### parameter-codes

`id` (the code), `parameter_name`, `unit_of_measure`, `parameter_group_code`, `parameter_description`,
`medium`, `statistical_basis`, `time_basis`, `sample_fraction`, `temperature_basis`, `epa_equivalence`.
Note that `unit_of_measure` here (`ft3/s`, `deg C`, `std units`) is spelled differently from the
`unit_of_measure` in the value collections (`ft^3/s`, `degC`, `pH Units`).

## Parameter codes

Confirmed against `parameter-codes` on 2026-11-08 (fixture `usgs-parameter-codes-selected.json`). The
"unit in values" column is what `latest-continuous` returned for a live site.

| Code | Name | Unit in `parameter-codes` | Unit in values | Seen live at |
|---|---|---|---|---|
| `00060` | Discharge | `ft3/s` | `ft^3/s` | Croton 01375000: 12.4 |
| `00065` | Gage height | `ft` | `ft` | Croton 01375000: 1.10 |
| `00010` | Temperature, water | `deg C` | `degC` | Bronx River 01302020: 13.2 |
| `63680` | Turbidity, Form Neph | `FNU` | `_FNU` (leading underscore) | Bronx River 01302020: 2.1 |
| `00300` | Dissolved oxygen | `mg/l` | `mg/l` | Bronx River 01302020: 5.7 |
| `00095` | Specific conductance at 25 C | `uS/cm @25C` | `uS/cm` | Bronx River 01302020: 880 |
| `00400` | pH | `std units` | `pH Units` | Bronx River 01302020: 7.4 |
| `62614` | Lake or reservoir elevation, NGVD 1929 | `ft` | `ft` | Great Sacandaga Lake 01323500: 755.49 |
| `62615` | Lake or reservoir elevation, NAVD 1988 | `ft` | `ft` | Great Sacandaga Lake: 754.61 (0.88 ft below the NGVD29 value) |
| `62616` | Lake or reservoir elevation, NGVD 1929 | **`m`** | not seen | none found in a New York bounding box |
| `00062` | Elevation, reservoir | `ft` | `ft` | sites 01428900, 01534180 and others in the Delaware and Susquehanna basins (datum not stated) |
| `72020` | Elevation above NGVD 1929 | `ft` | not seen | |
| `72036` | Reservoir storage | `Kac-ft` | not seen | |
| `62619` / `62620` | Estuary or ocean elevation, NGVD 1929 / NAVD 1988 | `ft` | `ft` | Hudson below Poughkeepsie 01372058: `62620` 0.80 |
| `72254` | Sensor velocity | `ft/sec` | `ft/s` | Hudson below Poughkeepsie (value `null`, qualifier `EQUIP`) |
| `90860` | Salinity, PSU | `PSU` | no unit string returned | Hudson Piers (ended 2026) |

For lakes, ask for `62614` and `62615` together and prefer the one that matches the site's
`vertical_datum`; a site may report only one. Lake gauges tested: Great Sacandaga Lake at Conklingville
(01323500) has both; Indian Lake near Indian Lake (01314500) has both but stopped reporting on
2025-10-01; Kensico Reservoir (01301900) has no time series at all. Values must be converted by the
core (feet to metres, cfs to m3/s, degC kept); `value` strings parse as decimal.

## Time

- `time` in `latest-continuous` and `continuous` is an RFC 3339 instant in UTC (`+00:00`).
- `time` in `daily` is a date. Check on 2026-11-08 against the recorded 7-day series: Croton 01375000
  daily mean for 2026-11-02 is 45.1 cfs; the mean of the 15-minute values over the UTC-5 day (05:00Z to
  05:00Z) is 45.55, over the UTC-4 day 49.14, over the UTC day 65.31. The same match held for 2026-11-03
  (14.2 against 14.25 / 14.62 / 16.34). A daily value is therefore the mean over the site's local
  **standard-time** day, which is why `time_zone_abbreviation` is `EST` with `uses_daylight_savings`
  `N`. For the sites tested, treat the date as local standard time (UTC-5), also in summer
  (checked for summer on 2026-11-15, next paragraph).
- Summer check, 2026-11-15, Croton 01375000: the daily means for 2026-07-06, 07-07 and 07-08 against the mean
  of the 96 15-minute `00060` values over each candidate day (fixtures
  `usgs-daily-croton-01375000-00060-mean-2026-07-06_2026-07-08.json` and
  `usgs-continuous-croton-01375000-00060-2026-07-05_2026-07-09.json`):

  | Day | daily mean | UTC-5 day (05:00Z to 05:00Z) | UTC-4 day (04:00Z to 04:00Z) | UTC day |
  |---|---|---|---|---|
  | 2026-07-06 | 1110 | 1091.78 | 987.16 | 643.30 |
  | 2026-07-07 | 2550 | 2560.21 | 2597.50 | 2637.50 |
  | 2026-07-08 | 1420 | 1422.71 | 1452.71 | 1588.33 |

  The daily value matches the UTC-5 (standard-time) day on all three days, in July as in November, so a
  summer daily mean is **not** the UTC-4 local civil day. The daily values are integers here (three
  significant figures), so the match is to within about 2 percent; the 07-06 day, with a rising flood, shows the
  difference most (1091.78 against 987.16).
- Do not use the daily date as a UTC date.
- `time-series-metadata.begin/end` are UTC; `last_modified` there has no offset.

## How time series are split

One monitoring location has many time series. A series is identified by `id`, and defined by parameter,
statistic, computation, period and, optionally, sublocation. Examples:

- Croton 01375000 (5 series): gage height instantaneous (`00065`, `00011`), discharge instantaneous
  (`00060`, `00011`), discharge daily mean (`00003`), and two annual-maximum event series (`Max At Event
  Time`, `Water Year`), whose `statistic_id` is `null`.
- Great Sacandaga Lake (4 series): `62614` and `62615`, each as instantaneous and as daily mean.
- Hudson below Poughkeepsie 01372058 (25 series): live ones are `62620` (primary) and `72254` (not
  primary, not approved); the rest ended between 2011 and 2019. A discharge series `00060` with
  `sublocation_identifier` `Unfiltered` has no `begin` or `end` and no values.
- Hudson Piers 25 and 84 and Hudson at Yonkers (01376307) are HRECOS water-quality stations
  (`web_description: HRECOS`): temperature, conductivity, salinity, pH, dissolved oxygen, turbidity. Their
  series ended 2025-04-30 (Pier 84), 2026-04-01 (Pier 25) and 2025-12-01 (Yonkers), so they cannot
  be used for live values.
- Parameters with only an old `time` in `latest-continuous` (the Poughkeepsie site returns 2011, 2019
  values next to a 2026 value) are dead. A parser must compare `time` with `now` and drop series older than
  a limit (a configurable age, for example three hours for 15-minute series).
- When two series share a parameter, choose `primary == "Primary"`, then the one with the newest `end`;
  select by `time_series_id`, not by parameter code, when requesting `continuous`.

## Statistics API (answers the "no statistics endpoint" question)

`https://api.waterdata.usgs.gov/statistics/v0/` (documentation
<https://api.waterdata.usgs.gov/statistics/v0/docs>, OpenAPI
<https://api.waterdata.usgs.gov/statistics/v0/openapi.json>, title "USGS Water Data Statistics API",
version `v0 - BETA`, "under active beta development and subject to change"). The migration guide points
users of the retired `nwis/stats` to it. It is a separate service from the OGC collections (it is not
listed in `/ogcapi/v1/collections`), under the same host and API-key system. No `X-RateLimit-*` header
was seen on it either.

Endpoints: `GET /statistics/v0/observationNormals` (day-of-year and month-of-year normals) and
`GET /statistics/v0/observationIntervals` (per-month, calendar-year, water-year values; very large:
1.5 MB for one site and `page_size=5`, which does not limit it). Parameters of `observationNormals`:
`monitoring_location_id` (repeatable), `parameter_code`, `normal_type` (`DOY` or `MOY`),
`computation_type` (`arithmetic_mean`, `maximum`, `median`, `minimum`, `percentile`; repeatable),
`start_date` and `end_date` (`MM-DD`, inclusive), `approval_status` (only `approved` data is used today),
`site_type_code`, state/county/country filters, `page_size` (default 1000, maximum 10000),
`next_token`, `mime_type`.

Response: GeoJSON `FeatureCollection` with a top-level `next` (null on the last page) and, per site
feature, `properties.data[]` with one entry per parameter and parent series
(`parameter_code`, `unit_of_measure`, `parent_time_series_id`, `parent_statistic_id`,
`parent_statistic_name`) and `values[]`:

| Field | Example | Notes |
|---|---|---|
| `time_of_year` | `11-08` | `MM-DD` for `day_of_year`; `MM` for `month_of_year` |
| `time_of_year_type` | `day_of_year` | |
| `computation` | `percentile` | |
| `values` + `percentiles` | `["0.634","0.966","1.545","15.0","237.0","689.6","1180.0"]` + `["5","10","25","50","75","90","95"]` | percentile entries carry arrays of strings; other computations carry a single `value` string |
| `sample_count` | `93` | number of years in the sample for that day |
| `approval_status` | `approved` | |
| `computation_id` | uuid | |

Example: Croton at New Croton Dam, 8 November, 93 years of daily means: 5th percentile 0.634 cfs, median
15.0, 95th 1180 cfs; the current 12.4 cfs is just under the median. Great Sacandaga Lake on 8 November
(`62614`, 96 years): median 754.97 ft; current 755.49 ft.

So "vs normal for this date" can be answered with one small request per site and parameter (about 2.4 KB
for six days with percentiles) instead of downloading the daily record (the whole 93-year Croton record
via `daily` is about 34,000 values; a 16-year slice was 5,844 values). Caveats: the service is beta and
may change; the percentile list is fixed (5, 10, 25, 50, 75, 90, 95), so the percentile rank of a
value is interpolated between them; only approved data are used; leap day `02-29` has its own row (366 rows).
The `daily` collection remains the fallback: `limit=50000` with `sortby=time` returned 16 years
(2010 to 2025) in one response, and the three-year cap quoted for `continuous` did not apply to `daily`
(a 1990 to 2026 request for the lake returned 13,460 values, 3.6 MB with `properties`; a 365-day request
returned 160 KB).
The fully unfiltered `observationNormals` response for one site and parameter was 424 KB (all computations,
366 days plus 12 months), so always send `normal_type`, `computation_type`, `start_date` and `end_date`.

## Other collections (listing from `/collections`, 2026-11-08)

`latest-continuous`, `continuous`, `latest-daily`, `daily`, `edr/daily`, `latest-field-measurements`,
`field-measurements`, `channel-measurements`, `combined-metadata`, `field-measurements-metadata`,
`time-series-metadata`, `monitoring-locations`, `peaks`, `time-series-revisions`, `time-series-methods`,
and code tables (`agency-codes`, `parameter-codes`, `statistic-codes`, `site-types`, `states`, `counties`,
`hydrologic-unit-codes`, `reliability-codes`, `time-zone-codes`, and others). The llms index
(<https://api.waterdata.usgs.gov/llms.txt>) also lists a real-time flood impact API
(<https://api.waterdata.usgs.gov/rtfi-api/docs>), a water-quality samples API
(<https://api.waterdata.usgs.gov/samples-data/docs>) and a STAC API; none was tested.

## Test places

| Place | Site | Finding on 2026-11-08 |
|---|---|---|
| Croton River | `USGS-01375000` CROTON R AT NEW CROTON DAM NR CROTON-ON-HUDSON NY, `ST`, 378 mi2, NAVD88, 41.2250, -73.8589 | live `00060` and `00065`, 15-minute; a dam-tailwater gauge: 163 cfs on 2026-11-01 fell to 12 cfs by 2026-11-08 |
| Lower Hudson | `USGS-01372058` HUDSON RIVER BELOW POUGHKEEPSIE NY, `ES` | only live tidal elevation `62620` (NAVD88 ft) and a velocity series with `null` values; no live temperature or discharge |
| Lower Hudson (stale) | `USGS-01376515` Pier 84, `USGS-01376520` Pier 25, `USGS-01376307` Yonkers | HRECOS water-quality series ended 2025 or April 2026; no site with live water temperature was found on the tidal Hudson south of Poughkeepsie; **unverified** whether another site (for example Hudson at Albany or Troy, upstream of the lower river) has live temperature |
| Inland lake | `USGS-01323500` GREAT SACANDAGA LAKE AT CONKLINGVILLE NY, `LK`, NAVD88 | live `62614` and `62615`, 15-minute; level 755.49 ft NGVD29 |
| Multi-parameter | `USGS-01302020` BRONX RIVER AT NY BOTANICAL GARDEN AT BRONX NY, `ST` | live `00010`, `00060`, `00065`, `00095`, `00300`, `00400`, `63680` |

CO-OPS station 8518750 and NDBC 44025 are outside this note.

## Items not verified

- Anonymous hourly limit, the `X-RateLimit-*` headers (absent from every response here) and the HTTP 429
  body.
- `datetime` durations such as `P7D`, `sortby=-time`, and the `X-Api-Key` header and `api_key`
  parameter (not used by design).
- Units of `drainage_area` and `altitude`, and the full list of `qualifier` codes.
- Live water temperature on the lower Hudson.

## Percentile classes (addendum, 2026-11-15)

USGS compares the current flow with the flows recorded at the gauge on the same day of the year and sorts the
percentile into classes. WaterWatch (`https://waterwatch.usgs.gov`) defined seven classes for streamflow;
its documentation page `https://waterwatch.usgs.gov/htmls/ptile_table_a.html` (read from the Internet Archive
copy of 2017-05-01, `http://web.archive.org/web/20170501151620/https://waterwatch.usgs.gov/htmls/ptile_table_a.html`,
because the live host now answers HTTP 301 to `https://waterdata.usgs.gov/blog/wdfn-stats-delivery/` and that
blog page and the old help pages return HTTP 403 to `curl` and the fetch tool used here) says:

```text
1 = Lowest ever at a streamgage
2 = < 10th percentile
3 = 10th to 24th percentile
4 = 25th to 75th percentile
5 = 76th to 90th percentile
6 = > 90th percentile
7 = Highest ever at a streamgage
```

An announcement on the WaterWatch pages (archive of 2026-02-11) says WaterWatch and WaterQualityWatch "will be
retired around February 24, 2026" (the notice points to alternative tools).


The USGS `hyswap` Python package (`https://github.com/DOI-USGS/hyswap`, `hyswap/utils.py`, function
`retrieve_schema`, read 2026-11-15) holds the schema "WDFN_stats", described as categorizing "flow similar to the
USGS National Water Dashboard", with the same bounds:

| Class (display name) | Percentile | `hyswap` range |
|---|---|---|
| Much below normal | below 10 | `[0, 10)` |
| Below normal | 10 to 24 | `[10, 25)` |
| Normal | 25 to 75 | `[25, 76)` |
| Above normal | 76 to 90 | `[76, 90)` |
| Much above normal | above 90 | `[90, 100)` |
| All-time low for this day | lowest value of the record | percentile 0 |
| All-time high for this day | highest value of the record | percentile 100 |

The `hyswap` ranges are closed on the left and open on the right (`pd.cut(..., right=False)`), so a percentile of
exactly 90 falls in "Much above normal" there while the older page says "> 90th" and "76th to 90th". The
other `hyswap` schema "WaterWatch" uses `[0, 10, 25, 75, 90, 100]` with labels including "Low" and "High". The
published bounds therefore agree on 10 / 25 / 75 or 76 / 90 as the class edges; the edge case at exactly the 75th
and 90th percentile is not settled by the sources, and percentiles from the Statistics API
(section "Statistics API") are only available at 5, 10, 25, 50, 75, 90 and 95, so a rank computed from them is
interpolated. The two lowest and highest classes mean a flow below the minimum or above the maximum of the
gauge's record for that day, which the Statistics API reports as the `minimum` and `maximum` computations.

**Unverified:** the live National Water Dashboard legend (its pages were not reachable here) and the exact
treatment of percentiles on a class edge. The 2022 national annual-streamflow fact sheet (`https://pubs.usgs.gov/fs/2022/3072/fs20223072.pdf`)
uses the same five names plus "Lowest" and "Highest" for annual ranks, not for daily percentiles.
