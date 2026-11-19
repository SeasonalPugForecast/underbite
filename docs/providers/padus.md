# PAD-US (USGS Protected Areas Database), public access layer

Verified on 2026-11-17 (UTC) with `curl` against the ArcGIS Online feature service and the PAD-US data dictionary (`PADUS_Standard_Tables_1.xlsx`, linked from the USGS PAD-US Data Manual page). An earlier survey is in `docs/providers/tier2.md`. Fixtures: `tests/fixtures/raw/padus/` (index in its `README.md`).

## Endpoint

| Item | Value |
|---|---|
| Service | `https://services.arcgis.com/v01gqwM5QqNysAAi/arcgis/rest/services/PADUS_Public_Access/FeatureServer` |
| Layer | `0` "PAD-US Public Access", polygons (`esriGeometryPolygon`) |
| Item | `https://www.arcgis.com/sharing/rest/content/items/c91a5655a1be428daeb778888e60db24` (last modified 2026-07-30) |
| Authentication | none |
| Formats | `f=json`, `f=geojson`, `f=pbf` |
| `maxRecordCount` | 2000 |
| Query capabilities | pagination (`resultOffset`, `resultRecordCount`), `orderByFields`, statistics, distinct, `returnCountOnly`, queries with distance, `maxAllowableOffset` and `geometryPrecision` |

No `User-Agent` is required; the requests here sent `underbite-scout/0.1`.

## Rate limits

No numeric limit is documented for the service. Every response carries Esri's usage headers, for example `x-esri-org-request-units-per-min: usage=43;max=28800` and `x-esri-query-request-units: 2`; the `max` is the organisation-wide budget for the hosting account, not a per-client limit. Responses are cacheable for 30 seconds (`cache-control: public, max-age=30`). One query for 100 rows without geometry over a box of 0.04 x 0.06 degrees timed out at 60 s once (HTTP status 000) and succeeded in the next call with a longer timeout, so allow at least 90 s and retry once. A whole-layer statistics query (`groupByFieldsForStatistics=Pub_Access` over the United States) did not finish within 120 s; do not run unbounded aggregations.

## `Pub_Access` codes

The data dictionary, Table 14 "Public Access domain codes and descriptions" (read 2026-11-17), defines four codes:

| Code | Description in the dictionary | Notes |
|---|---|---|
| `OA` | Open Access | open to the public; the dictionary adds no further definition |
| `RA` | Restricted Access | the service description gives "permit, seasonal" as examples |
| `XA` | Closed | no public access |
| `UK` | Unknown | access not reviewed; the service description says unknown areas are not displayed on the web map, but the feature service does return them (11 of 35 features near Croton) |

The layer defines no coded-value domain (`domain: null` on the field), so the codes arrive as the two-letter strings and the labels must be held in the app. Table 15 of the dictionary assigns a default access code per designation type; the service description warns that "not all areas have been locally reviewed", so show the code as "access: open (per PAD-US)" and not as legal permission. Fishing may still be prohibited or need a licence within `OA` land.

Counts in the Croton envelope (35 features): `OA` 21, `UK` 11, `RA` 2, `XA` 1; category `Fee` 32, `Easement` 3.

## Request the app needs

Public-access polygons around a spot, as an envelope in degrees, attributes only:

```text
GET /arcgis/rest/services/PADUS_Public_Access/FeatureServer/0/query
    ?geometry=-73.90,41.17,-73.86,41.23&geometryType=esriGeometryEnvelope&inSR=4326
    &spatialRel=esriSpatialRelIntersects&outSR=4326
    &outFields=Unit_Nm,Pub_Access,GAP_Sts,Category,DesTp_Desc,MngNm_Desc,GIS_Acres
    &orderByFields=OBJECTID&returnGeometry=false&resultRecordCount=40&f=json
```

Fixture `padus-croton-attrs-2026-11-17.json`: 35 rows, `exceededTransferLimit` absent because 35 is below `resultRecordCount`.

For a point, query a small envelope (a single-point `esriGeometryPoint` at Croton Point returned no features on 2026-11-08; a polygon contains a point only if the point is inside it).

### Paging

`resultOffset` and `resultRecordCount` work together with `orderByFields=OBJECTID` (without an order the page boundaries are not guaranteed). When `exceededTransferLimit` is `true` in the JSON response, request the next page at `resultOffset += resultRecordCount`. The server limit is `maxRecordCount` 2000; behaviour above that, and the name of the continuation flag in `f=geojson` output, were not tested (**unverified**), so page until a response returns fewer rows than requested.

### Polygon size and generalisation

`maxAllowableOffset` (in the units of `outSR`, degrees for 4326) simplifies geometry on the server, and `geometryPrecision` limits decimals. For the first five features in the Croton envelope (`f=geojson`):

| Parameters | Response size | Vertices per feature |
|---|---|---|
| none (full geometry, `outSR=4326`) | 9,965 bytes | 12, 23, 131, 4, 63 |
| `maxAllowableOffset=0.0005&geometryPrecision=5` | 2,746 bytes | 6, 7, 37, 4, 11 |

0.0005 degrees is about 55 m of latitude. Fixture: `padus-croton-geojson-offset0.0005-5rows-2026-11-17.json`. Use a coarser offset for map overlays at low zoom, and request geometry only for the polygons that must be drawn, since large parks are heavy: five features from offset 3 in the same envelope were 28,268 bytes at the same generalisation.

## Fields

| Field | Type | Meaning |
|---|---|---|
| `Unit_Nm` | string | unit name (sometimes upper case, sometimes with the owner type appended: "VILLAGE OF CROTON-ON-HUDS Land") |
| `Pub_Access` | string | `OA`, `RA`, `XA` or `UK` |
| `GAP_Sts` | string | GAP status code "1" to "4": 1 managed for biodiversity with disturbance events proceeding, 2 the same with disturbance suppressed, 3 managed for multiple uses (extractive or off-highway vehicle use), 4 no known mandate for biodiversity protection (dictionary Table 10) |
| `Category` | string | `Fee`, `Easement`, `Designation`, `Marine`, `Proclamation`, `Other` (dictionary Table 3) |
| `FeatClass` | string | source feature class |
| `DesTp_Desc` | string | designation type ("Local Park", "State Conservation Area") |
| `MngTp_Desc`, `MngNm_Desc` | string | manager type and name ("County Land", "State Department of Conservation") |
| `IUCN_Cat` | string | IUCN category |
| `BndryName`, `BndryExten`, `BndryID` | string | boundary source |
| `GIS_Acres`, `GIS_AcrsDb` | integer, double | area; `GIS_Acres` is rounded, so small parcels show 0 (the Croton Village Lands row) |
| `ST_Name` | string | state (512 characters declared) |

Geometry is Web Mercator (wkid 102100) unless `outSR=4326` is requested. Missing values are empty strings or null.

## License and attribution

Public-domain US government data. The item's license text is a no-warranty disclaimer that points to the PAD-US Data Overview. USGS asks for the citation "U.S. Geological Survey (USGS) Gap Analysis Project (GAP), 2024, Protected Areas Database of the United States (PAD-US) 4.1, https://doi.org/10.5066/P96WBCHS". Check the version number against the item description when the service is updated. The USGS Data Manual page has no separate terms-of-use section.

Re-checked 2026-11-18: unchanged. The ArcGIS item license text is the same no-warranty disclaimer (item modified 2026-07-30); the PAD-US Data Overview (`https://www.usgs.gov/programs/gap-analysis-project/science/pad-us-data-overview`) still gives "PAD-US 4.1", 2024, doi:10.5066/P96WBCHS as the citation, and the data history page says version 4.1, released March 2025, is the most recent.

## Unverified

- A description of each code beyond the dictionary's one-line labels: the "Data Overview" and the metadata XML were not read.
- Whether `UK` features are intended to be shown. They are returned by the feature service.
- A numeric per-client rate limit for ArcGIS Online (none published for this service).
