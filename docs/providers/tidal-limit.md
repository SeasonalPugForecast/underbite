# Tidal limit of a river or estuary (is this point tidal?)

Verified on 2026-11-20 with `curl`, no key, `User-Agent: underbite-scout/0.1`. Recorded responses are in
`tests/fixtures/raw/tidal-limit/` (index: `tests/fixtures/raw/tidal-limit/README.md`, which gives the exact
URL of each file). Anything not checked is marked **unverified**.

The question: is a point on a US river or estuary tidal, that is, below the head of tide? The 3DHP flowline
and waterbody layers (see `hydro.md`) cannot say, so four other sources were checked.

## Summary

| Source | Answers "tidal here?" | Verdict |
|---|---|---|
| USFWS National Wetlands Inventory (NWI), ArcGIS REST | Yes: code `R1...` (Riverine, Tidal subsystem) or `E...` (Estuarine) | Best fit. Correct at all five test points and at the Troy dam. |
| NOAA VDatum web API | Yes where it returns a value (coverage ends near the head of tide on the Hudson) | Correct on the Hudson and the Croton; **fails with error 412 on the whole Delaware**. Not documented as a service. |
| NHD (`nhd` MapServer), FType 493 Estuary | Only for the lower estuary | Hudson estuary polygons stop at Poughkeepsie (41.703 N). Wrong at Albany and Trenton. |
| 3DHP (`3DHP_all`) | No | No estuary or tidal type exists. |
| CO-OPS station metadata | Only as a distance heuristic | Prediction stations exist up to Troy (Hudson) and Trenton (Delaware); no polygon or flag. |
| NOAA CUSP, Medium Resolution Shoreline | **Unverified** | Downloads only, no service found (see below). |

Test points (2-decimal, as the app sends them) and the answer of each source. "T" is tidal, "N" is not.

| Point | Truth | NWI | VDatum | NHD 493 polygon in box | CO-OPS |
|---|---|---|---|---|---|
| Lower Hudson 40.75,-74.01 | T | `E1UBL` (estuarine) = T | -0.842 m = T | yes = T | Battery 8518750 = T |
| Poughkeepsie 41.70,-73.94 | T | `R1UBV` + `E1UBL6` = T | -0.51 m = T | yes (edge of the polygon) = T | 8518945 = T |
| Albany 42.65,-73.75 | T | `R1UBV`, `R1UBVx` = T | -0.559 m = T | **no (390 Lake/Pond only) = wrong** | 8518995 = T |
| Croton gauge 41.20,-73.86 | N | no `R1`/`E` code (`R3UBH`, `R5UBH` and `P*`) = N | -999999 = N | no = N | none near |
| Delaware at Trenton 40.22,-74.77 | T | `R1UBV` = T | **error 412** | **no (436 Reservoir) = wrong** | Trenton 8539993 = T |

Extra points that test the limit: Hudson just below the Troy dam (42.75,-73.69): NWI `R1UBV`, `R1UBVx`
and VDatum -0.57 m (tidal). Just above it (42.80,-73.69): no `R1` in NWI and VDatum -999999 (not tidal).
Croton mouth in Croton Bay (41.19,-73.89): NWI `E1UBL6`, `E2EM5P6` (estuarine) and VDatum -0.545 m (tidal).

## 1. USFWS National Wetlands Inventory (recommended)

- Base: `https://fwspublicservices.wim.usgs.gov/wetlandsmapservice/rest/services/Wetlands/MapServer`
  (ArcGIS Server `currentVersion` 11.5, IIS). Layer 0 `Wetlands` (polygons, `maxRecordCount` 500,
  `minScale` 250000, which does not apply to `query`), table 1 `NWI_Wetland_Codes`. The page
  `https://www.fws.gov/wetlands/data/web-map-services.html` names this REST root and says the services are
  "updated biannually along with the Wetlands Mapper". (An older host, `fwsprimary.wim.usgs.gov/wetlandsmapservice`,
  answers 404; use the host above.)
- No key, no account. No rate-limit headers (a `Set-Cookie: cookiesession1` is sent and not needed). No limits are published.
- Fields of layer 0: `ATTRIBUTE` (Cowardin code, up to 15 characters), `WETLAND_TYPE` (`Riverine`,
  `Estuarine and Marine Deepwater`, `Freshwater Pond` ...), `ACRES`, `QAQC_CODE`.
- The code's second character is the subsystem. For `R` (Riverine) subsystem `1` is **Tidal**: table 1 says
  it "extends from the upstream limit of tidal fluctuations down to the upper boundary of the Estuarine
  System, where the concentration of ocean-derived salts reaches 0.5 ppt during the period of average annual
  low flow". The fifth character `V` is "Permanently Flooded-Tidal" (subgroup "Freshwater Tidal"). Subsystems 2 to 5
  (`R2` lower perennial, `R3` upper perennial, `R4` intermittent, `R5` unknown perennial) are not tidal.
  Codes starting with `E` (Estuarine) are tidal and saline; `E1` is subtidal, `E2` intertidal.
  Fixture `nwi-1-code-table-r1ubv.json`.
- **Rule**: a point is tidal when any polygon within a small search distance has an `ATTRIBUTE` starting with
  `R1` (freshwater tidal river) or `E1`/`E2` (estuarine). Palustrine (`P`) tidal water regimes were not
  checked; do not use them.

### Requests

Envelope (works with a 2-decimal rounded point plus or minus 0.01 degree; response 0.5 to 5 KB in 0.7 to 1.7 s):

```text
GET /0/query?where=1=1&geometry=-73.76,42.64,-73.74,42.66&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=ATTRIBUTE,WETLAND_TYPE,ACRES&returnGeometry=false&f=json
```

Smaller, filtered form (369 bytes in 0.2 s; fixture `nwi-0-albany-point-distance500-r1.json`):

```text
GET /0/query?where=ATTRIBUTE LIKE 'R1%'&geometry=-73.75,42.65&geometryType=esriGeometryPoint&inSR=4326&distance=500&units=esriSRUnit_Meter&outFields=ATTRIBUTE,WETLAND_TYPE&returnGeometry=false&f=json
```

- Response `{"features":[{"attributes":{"ATTRIBUTE":"R1UBV","WETLAND_TYPE":"Riverine"}}]}`; an empty `features` array
  means no polygon matched. Errors may be HTTP 200 with an `error` object (as with the other ArcGIS services; check the body).
- A **bare point** (no `distance`) at the rounded Albany point matched nothing: the rounded point falls on
  the river edge or a bank. Use an envelope or a `distance` of 500 m.
- **A `where` with `OR` is blocked**: `ATTRIBUTE LIKE 'R1%' OR ATTRIBUTE LIKE 'E1%' OR ...` returned an HTML page "The URL
  you requested has been blocked" (a web firewall, HTTP status not recorded). A single `LIKE`, `=` or `1=1` worked. Query
  with `1=1` and classify the codes in the core, or send one `LIKE` per prefix.
- Envelope result sizes: 5 to 40 polygons, up to 5 KB without geometry (more on land with many wetlands).
  Never request geometry for this use.
- Result counts stay far below 500 for a 0.02 degree box; `exceededTransferLimit` is **unverified** at larger boxes.

### Evidence

| Box (rounded point +-0.01) | Codes (count) | Size |
|---|---|---|
| Battery | `E1UBL` | 496 B |
| Poughkeepsie | `R1UBV`, `R3UBH`, `E1UBL6` | 677 B |
| Albany | `R1UBV`, `R1UBVx`, `R3UBH` (2), `R5UBH` (3), `R4SBC`, `PUBHh`, `PUBHx` | 1.3 KB |
| Croton gauge | `R3UBH` (3), `R5UBH` (9), `R4SBC`, `P*` | 3.5 KB |
| Trenton | `R1UBV`, `R1USQ`, `R2UBF` (2), `R2UBHx` (18), `R2EM2F`, `R5UBH` (7) ... | 3.7 KB |
| Troy below the dam | `R1UBV`, `R1UBVx`, `R3UBH`, `R5UBH` (13) ... | 4.4 KB |
| Troy above the dam | no `R1`; `R3UBH`, `R4SBC`, `R5UBH` (15) ... | 5.0 KB |
| Croton mouth | `E1UBL6`, `E2EM5P6` (5), `R5UBH` ... | 2.1 KB |

Fixtures `nwi-0-<place>-box.json`. The tidal Hudson ends at the Troy dam in this data, as expected.

### Limits and caveats

- A box also catches tidal tributaries and creeks that join the river (`R1` on a creek mouth). That is correct
  for "water here is tidal" only if the point is on that water; an envelope around a river point may include a
  tidal creek nearby. Prefer the polygon nearest the point (needs geometry) or a smaller `distance`.
- Boundaries and codes come from aerial imagery of mixed ages ("check the metadata for the date of the source
  imagery"); the data are mapped with a biological definition and not for regulatory use. A reach can be coded
  `R2` or `R3` just above a real limit, and tidal freshwater is not detected on every river.
- Coverage is the wetland data layer; areas not yet mapped have no polygons. An empty result is therefore
  "unknown", not "not tidal".
- Seagrass and other subtidal vegetation are excluded; irrelevant for this question.
- Terms: the pages searched state no licence and no required attribution text. The Service's disclaimers say the
  data are not for regulatory use and a trade-name mention is not an endorsement. For the current release the Service
  gives the citation "United States Fish and Wildlife Service (2026), National Wetlands Inventory",
  DOI 10.7944/usfws.nwi.202605 (general DOI `https://doi.org/10.7944/USFWS.NWI`). Credit "U.S. Fish and Wildlife
  Service, National Wetlands Inventory". The rights status ("public domain") is **unverified**; ask
  `wetlands_team@fws.gov` if a licence statement is needed.

## 2. NOAA VDatum web API

- Endpoint: `https://vdatum.noaa.gov/vdatumweb/api/convert` (VDatum 4.8 web page). Documented at
  `https://vdatum.noaa.gov/docs/services.html`: required `s_x`, `s_y`, optional `s_z` (default 0), `region`
  (default `contiguous`), frames `s_h_frame`, `s_v_frame`, `t_v_frame`, geoid `geoid18` and others. No key.
- Request used: convert height 0 from `MLLW` to `NAVD88` (and `LMSL`) at the point:

```text
GET /vdatumweb/api/convert?s_x=-73.75&s_y=42.65&s_z=0&s_h_frame=NAD83_2011&s_coor=geo&s_v_frame=MLLW&s_v_geoid=GEOID18&t_h_frame=NAD83_2011&t_coor=geo&t_v_frame=NAVD88&t_v_geoid=GEOID18
```

- Response (about 450 bytes, JSON): `region`, the echoed inputs, `t_x`, `t_y`, `t_z` (metres) and `uncertainty`.
  Inside the tidal coverage `t_z` is a number (the Battery -0.842 m, Poughkeepsie -0.51, Albany -0.559,
  Troy dam below -0.57, Croton mouth -0.545). **Outside the tidal model the response is still HTTP 200 with
  `"t_z":"-999999"`** (Croton gauge, Hudson above the Troy dam, a bank point near Albany). The `-999999` value and the
  error code are not documented on the page.
- Evidence of the head of tide: on the Hudson the tidal MLLW offset exists up to the Troy dam and not above it, which
  matches the NWI result. The Croton gauge gives -999999 (not tidal).
- **The Delaware fails**: Trenton (40.22,-74.77), (40.21,-74.75), (40.20,-74.74), (40.23,-74.77), Philadelphia (39.95,-75.14),
  Burlington (40.10,-74.85) and (40.17,-74.80) all return `{"errorCode":412,"message":"Uncaught error, please contact NOAA
  VDatum Program Support team."}`, also for MLLW to LMSL. Whether that is a standing fault or a coverage gap is
  **unverified**. (40.26,-74.79 above Trenton returned -999999.) The service therefore cannot be used alone.
- The same call is rate-limited by nothing observed (no headers); each call took 1 to 4 s. No limits or terms are published on
  the pages read; the page links "Online Conversion" and NGS disclaimers whose text was not retrieved (**unverified**).
  The API is the one the VDatum web application uses; the page does not promise availability.
- It is a datum tool: it says "inside the tidal model", which hugs the water. A 2-decimal rounded point can fall on
  land and answer -999999 on a narrow river. Coverage on a river is therefore a necessary but unreliable signal.
- Region polygons (a coverage map): no ArcGIS REST layer was found. A search of public sources showed only unrelated NOAA layers
  (VPASS polygons, an Alaska proposal); **unverified** whether one exists.

## 3. NHD (National Hydrography Dataset, `nhd` MapServer)

- Base `https://hydro.nationalmap.gov/arcgis/rest/services/nhd/MapServer` (ArcGIS 11.3, same host and terms as
  `hydro.md`). Layers: 6 Flowline - Large Scale, 9 Area - Large Scale, 12 Waterbody - Large Scale (fields `FTYPE`,
  `FCODE`, `GNIS_NAME`, `AREASQKM`, `REACHCODE`). `NHDPlus_HR` is also served (layers 3 NetworkNHDFlowline, 9 NHDWaterbody).
- FType **493 Estuary** (FCode 49300) exists in layer 12 and marks the tidal estuary water. A 2-decimal box query works
  (about 2 KB without geometry). In the Hudson region it holds 17 polygons (Raritan Bay, Upper Bay, Long Island Sound,
  Jamaica Bay and so on) whose union extends **north only to 41.7033 N** (extent query, fixture
  `nhd-12-ftype493-hudson-region-extent.json`): polygon 146125933 (53 km2) spans 41.320 to 41.703 N.
  Above it the Hudson is River (FType 558 flowline `Artificial Path`, 460 `Stream/River`) or in the polygon layer
  `390 Lake/Pond` and a tiny polygon; at Albany no estuary polygon, and at Trenton a 436 Reservoir sliver.
- Flowline FTypes seen (names from the NHD domain as remembered, the numbers are from the responses): 558 Artificial Path, 566 Coastline, 460 Stream/River, 334 Connector, 428 Pipeline. FCode 46006
  is "Stream/River: perennial", 55800 Artificial Path, 56600 Coastline. None marks tidal reaches; there is no tidal FCode
  in the data returned. `Coastline` (566) lines appear along the estuary in the lower Hudson box. The layers
  `Sea/Ocean` (FType 483) and the full domain list were not queried (**unverified**).
- Verdict: 493 gives "estuary" for the lower Hudson and Poughkeepsie only; it does not extend to the head of tide.
- 3DHP (`3DHP_all`): layer 60 `featuretype` values are `1 River`, `2 Canal`, `3 Lake`, `4 Ocean or Great Lake`
  (group-by statistic over the whole layer: 85220, 9235, 7063520, 968 polygons). No estuary or tidal value. Layer 50 has
  no tidal attribute (fields in `hydro.md`). ArcGIS Server has no `queryables` endpoint (that is an OGC API Features
  concept); the field list of `/50?f=json` and `/60?f=json` is the schema.

## 4. CO-OPS station metadata (fallback)

- `https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations.json?type=tidepredictions&state=NY&units=metric`
  (2.0 MB for New York; not recorded in full). Each station has `lat`, `lng`, `name`, a `type` (`R` reference, `S`
  subordinate) and `tidal` (true). Per-station form `/stations/<id>.json?expand=details` is 0.6 to 3 KB (fixtures
  `coops-mdapi-station-*-details.json`).
- Tide-prediction stations reach the head of tide on both rivers: Hudson 8518945 Poughkeepsie (41.70), 8518995 Albany
  (42.65; `removed` 1987-07-25, still has predictions), TEC1603 Troy (42.717,-73.70); Delaware 8539993 Trenton
  Marine Terminal (40.1883,-74.755; `removed` 1992-10-31), 8548989 Newbold, 8539487 Fieldsboro. The Croton River has none.
- This only supports a rule such as "a tide-prediction station within a few km on the same water". It cannot say where
  the limit is between stations and is not a polygon, so it is a fallback. The active water-level stations on the Hudson
  (type `waterlevels`) are only 8518750, 8518962, 8518979 and 8516945.
- Terms: see `coops.md`.

## 5. NOAA shoreline products (CUSP, Medium Resolution Shoreline)

**Unverified.** Both are shoreline line downloads (shapefile or geodatabase through the NOAA shoreline site), not point-query
services, and no REST endpoint was found. Searches for a national head-of-tide dataset found only state layers
(Maine DEP `MaineHeadOfTide` MapServer; Oregon Water Resources Department `Hydro_Head_of_Tide`; a Washington Coastal Atlas
layer) and an academic study of tidal rivers of the continental United States (Ohio State University) with no download
link. State layers are not national and were not tested.

## Recommendation

Use NWI as the signal and keep CO-OPS station proximity as the current fallback:

1. Query NWI layer 0 with a 2-decimal rounded envelope (+-0.01 degree) or a 500 m point search, no geometry.
2. Tidal when any `ATTRIBUTE` starts with `R1` (freshwater tidal river) or `E` (estuarine); not tidal when the
   result has polygons but none of those; unknown when `features` is empty or the request fails.
3. If NWI is unknown or fails, keep the present rule (a tide-station pin on screen near the river).

VDatum agrees with NWI at every point except the Delaware, where it errors, and it has no published limits, so use
it only as a cross-check in tests, not as a data source. NHD 493 and 3DHP are not usable for the head of tide.

## Not verified

- NWI response headers for rate limiting and the behaviour above any limit; the licence and attribution text beyond the
  citation above; `exceededTransferLimit` on large boxes; whether the service answers identically from another host.
- NWI on other rivers (only the five test points, the Troy dam and the Croton mouth were queried); tidal freshwater
  reaches mapped as `R2` or `R3`.
- VDatum: the cause of error 412 on the Delaware, the meaning of codes, rate limits and terms, whether other regions
  (Gulf, West Coast, Alaska) differ.
- NHD flowline `Sea/Ocean` FType and the FCode domain; NOAA CUSP; state head-of-tide layers.
