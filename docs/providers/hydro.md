# USGS 3DHP hydrography (National Map)

Verified on 2026-11-15 against the live service, with `curl` and no key. Recorded responses are in
`tests/fixtures/raw/hydro/` (index: `tests/fixtures/raw/hydro/README.md`). Anything not checked is
marked **unverified**. An earlier survey of the service is in `docs/providers/tier2.md`.

## Summary

- Base: `https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/MapServer` (ArcGIS Server
  `currentVersion` 11.3, nginx behind CloudFront). Layer 50 is **Flowline** (polyline), layer 60 is
  **Waterbody** (polygon). Both report `maxRecordCount` 2500, capabilities `Map,Query,Data`, statistics
  supported, query formats JSON, geoJSON and PBF, and a display scale limit of 1:300000 that does not apply
  to `query`.
- No key and no account. No rate-limit headers (the response headers are `content-type`, `etag`,
  `cache-control: max-age=0,must-revalidate`, `x-cache`, CloudFront ids and `access-control-allow-origin: *`).
  No numbers are published; send a `User-Agent` and keep requests small and few.
- Terms (service description, see `tier2.md`): "Use Constraints: None. All data are open and
  non-proprietary"; not for critical applications or site-specific regulatory determinations;
  "Acknowledgment of the U.S. Geological Survey would be appreciated." Credit line: "USGS TNM / NGTOC - 3D
  National Hydrographic Program (3DHP.)".
- The response body is JSON (`f=json`). **Errors come back with HTTP 200** and an `error` object (see
  "Errors"), so always check for a top-level `error` key.
- Query time is very different by kind: a spatial query (envelope, or point with `distance`) answered in
  0.3 to 3 seconds; an attribute-only `where` over the whole layer (`levelpath=...`, `gnisid=...`, a
  `like` on the name) took 25 to 50 seconds in these tests. **The app should always send a small
  geometry together with any `where`**, except for lookups that are not time critical.

## Requests the app needs

Parameters are the ArcGIS REST query parameters. Quote URLs in a shell. Coordinates below are test
places rounded to 2 decimals, as the app sends them.

| Need | Request (path `/50/query` or `/60/query`) |
|---|---|
| Named flowlines in a box | `GET /50/query?where=gnisidlabel IS NOT NULL&geometry=-73.87,41.20,-73.84,41.23&geometryType=esriGeometryEnvelope&inSR=4326&outSR=4326&spatialRel=esriSpatialRelIntersects&outFields=id3dhp,gnisidlabel,featuretypelabel,lengthkm,streamorder,levelpath,hydrosequence,uphydrosequence,dnhydrosequence,mainstemid&returnGeometry=true&geometryPrecision=5&f=json` (21 features, 62 KB) |
| Flowlines near a point | `GET /50/query?where=1=1&geometry=-73.86,41.23&geometryType=esriGeometryPoint&inSR=4326&spatialRel=esriSpatialRelIntersects&distance=500&units=esriSRUnit_Meter&outFields=...&returnGeometry=false&f=json` |
| Walk a level path | `GET /50/query?where=levelpath=3767999 AND hydrosequence<=4815843&outFields=...&returnGeometry=true&outSR=4326&geometryPrecision=4&orderByFields=hydrosequence ASC&f=json` |
| Waterbodies near a point | `GET /60/query?where=1=1&geometry=-73.84,41.23&geometryType=esriGeometryPoint&inSR=4326&spatialRel=esriSpatialRelIntersects&distance=500&units=esriSRUnit_Meter&outFields=id3dhp,gnisid,gnisidlabel,featuretype,featuretypelabel,areasqkm,mainstemid&returnGeometry=false&f=json` |
| One waterbody outline | `GET /60/query?where=id3dhp='IZTBN'&outFields=...&returnGeometry=true&outSR=4326&geometryPrecision=4&maxAllowableOffset=0.0005&f=json` |
| Page | add `orderByFields=hydrosequence ASC&resultOffset=3&resultRecordCount=3` |
| Count | `returnCountOnly=true` returns `{"count":138}` |

Observed behaviour:

- `geometry` with `geometryType=esriGeometryEnvelope` takes `xmin,ymin,xmax,ymax` (longitude first) in the
  `inSR` system. A point with `distance` and `units=esriSRUnit_Meter` works (500 m around -73.86,41.23
  matched 11 flowlines, fixture `hydro-50-croton-point-distance.json`). **A point without `distance`
  against a polygon layer matched nothing at the USGS gauge locations** (the gauge points are on a dam or
  outside the polygon); use a search distance.
- **Always send `outSR=4326`.** Without it the geometry comes back in Web Mercator (`spatialReference`
  `wkid` 102100 / `latestWkid` 3857, metres) even when `inSR=4326`
  (fixture `hydro-no-outsr-and-bad-geometry-count2.json`).
- `geometryPrecision=4` or `5` keeps coordinates short; with `geometryPrecision=4` a 47-feature, 24.8 km
  stretch of the Croton River is 29 KB. Polylines come back as `geometry.paths` (an array of paths of
  `[lon, lat]`), polygons as `geometry.rings`.
- Polygons can be large: New Croton Reservoir (8.7 km2) with `geometryPrecision=4` is 901 KB (one ring with
  45343 vertices); `maxAllowableOffset=0.0005` (degrees when `outSR=4326`) reduced it to 212 vertices and 5 KB.
  Use a generalisation offset for any outline the app draws.
- Field values are not trimmed: `lengthkm` shows float noise such as `0.9109025500000001`; round in
  the core.

## Layer 50 Flowline: fields

| Field | Type | Meaning, as observed |
|---|---|---|
| `id3dhp` | string | feature id, 1 to 7 characters of `0-9A-Z` (`1Y1PM`, `GLPE`) |
| `gnisid`, `gnisidlabel` | integer, string | GNIS id and name; `gnisidlabel` is null for most small streams |
| `featuretype`, `featuretypelabel` | small int, string | `Channel Line`, `Waterbody Connector` (path through a lake or estuary), `Surface Connector` |
| `lengthkm` | double | length of this reach in kilometres |
| `streamorder` | integer | Strahler order: 7 for the lower Croton River, 2 for Purdy Brook, 8 to 9 for the Hudson estuary |
| `levelpath` | integer | id of the main path this reach belongs to (see below) |
| `hydrosequence` | double | position in the downstream ordering (see below) |
| `dnhydrosequence` | double | `hydrosequence` of the reach immediately downstream; **0** at the outlet |
| `uphydrosequence` | double | `hydrosequence` of the main-path reach immediately upstream; **0** at a headwater |
| `dnlevelpath`, `uplevelpath` | double | level path downstream of / upstream of this one at the reach |
| `terminalpath` | double | `levelpath` of the path that ends at the sea or a sink (44 for the Hudson and everything that drains to it) |
| `pathlength` | double | kilometres from the reach to the terminal outlet (118.9 for a Croton reach near the mouth, 121.1 for a reach just upstream) |
| `startflag`, `terminalflag` | small int | 1 at a headwater / at an outlet |
| `flowdirection`, `flowdirectionlabel` | small int, string | 1: "Flow direction is in digitized direction, and z-values on vertices flow downslope" |
| `divergence`, `divergencelabel`, `rtrndivergence` | small int, string | "No divergence" for ordinary reaches |
| `mainstemid` | string | geoconnex.us URI of the mainstem |
| `featuredate` | date | epoch milliseconds |
| `waterbodyid3dhp`, `catchmentid3dhp`, `flowpathid3dhp`, `workunitid`, `streamlevel`, `streamcalculator`, `arbolatesum` | various | present in the layer, not used here |

### Upstream and downstream

- **`hydrosequence` grows going upstream.** For every reach `dnhydrosequence < hydrosequence <
  uphydrosequence` (or `uphydrosequence` is 0). Sorting by `hydrosequence` ascending lists a path from its
  outlet to its source. In the 138 reaches of the Croton River level path no neighbour link was
  inconsistent (`next.up == this.hydrosequence` for every pair), and the first reach's `hydrosequence`
  equals the `levelpath` id (3767999). Reaches are therefore linked by value, not by adjacency: the next
  reach downstream is the feature whose `hydrosequence` equals `dnhydrosequence`, the next upstream is
  the one equal to `uphydrosequence`.
- A tributary joins the main path where its `dnhydrosequence` points into the other path: Purdy Brook's
  last reach (`dnhydrosequence` 4171913) lands on a Croton River reach; the Croton River's first
  reach points to 3743109, which is on the Hudson path (44), not in the Croton path.
- Digitised direction is the flow direction (`flowdirection` 1): the first vertex of a reach is its
  upstream end. Checked on three reaches (reach 3767999 runs from -73.8839,41.1827 to -73.8966,41.1526,
  toward the Hudson; reach 3792919 ends where 3767999 starts).
- **Level path** `levelpath` equals the `hydrosequence` of the downstream-most reach of that path. The
  Croton River main stem is level path 3767999: 138 reaches, 82.3 km in total, `hydrosequence` 3767999
  to 6884337; its upper part is named "East Branch Croton River".
- **A 25 km stretch** from the Croton mouth upstream is 47 reaches (`hydrosequence <= 4815843`, 24.82
  km); this response is 29 KB with geometry at precision 4. The whole 82 km path has 138 reaches, below the
  2500 limit, so one request returns it. Walking by `levelpath` returns 33 KB without geometry. A
  generic walker should still read `exceededTransferLimit`.
- Cumulative length of the first reaches (km): 3.53, 7.56, 12.04 ... 20.15 (reach 37), 25.39 (reach 48).

### Estuary: The Battery

Fixtures `hydro-50-battery-envelope.json`, `hydro-60-battery-envelope.json`,
`hydro-50-hudson-levelpath-44-mouth.json` (envelope -74.03,40.68,-73.99,40.72).

- Layer 50 has four reaches there: the Hudson River (level path 44, `Waterbody Connector`, stream order 8)
  and an unnamed order-2 path 2964792. Flowlines in the estuary are artificial connectors through the
  water, not channels, so they carry no tidal information.
- Level path 44 ends at `hydrosequence` 44 with `dnhydrosequence` 0, `terminalpath` 44. Going downstream
  from The Battery it runs through **Arthur Kill** (named reaches) and not through the Narrows: the
  3DHP "terminal path" of the Hudson goes out through Raritan Bay. Do not infer the open sea or the
  direction of ebb from it.
- Layer 60 has four polygons in the box: "Upper Bay" (`featuretypelabel` "River", 43.07 km2),
  an unnamed "River" polygon of 175.6 km2 and two unnamed tiny "Lake" polygons (0.0004 and 0.0013 km2).
  Estuary water is therefore typed "River", and the big polygons may be unnamed.

## Layer 60 Waterbody: fields

| Field | Type | Meaning |
|---|---|---|
| `id3dhp` | string | feature id |
| `gnisid`, `gnisidlabel` | integer, string | GNIS id and name (null for unnamed ponds) |
| `featuretype`, `featuretypelabel` | small int, string | 3 `Lake`; `River` for tidal river polygons. Other values (`Reservoir`, `Estuary` and so on) **unverified** |
| `areasqkm` | double | area in km2 (0 for slivers, such as `KZZHJ`) |
| `mainstemid` | string | geoconnex URI, may be null |
| `shape_Length`, `shape_Area` | double | in the layer's own units; `shape_Area` is square metres of the Web Mercator projection, so it overstates real area at this latitude. Use `areasqkm`. |
| `featuredate`, `workunitid` | date, string | not used |

- New Croton Reservoir: one polygon `IZTBN`, `gnisid` 958407, "Lake", 8.74 km2, extent
  -73.860,41.215 to -73.709,41.264, five rings (one outer, four islands or parts). A point query at
  -73.84,41.23 with `distance=500` returns it. The point -73.78,41.27 (outside) returns an empty `features`
  array.
- Great Sacandaga Lake: **four** polygons share `gnisid` 973352 (83.57, 8.93, 6.40 and 1.90 km2). A
  500 m or 1000 m search around the gauge `USGS-01323500` (rounded -73.93,43.32) returned the 6.40 and
  1.90 km2 pieces and one unnamed 0 km2 sliver, not the 83.57 km2 main body. Sum `areasqkm` over a
  `gnisid` for a lake area, and do not take a single polygon as the lake. The gauge sits at the dam, which
  is why a plain point does not fall inside the polygon.

## Paging

- `exceededTransferLimit: true` appears at the top level of the response when more rows exist than were
  returned (fixture `hydro-50-croton-paging-offset3-count3.json`: `resultOffset=3&resultRecordCount=3`
  returned three of 21 and the flag). The flag is absent when the result is complete.
- Add `orderByFields` for a stable order; without it the rows came back sorted by `id3dhp` in every test, which the service does not promise.
- `resultRecordCount` up to the service limit of 2500.

## Errors

- `where=nosuchfield=1` gives HTTP 200 and `{"error":{"code":400,"message":"Failed to execute query.","details":[]}}`.
- A missing layer (`/99/query`) gives HTTP 200 and `{"error":{"code":400,"message":"Invalid or missing input parameters.","details":[]}}`.
- **An unparsable `geometry` (`geometry=abc`) is ignored** and the query runs without a spatial filter: the
  default request returns up to 2500 features (3.3 MB in the test). Validate the envelope before sending.
- An empty result is `"features":[]`; it is not an error.

## USGS discovery queries around the Croton gauge (2026-11-15)

Box `-73.91,41.19,-73.81,41.29` (the gauge at 41.2250,-73.8589 rounded to 2 decimals, plus 0.05 degrees);
requests as in `docs/providers/usgs.md`, fixtures in `tests/fixtures/raw/usgs/`
(`usgs-monitoring-locations-bbox-croton-st.json`, `-lk.json`, `usgs-time-series-metadata-bbox-croton.json`).

- `monitoring-locations` with `site_type_code=ST`: 16 sites, from `USGS-01374390` (Furnace Brook) to the
  Croton gauge `USGS-01375000` ("CROTON R AT NEW CROTON DAM NR CROTON-ON-HUDSON NY") and Bird Brook; many
  are small partner stations at brook mouths. With `site_type_code=LK`: 3 sites, `USGS-01374995`
  (New Croton Reservoir) and two wells-like names (`WE  63`, `WE  64`). No `next` link in either.
- `time-series-metadata` with the same `bbox`: 12 series for four sites. Only `USGS-01375000` is live
  (`00065` and `00060` instantaneous, ending 2026-11-15, and a daily mean `00060`). `USGS-0137499350` ended
  in 2009; `USGS-01375500` ended in 1941 and its daily-mean series has `begin` and `end` **null**. A
  discovery parser must treat null `begin`/`end` as unknown and compare `end` with the current time to
  drop dead series. The series are listed in no particular order.

## Not verified

- Response headers for rate limiting: none observed; behaviour above any limit **unverified**.
- Other `featuretypelabel` values of layer 60 (a group-by statistic over the whole layer was not run).
- `f=geojson` and `f=pbf` output: listed in `supportedQueryFormats`, not requested.
- Whether the data are served identically from the `FeatureServer` endpoint.
- The data refresh date: the layer reports the "refreshed September 4, 2026" credit in `tier2.md`.
