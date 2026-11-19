# USACE CWMS Data API (reservoir pool, release and inflow)

Verified on **2026-11-17** (UTC) with `curl` and `python3` against `https://cwms-data.usace.army.mil/cwms-data`. This note extends the summary in `tier2.md`. Fixtures: `tests/fixtures/raw/cwms/` (index in its `README.md`). Items that could not be checked are marked **unverified**.

## Service

| Item | Value |
|---|---|
| Base URL | `https://cwms-data.usace.army.mil/cwms-data` |
| Version | request header `Accept: application/json;version=2`; the response `Content-Type` is `application/json;version=2`. OpenAPI document `/cwms-data/swagger-docs` reports "CWMS Data API", version `2026.09.22-b` (checked 2026-11-17, unchanged since 2026-11-08). |
| Authentication | none for reads. API keys and OpenID Connect apply to writes only. |
| Required headers | none beyond `Accept`. A `User-Agent` was always sent (`underbite-scout/0.1`). Responses set a `session` cookie (600 s); it is not needed. |
| Rate limits | none documented and no rate-limit or `Retry-After` headers on any response seen (about 120 requests over two hours, paced at one per second). **Unverified**: whether a 429 is ever returned. |
| Terms | the landing page `/cwms-data/` carries no terms. The project documentation (`cwms-data-api.readthedocs.io`) footer reads "Copyright Public Domain". Corps content is a US federal work; credit "U.S. Army Corps of Engineers". Corps data are provisional unless stated; no attribution text is prescribed (**unverified**: no terms page was found). |
| Time zone and format | JSON; every `date-time` is epoch **milliseconds, UTC**. The `time-zone` member names the location's zone (`US/Eastern`, `US/Central`, `Etc/GMT+6`, ...) and is informational. Request `begin` and `end` as ISO 8601 UTC (`2026-11-17T06:00:00Z`); `begin` and `end` are echoed in the response. |
| Units | `unit=SI` returns `m`, `m3`, `cms`; the default system (`EN`) returns `ft`, `ac-ft`/`kaf`, `cfs`. Always pass `unit=SI`. |

## Requests the app needs

```text
# 1. Projects of an office, with coordinates (find the project near a spot)
GET /cwms-data/catalog/LOCATIONS?office=NAB&location-kind-like=PROJECT&page-size=100
# 2. Series of one project (pool, storage, inflow, outflow) by location name
GET /cwms-data/catalog/TIMESERIES?office=NAB&like=Raystown%5C..*&page-size=100
# 3. The data
GET /cwms-data/timeseries?office=NAB&name=Raystown.Elev.Inst.15Minutes.0.Best-NAB
      &begin=2026-11-17T06:00:00Z&end=2026-11-17T08:00:00Z&unit=SI
# 4. One location (public name, coordinates, county)
GET /cwms-data/locations/Raystown?office=NAB
GET /cwms-data/offices
```

- The `like` parameter is a regular expression over the full series name. Escape the dot as `%5C.`. Unencoded `(`, `|` or `)` produce an HTTP 400 HTML page; percent-encode them (`%28`, `%7C`, `%29`). An alternation such as `.*%5C.%28Elev%7CStor%7CFlow-Out%7CFlow-In%29%5C..*` worked.
- `catalog/TIMESERIES` has `page-size` (a value of 5000 was accepted) and cursor paging, plus `like`, `timeseries-category-like`, `timeseries-group-like`, `location-category-like`, `location-group-like`, `bounding-office-like`, `include-extents`, `location-kind-like`, `negate-location-kind-like`, `location-type-like`, `exclude-empty`, `search-text` (parameter list from the OpenAPI document). There is **no bounding-box filter**: find projects through the per-office project list (request 1) and compare coordinates on the device or in a bundled table.
- `catalog/LOCATIONS?location-kind-like=PROJECT` returned 17 projects for NAB (9.7 KB) with `latitude`, `longitude` (WGS84), `public-name`, `state`, `county`, `time-zone`, `nearest-city`, `active`. Coordinates in `/locations/{name}` are the same.
- Offices: `/offices` lists 38 district offices (type `DIS`); only some hold reservoir data. An empty result is `{"entries":[],"page-size":N,"total":0}` with HTTP 200.
- Maximum range per request: not documented. The default `page-size` of a `timeseries` response is 500 values with a `next-page` cursor; 3.5 days of 15-minute data (329 values) came back in one page. **Unverified**: a hard limit on `begin` to `end`.

## Response fields (`timeseries`, fixtures `cwms-*-2026-11-17.json`)

| Field | Meaning |
|---|---|
| `name` | `Location.Parameter.Type.Interval.Duration.Version`; for example `Raystown.Elev.Inst.15Minutes.0.Best-NAB` |
| `office-id`, `units`, `time-zone` | as above; `units` is `m`, `m3` or `cms` under `unit=SI` |
| `interval`, `interval-offset` | ISO 8601 duration (`PT15M`, `PT1H`, `PT30M`, `PT24H`, or `PT0S` for irregular and `~1Day`) and offset in minutes |
| `begin`, `end`, `total`, `page`, `page-size`, `next-page` | echo of the window and paging |
| `value-columns` | `date-time` (epoch ms UTC), `value` (double), `quality-code` (int) |
| `values` | rows `[epoch_ms, value, quality]`. A missing value is `null` with quality `5`. |
| `vertical-datum-info` | `native-datum` (`NAVD-88`, `NGVD-29`, `OTHER`), `local-datum-name` (`PCD` at Raystown), station `elevation`, and `offsets` to other datums (metres under SI). Present on elevation series only. |
| `date-version-type` | `UNVERSIONED` for the series above |

## Quality codes (verified 2026-11-17)

The `quality-code` is the CWMS 32-bit quality integer. Source: the Hydrologic Engineering Center Python library documentation (`https://hydrologicengineeringcenter.github.io/hec-python-library/hec/quality.html`), compared with the codes the live API returned.

| Bits | Field | Values |
|---|---|---|
| 0 | screened | 0 unscreened, 1 screened |
| 1 to 4 | validity (mutually exclusive) | 0 unknown, 1 okay (bit 1), 2 missing (bit 2), 4 questionable (bit 3), 8 rejected (bit 4) |
| 5 to 6 | value range | 0 to 3 |
| 7 | changed | 0 original, 1 modified |
| 8 to 10 | replacement cause | 0 none, 1 automatic, 2 interactive, 3 manual, 4 restored |
| 11 to 14 | replacement method | 0 none, 1 linear interpolation, 2 explicit, 3 missing, 4 graphical |
| 15 to 25 | test failed (flags, several may be set) | absolute value, constant value, rate of change, relative value, duration, negative increment, skip list, user defined, distribution |
| 31 | protected | 0 or 1 |

Named whole codes from that page: 0 unscreened, 1 unknown (screened), **3 screened okay**, **5 screened missing**, **9 screened questionable**, **17 screened rejected**. If the screened bit is not set no other bit may be set.

Seen in live data: `0` on unscreened series with a value (MVP `rev`, SAM `Raw-*`, NAE `DCP-rev`, NAP `Computed`, NAB `Stor`), `3` on screened series (NAB `Best-NAB`, MVS `lrgsShef-rev`, LRL `USGS-rev`), and `5` with `value: null` (NAP Beltzville, two newest rows on 2026-11-08). The codes 9 and 17 did not occur in about 30 series over 3.5 days.

Rule for the app: a `null` value is missing whatever the code; show a value with quality 0 or 3; hide a value with the validity field of 4 (questionable, `(q >> 1) & 15 == 4`) or 8 (rejected), or label it. Quality `0` does not mean bad: the series is unscreened.

## Finding pool, outflow and inflow for a project

1. Pick the project's office (the district, `DIS` in `/offices`) and location name from the per-office project list.
2. Request the series catalogue with `like=<Location>\..*`. A project has many versions of each parameter. Prefer, in this order: a `15Minutes` or `1Hour` `Inst` series whose version says `Best-*`, `rev`, `Rev-*`, `Raw-USGS` or `DCP-rev`; avoid `Fcst-*`, `CWMS-Forecast*`, `National-CWMS-Forecast`, `*Manual`, `*Smoothed*`, `*-PY3-AI2`, `ARCHIVE-*`, `HISTORIAN_*`, `Regulating` and `delete`.
3. Parameters are `Elev` (pool), `Stor`, `Flow-In`, `Flow-Out`. Some projects carry tailwater or gate series with other names (`SmithlandDam-TW`, `CFR BEJ-gate 10`, `LockDam_01`).
4. Check data with `timeseries`, not with the catalogue: catalogue `extents.latest-time` was stale for every series checked (it said 2026-09-14 on 2026-11-17 while data ran to 2026-11-17 09:30Z).
5. Series names contain spaces and punctuation (`Carlyle Lk-Kaskaskia`, `Oliver(GPC)`, `Best-NAB;88`): percent-encode them.

## Curated projects (data checked on 2026-11-17)

Window checked: 2026-11-14 00:00Z to 2026-11-17 12:00Z. "Latest" is the newest non-null row. All have `Flow-Out` unless stated.

| Office | Location (public name, state) | Pool series (`Elev`) | Outflow series (`Flow-Out`) | Notes |
|---|---|---|---|---|
| NAB | `Raystown` (Raystown Lake, PA) | `Raystown.Elev.Inst.15Minutes.0.Best-NAB` m, q 3, latest 09:00Z | `Raystown.Flow-Out.Inst.15Minutes.0.Best-NAB-Computed-PY3` cms, q 3, latest 09:30Z | `Stor.Inst.15Minutes.0.Best-NAB`; `Flow-In.Inst.1Hour.0.Best-NAB-Computed`. Datum `OTHER` (project datum `PCD`; offset -0.2042 m to NAVD-88). Fixtures. |
| NAB | `Cowanesque` (Cowanesque Lake, PA) | `Cowanesque.Elev.Inst.15Minutes.0.Best-NAB` m, q 3 | `Cowanesque.Flow-Out.Inst.15Minutes.0.Best-NAB-Computed` cms, q 3 | datum `OTHER` |
| NAB | `Almond`, `Arkport`, `Aylesworth`, `Bush`, `Curwensville`, `East Sidney`, `Hammond`, `Indian Rock`, `Sayers`, `Stevenson`, `Stillwater`, `Tioga`, `Whitney Point`, `Savage`, `Randolph` | same naming (`<Name>.Elev...Best-NAB`) | same naming | **Unverified** for data; all listed in the catalogue with `Elev`, `Flow-In`, `Flow-Out`. NAB also lists a `<Name>-<River>` twin location with `National-CWMS-Forecast` series (forecast only). |
| NAE | `BRD` (Black Rock Dam, CT) | `BRD.Elev.Inst.15Minutes.0.DCP-rev` m, q 0, latest 09:30Z | only `BRD.Flow-Out.Inst.1Hour.0.CWMS-Forecast` (forecast) | closest pool data to the New York test places (about 60 km from the Croton River). 16 NAE project codes (BRD, BVD, BWD, EBD, EMD, EVD, FFD, HBD, HCD, HKD, HVD, MHD, NBD, TMD, WTD, WVD) have `Elev`, `Stor`, `Flow-In`, `Flow-Out`; only BRD was probed. Location codes are Corps abbreviations; names come from `/locations`. |
| NAP | `Beltzville` (Beltzville Dam, PA) | `Beltzville.Elev.Inst.15Minutes.0.Rev-DCP` m, q 3 and 5 | none | `Flow-In.Inst.1Hour.0.Computed`. Also `BlueMarsh`, `FEWalter`, `Jadwin`, `Prompton` with `Elev`, `Flow-In`, `Stor` only: **no `Flow-Out` in NAP**. |
| NAN | none | none | none | no `Elev` in the New York District (checked 2026-11-08 and again by catalogue 2026-11-17). |
| MVP | `Baldhill_Dam` (Baldhill Dam at Lake Ashtabula, ND) | `Baldhill_Dam.Elev.Inst.15Minutes.0.rev-NGVD29` m, q 0, latest 10:00Z (datum NGVD-29) | `Baldhill_Dam.Flow-Out.Inst.15Minutes.0.rev` cms, q 0 | MVP uses US/Central; many forecast series `Fcst-NCRFC-CHIPS`. Fixtures. |
| MVP | `MissHW_Leech` (Leech Lake Dam, MN) | `MissHW_Leech.Elev.Inst.15Minutes.0.rev-NGVD29` m, q 0 | `MissHW_Leech.Flow-Out.Inst.15Minutes.0.rev` cms, q 0 | other headwaters reservoirs `MissHW_Gull`, `_Pine River`, `_Pokegama`, `_Sandy`, `_Winni`, `RedLake_Dam`, `LacQuiParle_Dam`: unverified for data |
| MVS | `Carlyle Lk-Kaskaskia` (Carlyle Pool, IL) | `Carlyle Lk-Kaskaskia.Elev.Inst.30Minutes.0.lrgsShef-rev` m, q 3, latest 10:00Z (NAVD-88) | `Carlyle Lk-Kaskaskia.Flow-Out.Ave.~1Day.1Day.lakerep-rev` cms, **daily mean**, q 0, latest 2026-11-16 06:00Z | `Lk Shelbyville-Kaskaskia`, `Rend Lk-Big Muddy`, `Wappapello Lk-St Francis` follow the same pattern (unverified) |
| MVS | `Mark Twain Lk-Salt` (Mark Twain Pool, MO) | `Mark Twain Lk-Salt.Elev.Inst.30Minutes.0.lrgsShef-rev` m, q 3 | `Mark Twain Lk-Salt.Flow-Out.Ave.~1Day.1Day.lakerep-rev` (not probed) | forecast series `CWMS-Forecast-NoQPF` etc. dated 2026-10 |
| SAM | `Allatoona` (Allatoona Dam and Lake, GA) | `Allatoona.Elev.Inst.1Hour.0.Raw-USGS` m, q 0, latest 09:00Z (NGVD-29) | `Allatoona.Flow-Out.Inst.15Minutes.0.Raw-USGS` cms, q 0 | `time-zone` is `Etc/GMT+6` (fixed UTC-6, no daylight saving). Fixture. |
| SAM | `Buford` (Buford Dam and Lake Lanier, GA) | `Buford.Elev.Inst.15Minutes.0.Raw-USGS` m, q 0 | `Buford.Flow-Out.Inst.15Minutes.0.Raw-UR` cms, q 0 | |
| SAM | `Carters` (Carters Dam and Lake, GA) | `Carters.Elev.Inst.15Minutes.0.Raw-USGS` m, q 0 | `Carters.Flow-Out.Inst.15Minutes.0.Raw-Rereg_Out` (not probed) | |
| SAS | `Hartwell` (Hartwell Dam, GA/SC) | none live: `Elev.Inst.~1Day.0.ARCHIVE-DAILY` ended 2025-10 | `Hartwell.Flow-Out.Ave.1Hour.1Hour.Raw-SHEF_SAS` cms, hourly mean, q 3, latest 2026-11-17 12:00Z | `Stor.Inst.1Hour.0.Raw-SHEF_SAS` m3 is live (q 0). `HISTORIAN_SAS` and `ARCHIVE-DAILY` series were empty for 2026-11-14 to 17. `Thurmond`, `Russell` the same layout. |
| LRL | `Barren-Lake` (Barren River Lake, KY; a `SITE`) | `Barren-Lake.Elev.Inst.15Minutes.0.USGS-rev` m, q 3, latest 09:30Z (NGVD-29) | none catalogued | LRL has only `SmithlandDam-TW.Flow-Out.Inst.1Hour.0.Fcst-LRD-HMS` (forecast) |
| SPK | `Boca` (Boca Dam, CA) | `Boca.Elev.Inst.1Hour.0.Calc-usgs` m, q 0, latest 09:00Z (NGVD-29) | none live | other SPK projects use `Calc-val` series |
| SWL | `Bull_Shoals_Dam`, `Table_Rock_Dam` (AR/MO) | only `National-CWMS-Forecast` series (empty for the checked window) | forecast only | observed series not found in the catalogue: **unverified** whether observed data are published elsewhere in CWMS |

Offices with `Flow-Out` series in the catalogue on 2026-11-17: NAB 67, NAE 16, MVP 75, MVS 44, SAM 47, SAS 15, SAW 25, SWF 304, SWL 12, LRL 1, SWT 1. Offices with none: NAP, NAN, LRH, LRN, LRP, MVR, SAJ, SPK, SPL, NWK, NWO, NWP, NWS, NWW (the last five returned empty catalogues for every query; their data may be stored under a different office id, which was not found).

## Findings and limits

- No outflow series exists for NAP (Philadelphia) projects, including Beltzville. Releases at those dams are not available from CWMS.
- Pool elevation exists at the Corps' New England (NAE) projects and at Baltimore (NAB) reservoirs in Pennsylvania and New York State's Southern Tier (Susquehanna and Chemung basins); none for NAN.
- Vertical datum differs per project (`NAVD-88`, `NGVD-29`, or a project datum `OTHER`/`PCD`). Show the pool as height above the datum named in `vertical-datum-info`; "vs normal pool" needs conservation-pool elevations, which this API did not provide in the requests above (**unverified**: a `projects` or `levels` endpoint was not tried).
- Series are updated about every 15 minutes to hourly; the newest row was 0 to 3 hours old when checked.
- Series using a non-UTC `time-zone` (`US/Central`, `Etc/GMT+6`) still return UTC epochs.

## Licence, terms and attribution

Re-checked 2026-11-18: **unverified** (no terms page reachable). The OpenAPI `info` of `/cwms-data/swagger-docs` (version 2026.09.22-b) has title and description only, no licence. The project documentation (`https://cwms-data-api.readthedocs.io/latest/`) still ends "Copyright Public Domain.". The USACE site pages `https://www.usace.army.mil/Disclaimer/` and `.../Privacy-and-Security/` answered HTTP 403 to `curl`. No attribution text is prescribed anywhere read; the credit "U.S. Army Corps of Engineers" is a convention.
