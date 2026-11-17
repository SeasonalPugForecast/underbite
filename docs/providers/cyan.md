# EPA CyAN (cyanobacteria index by lake)

Verified on **2026-11-17** (UTC) with `curl` against `https://cyan.epa.gov` and the EPA CyAN pages. This note extends the summary in `tier2.md`. Fixtures: `tests/fixtures/raw/cyan/` (index in its `README.md`). Items that could not be checked are marked **unverified**.

## Service

| Item | Value |
|---|---|
| Endpoint | `GET https://cyan.epa.gov/cyan/cyano/location/data/{lat}/{lon}/all?type=olci&frequency={weekly\|daily}` |
| Origin of the URL | the script of EPA's web application (`https://qed.epa.gov/cyanweb/`, HTTP 200 on 2026-11-17). EPA does not publish the path or a schema. |
| Authentication, headers | none; no `User-Agent` requirement seen. CORS headers allow `Authorization`, `Content-Type`, `App-Name`. |
| Latency | 0.7 s (daily) to 15.7 s (first weekly request for Oneida Lake), 4.7 s for points outside lakes. Use a timeout of at least 30 s. |
| Rate limits | none documented or reported (**unverified**). Four requests were made. |
| Official status statement | `https://www.epa.gov/water-research/cyanobacteria-assessment-network-application-cyan-app` (page "last updated on March 31, 2026"): "the CyAN REST API will remain unchanged and continue to provide processed cyanobacteria index (CI) estimate data", and "EPA will no longer support the CyAN Android mobile application after December 31, 2024". `https://www.epa.gov/water-research/cyanobacteria-assessment-network-cyan` (last updated June 9, 2026): daily and weekly Sentinel-3 OLCI coverage "from 2016 to current day"; "the original project concluded in 2023"; the project "moved to higher-resolution Sentinel-2 Multi-Spectral Imager data in 2024"; "the Sentinel-3 portion of the project will conclude, but ... the data will continue to be publicly available and reprocessed annually". |
| Terms | no licence text on either page. The web app states "This experimental web application provides provisional satellite derived measures of cyanobacteria, which may contain errors and should be considered a research tool." (recorded 2026-11-08; not re-read). Credit "U.S. EPA Cyanobacteria Assessment Network (CyAN)". |

## Records after 2026-03

On 2026-11-17 the newest record in every response was in March 2026:

- Oneida Lake (43.20, -76.00), weekly: newest `imageDate` 03-28-2026; the `description` names the file `L20260812026087.L3m_7D_CYAN_CI_cyano_CYAN_CONUS_300m_7_2_web.png` (year 2026, days 081 to 087, which ends 2026-03-28).
- Oneida Lake, daily: newest 03-27-2026 (`L2026086.L3m_DAY_...`, day 86), then 09-08-2025.
- No record for April to November 2026 at any point queried, although summer 2025 has weekly records (2025-08-16, 08-23, 08-30, 09-06, 09-13).

A web search snippet reported an EPA notice dated 2026-03-25 that the CyAN web application was temporarily unavailable; that notice was not found on the EPA pages fetched on 2026-11-17 (**unverified**) and the web application answered HTTP 200. Whether the March 2026 end is a lapse in processing, the end of the Sentinel-3 series, or a seasonal absence of retrievals (ice and cloud) could not be established. The feed therefore appears **stale for seven and a half months**. Treat CyAN as unavailable when the newest record is more than 14 days old and say "no recent satellite data", never "no bloom". Re-check before the 2027 season.

## Record fields (fixtures `cyan-oneida-lake-weekly-2026-11-17.json`, `-daily-`, `cyan-the-battery-weekly-2026-11-17.json`)

| Field | Meaning |
|---|---|
| `metaInfo.locationName` | name of the mapped waterbody nearest the point (`Maple Bay` for Oneida Lake at 43.20, -76.00; `The Battery` for 40.70, -74.02); `Unknown Location` when the point is not in a mapped waterbody |
| `metaInfo.locationLat`, `locationLng` | the requested point echoed |
| `metaInfo.description` | `location with image file name as <file>` (the image used; its name encodes the composite dates: `L<yyyy><ddd><yyyy><ddd>` for a 7-day composite, `L<yyyy><ddd>` for daily) or, for no waterbody, `This location is not a lake or is not yet available in our database.` |
| `metaInfo.status` | `Finished` (also for the unknown-location case) |
| `metaInfo.requestTimestamp`, `queryDate` (and `...Long`) | the time of the request as `MM-DD-YYYY HH:MM:SS:mmm UTC` and epoch milliseconds; not the data date |
| `metaInfo.url.href` | an internal host name (`http://tomcat:8080/...`); ignore |
| `outputs[]` | newest first, only composites with a valid retrieval; empty for unknown locations |
| `outputs[].imageDate`, `imageDateLong` | date of the composite as `MM-DD-YYYY 04:00:00:0 UTC` (05:00 outside daylight saving: US Eastern midnight, so the local date is the one shown) and epoch milliseconds. Weekly composites are stamped on the **last** day of the 7-day window. |
| `outputs[].satelliteImageType`, `satelliteImageFrequency` | `OLCI`; `Weekly` or `Daily` |
| `outputs[].cellConcentration` | mean over valid 300 m cells near the point, in cells per mL (unit label from the web app, not from the API; **unverified** in the API) |
| `outputs[].maxCellConcentration` | maximum over the same cells |
| `outputs[].validCellsCount` | number of cells with a valid retrieval (2 to 6 seen) |
| `outputs[].latitude`, `longitude` | the cell used (snapped: -76.0017, -75.998 for Oneida Lake) |

Special pixel values of the underlying image (below detection, land, no data) never appear as numbers: such cells are simply absent from the record (**unverified**; inferred from the absence of zeros and sentinels in 16 + 24 + 4 records).

### A summer week with values (Oneida Lake, weekly, recorded 2026-11-17)

| `imageDate` | `cellConcentration` | `maxCellConcentration` | `validCellsCount` |
|---|---|---|---|
| 08-16-2025 | 9,549.9 | 108,642.6 | 6 |
| 08-23-2025 | 131,825.7 | 139,315.7 | 6 |
| 08-30-2025 | 121,338.9 | 121,338.9 | 6 |
| 09-06-2025 | 77,983.0 | 108,642.6 | 6 |
| 09-13-2025 | 25,822.6 | 40,179.1 | 4 |

The values are the cyanobacteria index converted to cells/mL; they are quantised (identical values repeat: 108,642.56, 40,179.082). The index is not a toxin measurement. The API returns no bloom category; thresholds for wording are **unverified** and must come from a cited guideline.

### Shapes to handle

1. A normal lake: `locationName` set, `outputs` with records (fixture `cyan-oneida-lake-weekly-2026-11-17.json`).
2. Not a mapped waterbody: `locationName` `Unknown Location`, `description` "This location is not a lake or is not yet available in our database.", `outputs` `[]`, HTTP 200 (fixture `cyan-new-croton-unknown-location-2026-11-17.json` at 41.20, -73.79, the New Croton Reservoir). Lake Champlain (44.55, -73.30) gave the same on 2026-11-08.
3. A mapped waterbody with sparse data: The Battery (40.70, -74.02) returned four weekly records in four years (2021-01-30, 2023-12-02, 2025-02-01, 2025-02-22), none in summer. CyAN includes some estuaries; do not read "mapped" as "lake".
4. Mapped but no records: `outputs` empty with a named location (not seen; **unverified**).

Coverage: about 2,000 of the largest US lakes and reservoirs according to EPA's description of the web app (not a count taken from the API).

## Usage notes

- A weekly response is small (4.5 KB for 16 records; daily 6.5 KB for 24). Request `weekly` for the card and `daily` only for a detail view.
- Round the coordinates to two decimals before the request (the privacy rule). Two decimals is about 1 km, larger than the 300 m cell, so the response may describe a neighbouring cell or waterbody; query a lake's own representative point where one is known.
- Cache per (rounded point, frequency) for 24 hours: the data change at most daily.
