# DFO IWLS (Canadian Hydrographic Service water levels and currents)

Verified on 2026-11-17 (UTC) with `curl` against `https://api-iwls.dfo-mpo.gc.ca/api/v1`, the service's own OpenAPI document and the Canadian Hydrographic Service (CHS) pages on `tides.gc.ca`. An earlier summary is in `docs/providers/tier2.md`. Fixtures: `tests/fixtures/raw/dfo-iwls/` (index in its `README.md`). Verdict: **usable**, with licence conditions (see "Licence and attribution").

## Service

| Item | Value |
|---|---|
| Base URL | `https://api-iwls.dfo-mpo.gc.ca/api/v1` |
| API version | `1.3.1` (`info.version` of the OpenAPI 3.1 document at `https://api-iwls.dfo-mpo.gc.ca/v3/api-docs`; Swagger UI at `/swagger-ui/index.html`) |
| Authentication | none; no key or account. A generic `User-Agent` was sent (`underbite-scout/0.1`); none was required. |
| Format | JSON (`application/json`); errors are `application/json` bodies (see "Errors") |
| Time | ISO 8601 UTC with `Z` in every request and response; `from`/`to` must be in that form (`2026-11-18T00:00:00Z`) |
| Caching | `cache-control: no-cache, no-store, max-age=0, must-revalidate`; a `cookiesession1` cookie is set (load-balancer affinity; not needed) |
| Coverage | Canada only. No New York station. Examples use Vancouver 07735, Toronto 13320 and Second Narrows 07745. |

Rate limits and ranges come from the CHS web-services page (`https://www.tides.gc.ca/en/web-services-offered-canadian-hydrographic-service`, read 2026-11-17): up to 3 requests per second and 30 requests per minute per client IP; HTTP 429 "likely" means one of these limits was exceeded. Data per request: 1 minute data one week, 3 minute data 3 weeks, all lower resolutions one month. No rate-limit response headers were present on any response (about 40 requests, paced at 4 seconds). The 429 response body was **not** recorded: provoking it needs more than 30 requests per minute, which the service's policy asks users to avoid (**unverified**; treat the status code alone as the signal).

## Vertical datum of `value` (answers the earlier open question)

Water-level values (`wlo`, `wlp`, `wlp-hilo`, `wlf`) are **metres above chart datum (CD)** at the station. Evidence, all from CHS material:

- The CHS glossary (`https://www.tides.gc.ca/en/definitions-content-tides-and-currents`) defines chart datum as the plane to which charted depths are related and, in non-tidal waters, "also the vertical datum for elevations and clearances". The SPINE forecast service description on the web-services page says its water level is "expressed relative to chart datum".
- `/stations/{id}/metadata` for Vancouver 07735 lists reference heights in the same frame: `LAT` (Lowest Astronomical Tide) -0.10, `LLWLT` 0.06, `MLLW` 1.17, `MWL` 3.06, `MHHW` 4.47, `HHWLT` 5.01, `HAT` 5.08 (all metres, `valuePrecision` 2). A chart datum close to LAT is what these numbers show. The observed `wlo` and predicted `wlp` values for the same station (2.0 to 3.4 m on 2026-11-17/18) sit inside that range.
- The same `metadata` object carries `datums[]`, the offset of other vertical datums relative to CD: 07735 has `CGVD2013` -2.88 and `CGVD28` -3.0 (metres, `offsetPrecision` 2). Toronto 13320 (Lake Ontario, `waterBody: LAKE`, `isTidal: false`) has `IGLD85` 74.2: chart datum at Toronto is 74.20 m on IGLD 1985, and its `wlo` values (0.445 and 0.456 m at 08:00 and 09:00 UTC on 2026-11-17) are heights above that chart datum, **not** IGLD 85 elevations. To get an IGLD 85 elevation add `datums[IGLD85].offset` to the value. The sign convention of the `CGVD*` offsets (CD elevation in that datum, or the reverse) is **unverified**: the metadata does not say, and the CHS pages read do not define it. Use only the `IGLD85` rule above, which is consistent with the Great Lakes chart datum for Toronto.
- Great Lakes stations (codes 10000 to 16010) also have `calculate-daily-means-igld85`, which the CHS glossary defines as the mean of the 24 hourly heights "referenced to IGLD (1985)", for the Great Lakes, the St Lawrence River down to Lac Saint-Pierre, and the Ottawa and Richelieu rivers.
- Stations with `datums: []` (Second Narrows 07745) have no conversion; the value is above that station's chart datum all the same.

Not verified: whether every river station (non-tidal, non-lake) uses a chart datum rather than a geodetic datum; only Vancouver and Toronto were inspected (2026-11-17). Read `datums[]` per station and show the datum name next to the height.

## Station list

`GET /stations` with no filter returns every station, 1,576 records, 918,660 bytes, no paging (fixture `stations-all-2026-11-17.json`).

Filters (OpenAPI): `code` (unique; overrides every other parameter), `chs-region-code` (`ATL`, `QUE`, `PAC`, `CNA`), `time-series-code`, `dateStart`, `dateEnd` (stations having data in that interval). There is **no bounding-box or radius filter**: select by region or fetch the whole list and filter locally. `chs-region-code=PAC` is about 245 KB.

Record fields: `id` (24 hex characters, used in every other path), `code` (5-digit string, `"07735"`), `officialName`, `alternativeName` (optional), `latitude`, `longitude`, `chsRegionCode`, `provinceCode`, `type`, `operating`, `timeSeries[]`.

Counts on 2026-11-17: regions ATL 649, PAC 370, CNA 287, QUE 270; `type` TEMPORARY 779, DISCONTINUED 623, PERMANENT 122, UNKNOWN 41, SEASONAL 6, VIRTUAL 5; `operating: true` 140, `false` 1,436. Time-series codes present: `wlp` 1,089, `wlp-hilo` 1,081, `wlo` 471, `wlf` 65, `wcsp1` 30, `wcdp1` 30, `wcp1-events` 30, `wlf-spine` 26, `wcs1` 9, `wcd1` 9, `wt1` 9, `wlp-bores` 6, others fewer. Offer only `operating: true` stations with a `wlo`, `wlp` or `wlp-hilo` series; many listed predictions are for discontinued or temporary stations.

`GET /stations/{id}` returns the same record as the list entry. `GET /stations/{id}/metadata` adds `classCode`, `datums[]`, `heights[]` (reference heights; each has `heightTypeId`, `value`, `valuePrecision`, optionally `date`; resolve ids with `/height-types`), `isTidal`, `isTideTableReferencePort`, `tideTableId`, `timeZoneCode` (IANA name such as `Canada/Pacific`), `waterBody`, `status`, `owner`, `stationOwner`, and for current stations `ebbDirection` and `floodDirection` (degrees). `GET /stations/data/latest` returns the last 20 minutes of `wlo` for every station (239 KB): not needed by the app.

## Data requests

```text
GET /stations/{id}/data?time-series-code=wlp-hilo&from=2026-11-18T00:00:00Z&to=2026-11-20T00:00:00Z
GET /stations/{id}/data?time-series-code=wlp&from=2026-11-18T00:00:00Z&to=2026-11-19T00:00:00Z&resolution=FIFTEEN_MINUTES
GET /stations/{id}/data?time-series-code=wlo&from=2026-11-17T08:00:00Z&to=2026-11-17T09:00:00Z&resolution=FIFTEEN_MINUTES
```

Parameters: `time-series-code` (default `wlo`), `from` and `to` (required), `resolution` one of `ALL`, `ONE_MINUTE`, `THREE_MINUTES`, `FIVE_MINUTES`, `FIFTEEN_MINUTES`, `SIXTY_MINUTES` (default **`ONE_MINUTE`**: omit it and a day of `wlp` is 1,440 rows).

Response: a JSON array, oldest first, no envelope.

| Field | Type | Meaning |
|---|---|---|
| `eventDate` | string | ISO 8601 UTC, `Z`, to the second (`2026-11-18T00:15:00Z`) |
| `value` | number | metres above chart datum (levels), knots (current speed, see below), degrees (direction) |
| `qcFlagCode` | string | `"1"` good, `"2"` not evaluated, unavailable or unknown, `"3"` questionable or suspect (UNESCO ocean data standard flags, from the CHS page). Predictions come as `"2"` for `wlp` and `"1"` for `wlp-hilo`; observations as `"1"` after automated tests. |
| `reviewed` | boolean | `false` for all recorded points |
| `timeSeriesId` | string | joins to the station's `timeSeries[].id` |
| `qualifier` | string | only on `wcp1-events`: `SLACK`, `EXTREMA_FLOOD`, `EXTREMA_EBB` |

No missing-value marker was seen; gaps are absent rows. Recorded: Vancouver `wlp` at 15 minutes gives 97 points per UTC day and 769 for 8 days; `wlp` at 1 minute gives 60 per hour; `wlo` with `resolution=ALL` returns 1-minute points here.

### Resolutions and ranges (verified 2026-11-17)

The time-series definitions (`/time-series-definitions`) give `allowedPeriodInDays`: 7 for `wlo`, `wlp`, `wlf`, `wlf-spine`, `wlf-vtg`, `wlf-vh20`, `dvcf`, `dvcf-spine1`, `wcs*-derv`, `wcd*-derv`, `wcso`, `wcdo`; 366 for `wlp-hilo`, `wlp-bores`, `wcp1-events` and `wcp-reversing-falls`. The OpenAPI text for `resolution` says the allowed span is 7 days times the resolution in minutes, capped at 31 days, and one year for series without a fixed interval. Observed: `wlp` at `ONE_MINUTE` over 29 days answers 400 ("date interval should not be bigger than 7 days"); `SIXTY_MINUTES` over 32 days answers 400 ("... bigger than 31 days"); `FIFTEEN_MINUTES` over 8 days answers 200. Not every resolution exists at every station (OpenAPI note); the error for an unavailable resolution was **not** observed.

### Predictions

- `wlp`: predicted water level; the 1 minute and 15 minute resolutions were recorded, the others are listed by the OpenAPI document only (**unverified**). 1,089 stations.
- `wlp-hilo`: high and low water, each row an extremum with its time; the row does not say high or low, so derive it from neighbouring values. Vancouver on 2026-11-18: 03:22 2.785 m, 06:51 3.026 m, 13:04 2.028 m, 20:36 4.413 m (four extrema a day).
- Only the `wlf` family (forecasts including weather effects) is observation-adjusted; `wlp` is astronomical. The CHS page warns that predictions "are based upon the latest information available as of the date of your request" and that the printed Canadian Tide and Current Tables prevail in case of disparity.
- `GET /tide-tables` and `/tide-tables/{id}` list the printed table areas (`AREA`, `SUB_AREA`); `tideTableId` in the metadata ties a station to one. Not needed.

### Currents

Currents exist for 30 stations (mostly British Columbia narrows and Nova Scotia channels), in the series `wcsp1` (predicted speed), `wcdp1` (predicted direction), `wcp1-events` (predicted slack and extrema, 366 days per request), `wcs1`/`wcd1` (observed speed and direction), and `wcs2`/`wcd2`. Speed is in **knots**: the phenomena list (`/phenomena`) gives `wcs` unit `kn`, `wcd` unit `deg`, `wl` unit `m`, `wt` unit `c`, `ap` unit `hpa`, `ws` unit `psu`, `aws` unit `kn`. Examples at Second Narrows 07745 (fixtures): `wcsp1` 2.16 kn at 2026-11-18T00:00Z; `wcp1-events` rows `SLACK` 0.0 at 03:39Z, `EXTREMA_FLOOD` 0.655 at 05:15Z, `SLACK` at 06:42Z, `EXTREMA_EBB` 1.724 at 10:20Z. Flood and ebb directions are in the station metadata (flood 90, ebb 270). `wlp-bores` covers 6 stations (Moncton, Truro, Maccan and others) and `wcp-reversing-falls` one.

Speed values are signed? **Unverified**: all recorded `wcsp1` values are positive; use `wcp1-events` qualifiers for the flood/ebb phase and the metadata directions.

### Harmonic constituents

The OpenAPI document lists no harmonic-constituent endpoint (stations, station statistics, data, metadata, holdings, images, height types, tide tables, benchmarks, phenomena, time-series definitions). Constituents are **not** available from this API; the app should use `wlp` and `wlp-hilo`.

### Statistics

`/stations/{id}/stats/calculate-monthly-mean`, `calculate-daily-means` and `calculate-daily-means-igld85` exist (documented in OpenAPI; **unverified**, not called).

## Errors

Spring-style JSON, `application/json`:

```text
400 {"timestamp":"2026-11-17T10:13:23.939662114Z","status":"400 BAD_REQUEST","message":"Constraints not respected","path":"/api/v1/stations/.../data","errors":["2026-11-01T00:00:00Z / 2026-11-30T00:00:00Z: date interval should not be bigger than 7 days"]}
400 {"... ","message":"Wrong parameter format","errors":["from is improperly formatted"]}
404 {"detail":"No endpoint GET /api/v1/stations/<id>/datums.","status":404,"title":"Not Found"}   (application/problem+json)
```

429 body: **unverified** (not provoked).

## Licence and attribution

Sources: the OpenAPI `info.description` links "Terms of service" (`https://www.dfo-mpo.gc.ca/terms-avis/terms-conditions-avis-eng.htm`), "License of API" (`https://www.tides.gc.ca/en/licence-agreement`, French `https://www.marees.gc.ca/fr/accord-de-licence`, modified 2023-08-14) and a contact address for updates (`chsinfo@dfo-mpo.gc.ca`). The licence agreement was read in full on 2026-11-17. Terms that bear on the app:

- Non-exclusive licence for access to and use of CHS data; CHS keeps the copyright, and the data "shall not be sold, licensed, leased, assigned or given to a third party".
- The data "shall not be used in the purpose of producing derivative products for commercial purposes, for sale or profit under any form whatsoever". A free, ad-free app is not for profit, but commercial use is excluded: any paid tier or advertising would breach it.
- The data shall not be used for navigation.
- A derivative product (not for profit, not for navigation) "shall include the following notice in a prominent location": "This product is not to be used for navigation. This product was made by or for the User [name] and contains intellectual property (Data) of the Canadian Hydrographic Service of the Department of Fisheries and Oceans. The copyrights in the Data are and remain the property of His Majesty the King in Right of Canada and shall not be sold, licensed, leased, assigned or given to a third party. The incorporation of the Data in this product does not constitute an endorsement or an approval of this product by the Canadian Hydrographic Service, the Department of Fisheries and Oceans or His Majesty the King in Right of Canada".
- Metadata that further defines the data must be taken into account; the data reflect conditions at the time of collection; no warranty or liability.
- The agreement takes effect on use and ends when CHS posts a public notice; copies must then be destroyed within 15 days. Governing law is Ontario, forum the Federal Court of Canada.

Consequences: do not bundle IWLS data (station lists or predictions) in the app or in the repository beyond test fixtures; fetch at run time. The fixtures here are recordings for tests and carry the same copyright. Show the full notice above (or a link to it) wherever IWLS values appear, plus "Canadian Hydrographic Service, Fisheries and Oceans Canada" as the source and "not for navigation". Whether sharing cached snapshots between users (the `.underbite` file) counts as "given to a third party" is **unverified**; avoid including IWLS series in shared files until clarified with CHS.

## Known issues and changes

- No deprecation or outage notice found on the pages read (2026-11-17). The service asks users to send a name and email to `chsinfo@dfo-mpo.gc.ca` to receive update notices; this was not done.
- The `/stations/{id}/datums` path does not exist (404); datums are in `/metadata`.
- Default `resolution` is `ONE_MINUTE`; always send `resolution` explicitly.
