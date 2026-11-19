# NOAA GLERL ERDDAP: GLSEA sea surface temperature

Verified on 2026-11-16 (UTC) with `curl` against `https://apps.glerl.noaa.gov/erddap` (ERDDAP 2.31.1, `Erddap-Server` response header) and the dataset's own metadata. Fixtures: `tests/fixtures/raw/glerl/` (index in its `README.md`). The general description of this server (other datasets, license text, `GLCFS` not served) is in `tier2.md`.

## Dataset

| Item | Value |
|---|---|
| Dataset id | `GLSEA_ACSPO_GCS` |
| Title | Sea Surface Temperature from the Great Lakes Surface Environmental Analysis (ACSPO GLSEA), built from the ACSPO L3S-LEO SST |
| Variable | `sst`, `degree_C` (`standard_name` `sea_water_temperature`), raw `_FillValue` -99999.0; missing cells are `null` in JSON |
| Axes | `[time][latitude][longitude]` |
| Grid | 0.014015517 degrees in both directions (about 1.56 km north-south, 1.1 km east-west at 43 N); latitude 38.8749872 to 50.6059752, longitude -92.4199507 to -75.8816403 |
| Latitude order | **ascending** (the first row of a subset is the southernmost latitude: 41.2996717 for a request starting at 41.3). Unlike the 4 km VIIRS datasets on CoastWatch. |
| Cell centres | not on round numbers: 41.2996717, 41.3277027, 41.3557338, ... A request `(41.3):2:(42.1)` snaps to the nearest cell centre and then steps by two cells (0.0280 degrees). |
| Time | one field per day at **12:00:00Z** (`2026-11-05T12:00:00Z`, `...11-06T12:00:00Z`, ... eleven consecutive days seen); axis 2006-12-11T12:00:00Z (the `.das` `actual_range` low value 1.1658384e9) to 2026-11-15T12:00:00Z |
| Latest at check | 2026-11-15T12:00:00Z when checked at 23:49 UTC on 2026-11-16, so about 1 day behind; the dataset sets `testOutOfDate` "now-3days" |
| Licence text | "The data may be used and redistributed for free but is not intended for legal use, since it may contain inaccuracies..." (full text in the `.das`); institution "CoastWatch Great Lakes Node"; `infoUrl` `https://coastwatch.glerl.noaa.gov/glsea/glsea.html` |

## `(last)`

`[(last)]` resolves to the newest time. The request below returned `time` `2026-11-15T12:00:00Z`. A time after the newest one, `2026-11-16T12:00:00Z`, answered `404 Not Found` with the text `Your query produced no matching results ... "Start" is greater than the axis maximum=2026-11-16T12:00:00Z (and even 2026-11-15T12:00:00Z)`. So a client either uses `(last)` or reads the time axis first (`time[(2026-11-05):1:(last)]`, 0.5 KB), and does not ask for today.

## Requests

Western Lake Erie, newest day, every second cell (fixture `glsea-acspo-gcs-sst-lake-erie-west-2026-11-15.json`):

```
https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.json?sst%5B(last)%5D%5B(41.3):2:(42.1)%5D%5B(-83.6):2:(-82.6)%5D
```

The response is an ERDDAP table: `columnNames` `time`, `latitude`, `longitude`, `sst`; `columnUnits` `UTC`, `degrees_north`, `degrees_east`, `degree_C`; 1,073 rows (29 latitudes by 37 longitudes), of which 428 had a value (10.71 to 14.07 degree_C on 2026-11-15). The rest are `null` (land and cloud-masked cells). The analysis is clear-sky, so cloudy days leave holes; whether the product fills older values forward was not examined (**unverified**).

Time axis of the last days (fixture `glsea-acspo-gcs-time-2026-11-05-to-last.json`):

```
https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.json?time%5B(2026-11-05):1:(last)%5D
```

Metadata (fixture `glsea-acspo-gcs.das`): `https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.das`. The `history` attribute records the last file build at 02:51 UTC on 2026-11-16; the source file names carry a day of year (`2026_309_glsea.asc`).

## Limits

No limit is published for this server beyond the general ERDDAP documentation (see `erddap.md`, "Request limits"). Four requests at 6 to 8 second spacing on 2026-11-16 returned `200` (three) or the expected `404` (one past the end); no rate-limit header was present. No `429` was provoked.

## Credit

"NOAA Great Lakes Environmental Research Laboratory (CoastWatch Great Lakes Node)", with the sentence "not intended for legal use". The SST is a satellite analysis of the skin or near-surface water, not a gauge reading; it is not a navigation product.

Re-checked 2026-11-18: unchanged. The `license` attribute of `GLSEA_ACSPO_GCS` (`https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.das`) still reads "The data may be used and redistributed for free but is not intended for legal use, since it may contain inaccuracies...".

## Not verified

- Rate limits and the `429` response headers.
- Whether GLSEA applies a lake-ice mask (ice cover) in winter and how an ice-covered cell appears in `sst`.
- Other GLSEA datasets (`GL_Ice_Concentration_GCS`, `glsea_avgtemps_3`) were not re-requested.
