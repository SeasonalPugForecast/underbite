# ECCC hydrometric (Water Survey of Canada, OGC API at api.weather.gc.ca)

Verified on 2026-11-17 (UTC) with `curl` against `https://api.weather.gc.ca/collections`, the collections' own queryables and schema documents, the MSC Open Data pages (`eccc-msc.github.io/open-data`) and the Water Survey of Canada pages on `wateroffice.ec.gc.ca`. An earlier summary is in `docs/providers/tier2.md`. Fixtures: `tests/fixtures/raw/eccc/` (index in its `README.md`). Verdict: **usable**.

## Service

| Item | Value |
|---|---|
| Base URL | `https://api.weather.gc.ca/collections/{collection}` (OGC API - Features, `X-Powered-By: pygeoapi 0.20.0`) |
| Collections used | `hydrometric-stations`, `hydrometric-realtime`, `hydrometric-daily-mean` (others listed on the service root: `hydrometric-monthly-mean`, `hydrometric-annual-statistics`, `hydrometric-annual-peaks`) |
| Authentication | none; a generic `User-Agent` was sent, none required |
| Format | `f=json` returns GeoJSON (`application/json` body for `/items`; the queryables and schema documents are `application/schema+json`); also `jsonld` and `html` |
| Common parameters | `f`, `lang` (`en-CA`, `fr-CA`), `limit`, `offset`, `bbox` (4 or 6 numbers, `minlon,minlat,maxlon,maxlat`), `datetime`, `sortby` (`-FIELD` descends), `resulttype=hits`, `skipGeometry`, `properties` (comma list of fields) and any queryable as an equality filter |
| Rate limits | none stated; no rate-limit headers on any response (about 25 requests paced at 2 seconds). HTTP 429 behaviour **unverified**. |
| Response headers | `access-control-allow-origin: *`, `content-language: en-CA`, gzip when requested |

Paging: the response carries `numberMatched` (when `resulttype=hits` it has no features and returns only the count), `numberReturned`, and `links` with `rel: next` holding the next `offset`. Verified: `limit=10` of 12 stations gives a `next` link with `offset=10`; `offset=3&limit=3` returns the next three. The default page size and the maximum `limit` were not established; `limit=1100` returned 1,096 rows, `limit=100000` was accepted with `resulttype=hits`. A non-integer limit answers `400 {"code":"InvalidParameterValue","type":"InvalidParameterValue","description":"limit value should be an integer"}`.

## Units, datum and codes (answers the earlier open question)

The collections' queryables and schema documents list names and JSON types only (no units, titles or descriptions: `LEVEL` is `number`/`float`), so the units come from ECCC's own documentation:

- **`LEVEL`: metres.** MSC Datamart page for the same real-time data (`https://eccc-msc.github.io/open-data/msc-data/obs_hydrometric/readme_hydrometric-datamart_en/`): "Water Level (m): Water level in meters".
- **`DISCHARGE`: cubic metres per second.** Same page: "Discharge (cms): Discharge in cubic meters per second". Bow River at Calgary reads 0.865 m and 52.6 m3/s on 2026-11-17, consistent with a winter flow.
- **Datum of `LEVEL`: the station's own gauge datum, not a geodetic height, in most cases.** The stations collection has a text field `VERTICAL_DATUM` ("the name of the datum that water levels are referred to", per `https://wateroffice.ec.gc.ca/station_metadata/station_list_help_e.html`). For 05BH004 and most Calgary stations it is `ASSUMED DATUM`, which the Water Survey FAQ (`https://wateroffice.ec.gc.ca/contactus/faq_e.html`) describes as an arbitrary local coordinate system: the reading "does not reflect the depth of the water, nor its height above sea level". A few stations carry a real datum (Glenmore Reservoir 05BJ008: `GEODETIC SURVEY OF CANADA DATUM`); the field is an empty string for some discontinued stations. Show `LEVEL` as a gauge reading, compare it only with the same station's history, and use `VERTICAL_DATUM` to decide whether it may be shown as an elevation. Other values of `VERTICAL_DATUM` were not enumerated (**unverified**).
- The Datamart CSV page lists `QA/QC` for the water level as 1 preliminary, 2 reviewed, 3 checked, 4 approved. The API's `/items` documents have no QA field.
- `LEVEL_SYMBOL_EN/FR` and `DISCHARGE_SYMBOL_EN/FR` are the Water Survey symbols, spelled out in the API: observed `Ice Conditions` / `Conditions à glace` (the Bow River had 337 ice-affected discharge days out of 1,096 in 2023 to 2025; the level symbol was always `null`). The Water Survey FAQ defines the symbol letters: A partial day (daily mean estimated despite gaps over 120 minutes), B ice conditions (flow estimated), D dry (level only), E estimate (no measured data, estimated indirectly), R revised (historical discharge revised after 1989-01-01). The mapping of the full-text strings in the API to those letters beyond `Ice Conditions` = B was not observed (**unverified**). In the Datamart CSV the symbol columns are "future use".
- Units in `hydrometric-stations`: `DRAINAGE_AREA_GROSS` and `DRAINAGE_AREA_EFFECT` are km2 (station list help page).

## Requests

```text
GET /collections/hydrometric-stations/items?bbox=-114.2,50.9,-113.9,51.2&f=json&limit=10&sortby=STATION_NUMBER
GET /collections/hydrometric-stations/items?STATION_NUMBER=05BH004&f=json
GET /collections/hydrometric-realtime/items?STATION_NUMBER=05BH004&f=json&limit=12&sortby=-DATETIME
GET /collections/hydrometric-daily-mean/items?STATION_NUMBER=05BH004&f=json&datetime=2023-01-01/2025-12-31&sortby=DATE&limit=1100&skipGeometry=true&properties=DATE,LEVEL,DISCHARGE,DISCHARGE_SYMBOL_EN,LEVEL_SYMBOL_EN
GET /collections/hydrometric-realtime/items?STATION_NUMBER=05BH004&f=json&limit=1&resulttype=hits
```

### `hydrometric-stations`

All stations: `numberMatched` 8,057 (2026-11-17); with `REAL_TIME=1&STATUS_EN=Active` 2,627 matched. A bounding box of about 0.3 by 0.3 degrees around Calgary matched 12 stations (about 2 KB per station), including discontinued ones: filter `STATUS_EN=Active` and `REAL_TIME=1` for live data.

| Property | Type | Meaning |
|---|---|---|
| `STATION_NUMBER` / `IDENTIFIER` | string | station id (`05BH004`: basin digits, sub-basin letters, station digits) |
| `STATION_NAME` | string | upper case; English or French according to the location |
| `PROV_TERR_STATE_LOC` | string | `AB`, ... (also US state codes for border stations) |
| `STATUS_EN` / `STATUS_FR` | string | `Active`, `Discontinued` / `En service`, `Fermée` |
| `CONTRIBUTOR_EN` / `_FR` | string | operating agency; empty string when unknown |
| `VERTICAL_DATUM` | string | see above; empty string when unknown |
| `REAL_TIME` | integer (byte) | 1 if the station reports in real time |
| `RHBN` | integer (byte) | Reference Hydrometric Basin Network flag |
| `DRAINAGE_AREA_GROSS`, `DRAINAGE_AREA_EFFECT` | number | km2 |
| geometry | Point | `[lon, lat]`, float32 widened (`-114.05146026611328`) |
| `links` | array | related daily and monthly mean items, and the Water Office page |

### `hydrometric-realtime`

Last 30 days only (collection description: "over 2100 hydrometric stations across Canada (last 30 days)"); Bow River at Calgary: 8,666 rows matched, i.e. 288 per day at 5-minute steps. The newest row at the check was 2026-11-17T09:05:00Z, read at 10:16Z. Water Office documents satellite transmission every 1 to 3 hours, land-line polling hourly to daily, and delivery of unit values "within 6 hours of recording" (FAQ). Always sort `-DATETIME` and take the first row; consecutive rows repeat the last value between transmissions (the first three recorded rows share 0.865 m and 52.6 m3/s).

| Property | Type | Meaning |
|---|---|---|
| `IDENTIFIER`, `id` | string | `<station>.<DATETIME>` |
| `STATION_NUMBER`, `STATION_NAME`, `PROV_TERR_STATE_LOC` | string | as above |
| `DATETIME` | string | UTC, `2026-11-17T09:05:00Z` |
| `DATETIME_LST` | string | local standard time with offset, `2026-11-17T02:05:00-07:00` (never daylight time) |
| `LEVEL` | number | metres, gauge datum (see above); `null` when the station does not measure it |
| `DISCHARGE` | number | m3/s; `null` when not measured (**unverified**: not seen for 05BH004; null symbol fields were seen) |
| `LEVEL_SYMBOL_EN/FR`, `DISCHARGE_SYMBOL_EN/FR` | string or null | qualifiers, `null` when none |
| geometry | Point | station position |

### `hydrometric-daily-mean` (for "compared with normal")

The collection is the HYDAT archive (temporal extent from 1850-01-01). Bow River at Calgary has 41,305 daily rows. Its newest row on 2026-11-17 is **2025-12-31**, so the archive lags by about a year (HYDAT is published once a year): "normal" for a day of the year must be computed from the history, and the current year needs `hydrometric-realtime` (30 days) only. One request for three years (`datetime=2023-01-01/2025-12-31`, `skipGeometry=true`, `properties=...`) returned 1,096 rows in about 222 KB (fixture; about 18 KB on the wire with gzip). `datetime` takes `start/end` dates; there is no day-of-year filter, so select the years client-side.

| Property | Type | Meaning |
|---|---|---|
| `DATE` | string | `YYYY-MM-DD`, station local day |
| `LEVEL` | number | daily mean level, metres, gauge datum; float32 widened (`0.7879999876022339`: round to 3 decimals) |
| `DISCHARGE` | number | daily mean, m3/s, same widening (`38.400001525878906`) |
| `LEVEL_SYMBOL_*`, `DISCHARGE_SYMBOL_*` | string or null | e.g. `Ice Conditions` |

Missing days are absent rows (not checked for 05BH004; **unverified** for gaps).

## Licence and attribution

Environment and Climate Change Canada Data Services End-use Licence, version 2.1.1 (August 2026), `https://eccc-msc.github.io/open-data/licence/readme_en/` (read 2026-11-17): worldwide, royalty-free, perpetual, non-exclusive, commercial use allowed; copy, modify, publish, translate, adapt and distribute for any lawful purpose. Conditions: acknowledge the source, including any attribution the provider specifies; attribute outside originators; where a provider gives none or several attributions are impractical, use "Contains information licenced under the Data Server End-use Licence of Environment and Climate Change Canada." and link the licence where possible. Suggested credit line: "Data Source: Environment and Climate Change Canada" (the text the licence gives as an example), plus the contributor from `CONTRIBUTOR_EN` where present. The licence does not grant use of personal information, third-party rights, or the logos and symbols of the department, and states no endorsement. The collections carry no per-collection attribution field. Water levels are not for navigation and carry no warranty.

## Known issues

- The units are not machine-readable in the queryables or schema; they are fixed here from ECCC's pages (`LEVEL` m, `DISCHARGE` m3/s).
- Float32 widening in the daily-mean and station documents: round values for display.
- No outage or deprecation notice was found on the pages read (2026-11-17). The MSC Open Data web-services page `.../msc-geomet/web-services_en/` returned 404 on that date.
