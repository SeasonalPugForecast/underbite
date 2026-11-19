# NOAA National Water Prediction Service (NWPS) API

Verified on 2026-11-08 against the live service with `curl`, no key and no special headers other than
`User-Agent`. Recorded responses are in `tests/fixtures/raw/nwps/` (index:
`tests/fixtures/raw/nwps/README.md`). Anything not checked is marked **unverified**.

## Summary

- Base URL `https://api.water.noaa.gov/nwps/v1`. Swagger UI at `/nwps/v1/docs/`; the machine-readable
  definition (Swagger 2.0 JSON, title "National Water Prediction Service API", version `1.0.0`) is at
  `https://api.water.noaa.gov/nwps/v1/docs/swagger.json`. `/nwps/v1/openapi` and `/nwps/v1/openapi.json`
  answer 404.
- No authentication. No rate-limit header was present on any response; no published limit was found.
  **Unverified**: any throttling. About 15 requests were made.
- Responses are plain JSON (`application/json`), not GeoJSON. Errors: HTTP 404 with
  `{"code":5,"message":"[ZZZZ9] could not find gauge ID","details":[]}`.
- The service sets load-balancer cookies (`AWSALB`, `AWSALBCORS`); they are not needed and not stored.
- Data are public domain NOAA/NWS products. Official forecasts come from River Forecast Centers (here
  NERFC and its Weather Forecast Offices). Credit text found in the data: `dataAttribution` objects name
  "US Geological Survey" for observations and, for tide gauges, "NOAA National Ocean Service". The API
  page describes the NWPS application API as official streamflow forecasts, observations, National Water
  Model output, crest history, impacts, flood categories and location metadata, and says it holds no
  history beyond crest and low-water history (<https://water.noaa.gov/about/api>).
- NOAA labels two sister services (HEFS ensembles and the National Water Model) experimental, "not
  supported 24/7"; the reach endpoint used below returns National Water Model output and may carry the
  same caveat (**unverified** for this endpoint).

## Endpoints (from `swagger.json`, each one tested unless noted)

| Request | Purpose |
|---|---|
| `GET /gauges?bbox.xmin=&bbox.ymin=&bbox.xmax=&bbox.ymax=&srid=EPSG_4326` | gauges in a box (longitude = x). Returns `{"gauges":[...]}` with a summary per gauge. A New York box (`-79.8,40.5,-71.8,45.1`) returned 783 gauges, 800 KB; use small boxes |
| `GET /gauges/{identifier}` | metadata, status, flood categories, crests, impacts, datums. `{identifier}` is the NWS location id (LID, for example `AUON6`) or the USGS site number (`04235440` returned HTTP 200) |
| `GET /gauges/{identifier}/stageflow` | observed and forecast together; the observed part is the last 30 days at 15-minute spacing, about 470 KB |
| `GET /gauges/{identifier}/stageflow/forecast` | forecast only (2 KB) |
| `GET /gauges/{identifier}/stageflow/observed` | observed only (about 430 KB for 2823 values) |
| `GET /products/stageflow/{identifier}/{pedts}` | same body, selected by product code (`HGIFF`, `HGIRG`); returns the same data as the two lines above |
| `GET /reaches/{reachId}/streamflow?series=` | National Water Model flow for a stream reach; `series` is `analysis_assimilation` (default), `short_range`, `medium_range`, `long_range`, `medium_range_blend` |
| `GET /gauges/{identifier}/ratings`, `/gauges/{identifier}/riverflow`, `POST /gauges/raag`, `GET /reaches/{reachId}`, `GET /monitor` | ratings, upstream and downstream gauges, other; **unverified** (not called) |

There is no query parameter to limit the observed series; the app should call `stageflow/forecast`, and
call `stageflow/observed` only if it needs more than the USGS values (the USGS API already supplies
observed data).

## Fields

### Gauge list item (`gauges[]`)

`lid`, `name`, `rfc{abbreviation,name}`, `wfo{abbreviation,name}`, `state{abbreviation,name}`,
`latitude`, `longitude` (decimal degrees, WGS84), `pedts{observed,forecast}`, `status{observed,forecast}`.
`pedts.forecast` is `""` when the gauge has no forecast (213 of 783 New York box entries); 3 had no
observed product either.

### Gauge (`/gauges/{identifier}`), in addition

| Field | Example | Notes |
|---|---|---|
| `lid` | `AUON6` | NWS location id |
| `usgsId` | `04235440` | USGS site number without the `USGS-` prefix; `""` or absent when the gauge is not USGS-operated (`BATN6` has `""`). This is the mapping to the `USGS-<usgsId>` monitoring location of the Water Data API |
| `reachId` | `6186206` | National Water Model reach for `/reaches/{reachId}/streamflow` |
| `name`, `description`, `county` | `Owasco Outlet at Auburn`, `Cayuga` | names can contain `(IN MLLW)` when the stage is relative to tidal datum |
| `timeZone` | `EST5EDT` | local zone of the gauge; every timestamp is UTC (`Z`) |
| `pedts` | `{"observed":"HGIRG","forecast":"HGIFF"}` | SHEF product codes: `HG` stage, `HP` pool elevation (lakes, reservoirs), `HM` tide-gauge height (MLLW), `QT` flow; the forecast code ends `FF`, `FE` or similar. 565 of 783 New York observed products were `HGIRG` |
| `status.observed`, `status.forecast` | `{primary, primaryUnit, secondary, secondaryUnit, floodCategory, validTime}` | latest observed value, and one forecast value: on the day recorded it was the highest forecast value still in the future (Cohoes 8.9 ft at 2026-11-10T06:00Z, while the earlier 10.4 ft at 2026-11-08T12:00Z had passed), so it is the forecast peak, not the next value; read the series for the hydrograph |
| `flood.stageUnits`, `flood.flowUnits` | `ft`, `cfs` | |
| `flood.categories.{action,minor,moderate,major}.{stage,flow}` | `{"stage":8,"flow":-9999}` | thresholds; `-9999` = not defined. Owasco Outlet: 7 / 8 / 9 / 10 ft. Mohawk at Cohoes: 17 / 20 / 21 / 22 ft. No gauge tested had a flow threshold |
| `flood.crests.recent[]`, `.historic[]` | `{"occurredTime":"2011-09-08T19:50:00Z","stage":20.57,"flow":94275,"preliminary":"O","olddatum":false}` | flow in cfs here |
| `flood.impacts[]` | `{"stage":22,"statement":"Major flood stage."}` | |
| `flood.lro` | `{"minorCS":"< 0.05",...,"interval":"NDJ"}` or `null` | long-range outlook probabilities |
| `datums.vertical.value[]` | `{"abbrev":"NAVD88","value":49.13,...}` | datum of the gauge zero |
| `hydronotes[]` | `{"statement":"Gauge reading affected by ocean tides.","effective":"0101","expiration":"1231"}` | `MMDD` range; also an ice note valid 1 December to 31 March |
| `upstreamLid`, `downstreamLid` | `TRYN6`, `PKMN6` | neighbouring gauges |
| `inService{enabled,message}` | | |
| `forecastReliability` | `Forecasts are issued routinely year-round.` | free text |
| `images.hydrograph.{default,floodcat}` | `https://water.noaa.gov/resources/hydrographs/auon6_hg.png` | NWS-drawn hydrograph images |
| `dataAttribution[]` | `{"abbrev":"USGS","text":"04235440","title":"Observations courtesy of U.S. Geological Survey","url":...}` | credit lines to show |
| `ObservedFloodCategory`, `ForecastFloodCategory` | `no_flooding` | duplicates `status.*.floodCategory` |
| `TruncateObs`, `TruncateFcst` | `"-1"`, `"3"` | strings; meaning **unverified** |

`floodCategory` values seen across 783 New York gauges: `no_flooding`, `not_defined`, `action`,
`low_threshold`, `obs_not_current`, `fcst_not_current`, `out_of_service`. Values for `minor`, `moderate`
and `major` were not on the day recorded and are **unverified** but follow the `categories` keys.

### Stage and flow series (`stageflow`)

```json
{"pedts":"HGIFF","issuedTime":"2026-11-07T16:45:00Z","wfo":"BGM","timeZone":"EST5EDT",
 "primaryName":"Stage","primaryUnits":"ft","secondaryName":"Flow","secondaryUnits":"kcfs",
 "data":[{"validTime":"2026-11-07T18:00:00Z","generatedTime":"2026-11-07T16:54:22Z","primary":1.8,"secondary":0.204}]}
```

| Field | Notes |
|---|---|
| `issuedTime` | when the forecast was issued (UTC); the observed series carries the time of the last update |
| `data[].validTime` | UTC instant the value is for |
| `data[].generatedTime` | when NWPS produced the record |
| `data[].primary` | stage in `primaryUnits` (feet) |
| `data[].secondary` | flow in `secondaryUnits`: **kcfs**, thousands of cubic feet per second (0.204 kcfs = 204 cfs) |
| missing value | `-999` (observed, and gauge status) or `-9999` (forecast flow, thresholds). Treat any negative flow or `-999`/`-9999` as absent; stage can be legitimately negative (Albany `-0.47` ft), so test for the exact sentinels |

Cadence: observed 15 minutes (Owasco), forecast every 6 hours for 3 days (Owasco, 12 points; Cohoes,
12 points) or hourly for 3 days (Albany, 72 points, stage only: the flow field is `-9999`). Albany's
observed flow is `-999`; its stage is tidally affected (see `hydronotes`). The forecast recorded was issued
2026-11-07 16:45Z and was still the current one about 22 hours later; the update schedule is
**unverified** (the gauge `forecastReliability` says forecasts are routine, year-round).

A forecast can be flat (Owasco Outlet forecast 1.8 ft throughout, at low water) or vary (Mohawk at
Cohoes forecast 8.6, 10.2, 8.2 ... 8.8 ft).

### Reach streamflow (National Water Model)

`reach{reachId,name,latitude,longitude,streamflow[],route{upstream[],downstream[]}}` plus one object per
series: `analysisAssimilation`, `shortRange`, `mediumRange`, `longRange`, `mediumRangeBlend`. Only the
requested `series` is filled (`{}` otherwise). Each filled object is
`{"series":{"referenceTime":"2026-11-08T12:00:00Z","units":"ft³/s","data":[{"validTime":"2026-11-08T13:00:00Z","flow":18.01}]}}`
with `flow` in cubic feet per second (not kcfs), hourly. This gives a model flow forecast for ungauged
streams (fixtures for reach 6228280, Cross River at Katonah, a Croton River tributary); it is model
guidance, not an official forecast.

## Mapping a gauge to other sources

- USGS: use `usgsId` (`04235440` is `USGS-04235440` in the Water Data API). Observed stage and flow in
  NWPS are the USGS data relayed; NWPS adds forecasts and flood categories. Gauges tested with a mapping:
  AUON6 to 04235440, COHN6 to 01357500, ALBN6 to 01359139, KTNN6 to 01374901. A New York site with
  both a USGS record and a forecast is therefore found by listing NWPS gauges in a box and joining on
  `usgsId`.
- Tide gauges (`BATN6`, "New York Harbor at The Battery (IN MLLW)", 40.700556, -74.014167) carry no
  CO-OPS station id; the CO-OPS station 8518750 is at 40.7006, -74.0142 (CO-OPS metadata), so the match is
  by name and position. The NWPS observed stage was 4.98 ft at 13:36Z when CO-OPS reported
  4.222 ft MLLW at 14:24Z (a different minute, so not a comparison). Use CO-OPS for the Battery; NWPS
  adds only flood category thresholds (action 7, minor 7, moderate 8.3, major 9.4 ft MLLW).
- The Croton River site USGS 01375000 and the Hudson sites near it had no NWPS gauge in the box
  `-74.4,40.9,-73.6,41.6` (32 gauges; the Hudson ones are `PMTN6` Piermont and `WPTN6` West Point);
  the nearest forecast gauges in New York are `COHN6`, `ALBN6`, `AUON6`.
- Gauges with a pool-elevation product (`HPIRG`/`HPIFF`) cover lakes and reservoirs, for example
  Canandaigua Lake (`CNDN6`) and Ashokan Reservoir East (`ASEN6`); only the list entry was inspected.

## Test places

| Gauge | Finding on 2026-11-08 |
|---|---|
| `AUON6` Owasco Outlet at Auburn NY (USGS 04235440, 42.9322, -76.5653) | observed 1.76 ft and 0.195 kcfs; flat 1.8 ft forecast; categories defined |
| `COHN6` Mohawk River at Cohoes NY (USGS 01357500) | observed 9.83 ft and 3.29 kcfs; forecast varies; 59 recent crests |
| `ALBN6` Hudson River at Albany NY (USGS 01359139) | tidal, stage forecast only, flow sentinels |
| `KTNN6` Cross River at Katonah NY (USGS 01374901, Croton watershed) | observed only, no forecast, flood categories `-9999` |
| `BATN6` New York Harbor at The Battery | tide-height forecast to 2026-11-10; no USGS id |

## Items not verified

- Throttling and any usage policy beyond the NOAA public-domain statement.
- Meaning of `TruncateObs` and `TruncateFcst`; the minor, moderate and major `floodCategory` strings.
- Behaviour of `ratings`, `riverflow`, `raag` and `monitor`.
- Whether the reach endpoint shares the experimental label of the other National Water Model services.
- Outages: none seen; no deprecation notice found for `/nwps/v1`.

## Licence, terms and attribution

Re-checked 2026-11-18: unchanged. The NWPS site footer (`https://water.noaa.gov/`) links to the NWS disclaimer, `https://www.weather.gov/disclaimer`, read the same day: public domain unless noted and usable for any lawful purpose if one does not claim it as one's own, imply NOAA or NWS endorsement, or modify it and present it as official; the NWS name and visual identifier are protected and not to be used without permission except to identify unaltered content; experimental products are "intended for research use"; the "Public Notice of Appropriate Use" asks clients to know the refresh cycle and to retry a failed service no more often than every minute. No NWPS-specific terms page was found (`water.noaa.gov/about/disclaimer` and `/about/nwps` answered 404). The credit lines are the `dataAttribution` objects in the data (see the field table).
