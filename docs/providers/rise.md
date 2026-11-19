# USBR RISE (reservoir elevation, storage and release)

Verified on **2026-11-17** (UTC) with `curl` and `python3` against `https://data.usbr.gov/rise/api`. This note extends the summary in `tier2.md`. Fixtures: `tests/fixtures/raw/rise/` (index in its `README.md`). Items that could not be checked are marked **unverified**.

## Service

| Item | Value |
|---|---|
| Base URL | `https://data.usbr.gov/rise/api` (API Platform: JSON:API with Hydra; schema in `/rise/api/docs.jsonld`, 90 KB) |
| Version | none in the URL. The release notes (RISE 2.15.3, 2025-08-13) say the API code was converted from annotations to attributes "as a precursor to the planned upgrade of the RISE API platform": expect a future change of the API platform. |
| Format | `Content-Type: application/vnd.api+json`; errors `application/problem+json`. Sending `Accept: application/vnd.api+json` or none gave the same JSON. |
| Authentication | none |
| Required headers | none. A `User-Agent` was sent. Responses set two session cookies (F5); they are not needed. |
| Rate limits | none documented, none reported in headers. **Unverified** (the documentation page `https://data.usbr.gov/rise-api` has no readable content for a script; the help site was not reachable). About 60 requests were made at one per second without a refusal. |
| Terms | each catalogue item carries a `disclaimer`: "Data are provisional and subject to revision unless otherwise noted. Data users are cautioned to consider carefully the provisional nature of the information before using it." The Reclamation disclaimer page (`https://www.usbr.gov/main/disclaimer.html`) defers to the Department of the Interior disclaimer policy and states no copyright, licence or credit requirement. Credit "U.S. Bureau of Reclamation (RISE)". |
| Time | `dateTime` is ISO 8601 with `+00:00`. Daily values are stamped at **local midnight expressed in UTC**: `07:00Z` in November (MST), `06:00Z` in June to October (MDT) for Mountain-time locations (observed 2026-10-01 06:00Z, 2026-11-15 07:00Z). The location's `timezone` and `timezoneOffset` give the zone. Compute the local day from the location's zone, not from the UTC date. |

## Requests the app needs

```text
GET /rise/api/location?locationTypeId=3&itemsPerPage=100&page=1        reservoirs (286 locations, 3 pages)
GET /rise/api/location/1535                                            one location (coordinates, datum, time zone)
GET /rise/api/catalog-record/2300                                      lists the catalog items of a location
GET /rise/api/catalog-item?id[]=341&id[]=4314&itemsPerPage=100         item details (parameter, unit, timestep, end date)
GET /rise/api/result?itemId=341&dateTime[after]=2026-11-10&itemsPerPage=10&order[dateTime]=DESC
```

Brackets must be percent-encoded for a strict client (`%5B`, `%5D`).

- **Filters that work**: on `location`, `locationTypeId` (1 canal 44, 2 diversion 4, **3 lake/reservoir 286**, 4 power plant 53, 8 pumping plant 4, 9 building 5, 10 well 4, 11 dam 1); on `catalog-item`, `id[]`; on `result`, `itemId`, `locationId`, `modelRunId` (**one is required**), `dateTime[after]`, `dateTime[before]`, `order[dateTime]`.
- **Filters that are ignored** (the full set is returned, `totalItems` unchanged): `locationName`, `parameterId`, `locationId` and `catalogRecord` on `catalog-item`, `after=`. A result query with an ignored filter and no real one returned `totalItems` 29,029 for one item after 51 seconds, so verify `meta.totalItems` is small.
- **`result` without a filter**: HTTP 400 in 0.3 s (fixture `rise-result-no-filter-error-2026-11-17.json`), detail "Request must include one of the following: `itemId`, `modelRunId`, or `locationId`". `result?locationId=1535&itemsPerPage=1` answered in 1.2 s (all parameters of a location).
- **Paging**: `meta` is `{totalItems, itemsPerPage, currentPage}`; `page=N`. `itemsPerPage` is capped at **100** for `location` and `catalog-item` (a request for 300 returned 100) but **1000** was honoured on `result` (1,000 rows, 383 KB, 1.4 s). Default 25. The response has no `Link` paging headers; compute pages from `totalItems`.
- Date range: `dateTime[after]=2024-01-01` returned 1,178 daily rows in one query (1,000 per page), so a range of years is accepted. No maximum range was found.
- Latency: normally 0.2 to 1.4 s.

## Result fields (fixture `rise-result-341-flaming-gorge-elevation-2026-11-17.json`)

| Field | Meaning |
|---|---|
| `_id`, `id` | result id |
| `itemId`, `locationId`, `parameterId`, `sourceCode` | joins to the catalogue (`sourceCode` `uchdb2` at Flaming Gorge) |
| `dateTime` | ISO 8601 UTC; see "Time" above |
| `result` | number in the catalogue item's `parameterUnit` (`ft` elevation, `af` storage, `cfs` release); missing value markers were **not seen** in the rows fetched (**unverified**: whether `null` appears) |
| `status` | `null` in every row seen |
| `resultAttributes` | `{"timeStep":"day","resultType":"observed"}` |
| `modelRunId`, `modelRunMemberId` | `null` for observations |
| `lastUpdate`, `createDate`, `updateDate` | server timestamps (UTC) |

Catalogue item (fixture `rise-catalog-items-flaming-gorge-2026-11-17.json`): `itemTitle`, `parameterName` (for example "Lake/Reservoir Elevation"), `parameterUnit`, `parameterTimestep` (`daily`), `parameterTransformation` (`instant`), `temporalStartDate`, `temporalEndDate` (`2099-12-31` marks a placeholder item with no data end), `itemVerticalDatum` (`null` in all items checked), `disclaimer`, `updateFrequency`, `sourceCode`.

Location (fixture `rise-location-1535-flaming-gorge-2026-11-17.json`): `locationName`, `locationCoordinates` (GeoJSON `[lon, lat]` for a point; a polygon for some lakes such as Fort Peck, Truckee Canal, and null for group records), `elevation`, `verticalDatum._id`, `timezone` (`MT`, `PT`, `CT`), `projectNames`, `locationTypeName`, relationship `catalogRecords`.

## Elevation datum

The datum is not stated on the item. It is on the location: `NAVD88`, `NGVD29`, `EGM96`, `USGS 1912` (Lake Mead) or missing (Pathfinder, Canyon Ferry, Lake Powell sub-locations, "Unknown" at Gray Reef). The Colorado River storage projects (Flaming Gorge, Fontenelle, Navajo, Lake Powell) are labelled `EGM96`, which is unusual for reservoir pool levels (**unverified**: whether the label is accurate). Show the datum as stated by RISE and do not convert elevations.

## Curated reservoirs (data checked 2026-11-17)

Item ids, units `ft` (elevation), `af` (storage), `cfs` (release, inflow). "End" is the item's `temporalEndDate` (item metadata; the latest result date for items without a note). Daily timestep for all.

| Location id | Reservoir (state), coordinates | Elevation | Storage | Release | Inflow | End date | Datum on the location |
|---|---|---|---|---|---|---|---|
| 1535 | Flaming Gorge (UT), 40.98, -109.57 | 341 | 337 | 4314 total, 4306 powerplant, 4371 spillway, 4390 bypass | 339 (inflow), 338 (unregulated) | 2026-11-15 | EGM96 |
| 393 | Lake Powell, Glen Canyon Dam (AZ), 37.06, -111.30 | 508 | 509 | 4315 total, 507 powerplant, 4391 spillway | 511 | 2026-11-15 | EGM96 |
| 3514 | Lake Mead, Hoover Dam (NV), 36.02, -114.74 | 6123 | 6124 | 6125 total | none | 2026-11-15 (release 11-16) | USGS 1912 |
| 1536 | Fontenelle (WY), 42.06, -110.10 | 349 | 347 | 4313 total | 4281 | 2026-11-15 | EGM96 |
| 423 | Navajo (NM), 36.86, -107.53 | 612 | 613 | 4316 total | 616 | storage 2026-11-15, release 2026-11-12 | EGM96 |
| 281 | Boysen (WY), 43.40, -108.18 | 196 | 195 | 199 total, 9 spillway | 197 | 2026-11-15 | NGVD29 |
| 433 | Pathfinder (WY), 42.47, -106.85 | 645 | 644 | 647 total | none | release 2026-11-14 | none |
| 295 | Canyon Ferry (MT), 46.65, -111.73 | 14 | 13 | none | none | 2026-11-15 | none |
| 337 | Fresno (MT), 48.61, -109.95 | 355 | 354 | 357 total, 17 spillway | none | 2026-11-15 | NGVD29 |
| 471 | Shasta (CA), 40.72, -122.42 | 10863 | 10864 | 10869 total, 10866 powerplant, 10868 spillway | 10867 | 2026-11-16 | NAVD88 |
| 3203 | Trinity (CA), 40.80, -122.76 | 10853 | 10854 | 10859 total | 10857 | 2026-11-16 | NAVD88 |
| 334 | Folsom (CA), 38.68, -121.18 | 10775 | 10774 | 10772 total | 10773 | 2026-11-16 | NAVD88 |
| 387 | Keswick (CA), 40.60, -122.44 | 10795 | 10794 | 10792 total | 10793 | 2026-11-16 | NAVD88 |
| 3641 | Anderson Ranch (ID), 43.36, -115.45 | 129153 | 129152 | 129160 powerplant (ended 2026-10-06) | none | elevation 2026-11-08 | NGVD29 |
| 3642 | Arrowrock (ID), 43.60, -115.92 | 129218 | 129217 | none | 129219 (computed) | 2026-11-08 | NGVD29 |
| 7192 | Lake Owyhee (OR), 43.64, -117.24 | 132262 | 132261 | 132270 total | none | 2026-11-08 | NAVD88 |
| 7185 | Jackson Lake (WY), 43.86, -110.59 | 131200 | 131199 | none | none | 2026-11-08 | NAVD88 |
| 323 | Elephant Butte (NM), 33.22, -107.18 | 332 | 329 | 4377 total | 330 | release 2026-11-16 | EGM96 |
| 7302 | Lake Meredith (TX), 35.71, -101.55 | 134006 | 134005 | none | none | 2026-11-15 | NAVD88 |

Notes: item ids in the 129000 to 134000 range have a one-week lag (the last value was 2026-11-08 on 2026-11-17). Items ending `2099-12-31` (Lake Sumner 6171 to 6175) are forecast placeholders. Reservoir coverage is western; no Reclamation reservoir is near the New York test places. The entries "Lake Powell At ..." and "Lake Mead At ..." in the reservoir list are sub-locations (marinas, intakes) that list no elevation, storage or release items in the item query above (**unverified**: whether they carry other items).

Release and storage units: some locations publish `af` and `cfs` release as separate items (`4353`/`4314` at Flaming Gorge). Prefer the `cfs` item.

## Findings

- A location lookup by name is not possible (the filter is ignored). Fetch the reservoir location list (three pages, 218 KB at 100 per page) once, bundle the id, name and coordinates, and fetch catalogue items per location with the record-to-item route above when needed.
- The `result` endpoint requires an `itemId`, `locationId` or `modelRunId` now. `tier2.md` recorded a 30 s timeout for this case on 2026-11-08; on 2026-11-17 the server answers HTTP 400.
- The ignored-filter behaviour is silent. Always check `meta.totalItems` against the expected number of rows before reading a page.

## Licence, terms and attribution

Re-checked 2026-11-18: unchanged. `https://www.usbr.gov/main/disclaimer.html` (last updated 8/5/26) still defers to the Department of the Interior disclaimer policy and states no licence or credit requirement; the Interior page `https://www.doi.gov/privacy/doi-disclaimer` answered 404 and was not read (**unverified**). The catalogue-item `disclaimer` text was not fetched again.
