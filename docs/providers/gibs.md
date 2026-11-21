# NASA GIBS (Global Imagery Browse Services)

Verified on 2026-11-08 and re-checked on 2026-11-16 (UTC; see "Re-check, 2026-11-16") with `curl` against `gibs.earthdata.nasa.gov` and the GIBS API documentation (`https://nasa-gibs.github.io/gibs-api-docs/`). Fixtures: `tests/fixtures/raw/gibs/` (index in its `README.md`).

## Endpoints

| Service | URL | Notes |
|---|---|---|
| WMTS capabilities, Web Mercator | `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/1.0.0/WMTSCapabilities.xml` | 5.8 MB, 1,332 layers, `cache-control: public, max-age=1800`. Do not ship or fetch it on a phone; hard-code the few layer ids and re-check them from a build-time script. |
| WMTS tiles (REST) | `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/{Layer}/default/{Time}/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}.{ext}` | `best` merges the best available product (science quality where it exists, otherwise near real time). `std` and `nrt` select one kind; `all` lists everything. |
| WMS, Web Mercator | `https://gibs.earthdata.nasa.gov/wms/epsg3857/best/wms.cgi?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS={Layer}&STYLES=&FORMAT=image/png&TRANSPARENT=true&CRS=EPSG:3857&BBOX={minx},{miny},{maxx},{maxy}&WIDTH=256&HEIGHT=256&TIME={date}` | Verified for `GHRSST_L4_MUR_Sea_Surface_Temperature`: 200, `image/png`, 2,874 bytes for a 256 x 256 box off New York. Versions 1.1.1 and 1.3.0, key-value requests only. |
| Colour map (value to RGB) | `https://gibs.earthdata.nasa.gov/colormaps/v1.3/{ColorMapName}.xml` | The name is in each layer's `ows:Metadata` entry; for MUR SST it is `GHRSST_Sea_Surface_Temperature` (not the layer id), for the NOAA-20 chlorophyll layer `VIIRS_Chlorophyll`. Fixture: the SST map. |
| Legends | `https://gibs.earthdata.nasa.gov/legends/{Name}_H.svg` and `_V.svg` | Listed in the capabilities (`LegendURL`). |

No key, account or header is required beyond a normal `User-Agent`. Responses carry `access-control-allow-origin: *`.

## Tile addressing (verified)

- Tile size 256 x 256, EPSG:3857, origin at the top left of the standard Web Mercator world square, so a GIBS tile is the usual XYZ tile: `{TileMatrix}` = zoom, `{TileRow}` = y, `{TileCol}` = x, with y counted from the north. The order in the URL is **row before column**: `.../7/48/37.png` is x = 37, y = 48.
- TileMatrixSet name sets the maximum zoom: `GoogleMapsCompatible_Level7` gives zoom 0 to 7, `_Level9` gives 0 to 9 (matrix 9 is 512 x 512 tiles, scale denominator 1,091,957.5). Asking for a deeper zoom than the set returns `400`: MUR SST at zoom 8 under `_Level7` answered `400 text/html`. For `MKTileOverlay`, set the source's maximum zoom per layer and over-zoom in the overlay itself: on iOS 27.0 simulators MapKit requests no tiles above `maximumZ` and draws nothing there, so deeper tiles are cut from the deepest available ancestor (checked 2026-11-16).
- `{Time}` is `YYYY-MM-DD` (UTC day) for daily layers, or `default` (or omitted: `.../default/{TileMatrixSet}/...`) for today. The granule layers take `YYYY-MM-DDTHH:MM:SSZ`. A day with no data answers `404 text/html` (MUR for 2026-11-08, the day of the check). `default` returned `200` with a tiny, mostly empty tile for the VIIRS true-colour layer (1,665 bytes), because the current day is only partly covered; request yesterday's date for a full image.
- Tile response headers include `layer-time-actual` (the date really served), `layer-identifier-actual`, `etag`, and for current layers `cache-control: max-age=0, no-store, no-cache`. Cache by (layer, date, z, x, y) in the app.

### URL templates for `MKTileOverlay`

`MKTileOverlay(urlTemplate:)` substitutes `{z}`, `{x}`, `{y}`. Put the date into the string before creating the overlay (one overlay per layer per date):

```
https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/{Layer}/default/{date}/{TileMatrixSet}/{z}/{y}/{x}.{ext}
```

Note that `{y}` precedes `{x}` in the path; MapKit's template uses names, so the order in the string is free. GIBS tiles are north-down (XYZ), so `isGeometryFlipped` stays false.

| Layer | `{Layer}` | `{TileMatrixSet}` | max zoom | `{ext}` | Latest date at check | Notes |
|---|---|---|---|---|---|---|
| True colour, VIIRS NOAA-20 | `VIIRS_NOAA20_CorrectedReflectance_TrueColor` | `GoogleMapsCompatible_Level9` | 9 | `jpg` | 2026-11-08 (partial; 2026-11-07 complete) | daily from 2018-01-05; JPEG, no alpha |
| True colour, VIIRS NOAA-21 | `VIIRS_NOAA21_CorrectedReflectance_TrueColor` | `GoogleMapsCompatible_Level9` | 9 | `jpg` | 2026-11-08 | daily from 2023-02-10 |
| True colour, VIIRS S-NPP | `VIIRS_SNPP_CorrectedReflectance_TrueColor` | `GoogleMapsCompatible_Level9` | 9 | `jpg` | **2026-11-02** (end of S-NPP mission) | do not use for new dates |
| True colour, MODIS Terra | `MODIS_Terra_CorrectedReflectance_TrueColor` | `GoogleMapsCompatible_Level9` | 9 | `jpg` | 2026-11-08 | daily from 2000-02-24 |
| True colour, MODIS Aqua | `MODIS_Aqua_CorrectedReflectance_TrueColor` | `GoogleMapsCompatible_Level9` | 9 | `jpg` | 2026-11-08 | daily from 2002-07-03 |
| SST, GHRSST MUR L4 (1 km) | `GHRSST_L4_MUR_Sea_Surface_Temperature` | `GoogleMapsCompatible_Level7` | 7 | `png` | 2026-11-07 | daily from 2002-06-01; land transparent; palette 216 entries from below 0 degree_C to above 32 degree_C (colour map fixture) |
| SST, MUR 0.25 degree | `GHRSST_L4_MUR25_Sea_Surface_Temperature` | `GoogleMapsCompatible_Level6` | 6 | `png` | 2026-11-07 | coarser |
| SST anomaly, MUR | `GHRSST_L4_MUR_Sea_Surface_Temperature_Anomalies` | `GoogleMapsCompatible_Level7` | 7 | `png` | 2026-11-07 | |
| SST, L2 swaths | `MODIS_Aqua_L2_Sea_Surface_Temp_Day`, `MODIS_Terra_L2_Sea_Surface_Temp_Day`, `VIIRS_SNPP_L2_Sea_Surface_Temp_Day` (and `_Night`) | `GoogleMapsCompatible_Level7` | 7 | `png` | MODIS 2026-11-08; S-NPP 2026-11-02 | single-pass, clouds and gaps |
| Chlorophyll, VIIRS NOAA-20 | `VIIRS_NOAA20_Chlorophyll_a` | `GoogleMapsCompatible_Level7` | 7 | `png` | 2026-11-08 | daily from 2017-12-13 |
| Chlorophyll, VIIRS NOAA-21 | `VIIRS_NOAA21_Chlorophyll_a` | `GoogleMapsCompatible_Level7` | 7 | `png` | 2026-11-07 | |
| Chlorophyll, PACE OCI | `OCI_PACE_Chlorophyll_a` | `GoogleMapsCompatible_Level7` | 7 | `png` | 2026-11-08 | from 2024-02-25 |
| Chlorophyll, MODIS Aqua / Terra | `MODIS_Aqua_L2_Chlorophyll_A`, `MODIS_Terra_L2_Chlorophyll_A` | `GoogleMapsCompatible_Level7` | 7 | `png` | 2026-11-08 | |
| Chlorophyll, S-NPP | `VIIRS_SNPP_L2_Chlorophyll_A` | `GoogleMapsCompatible_Level7` | 7 | `png` | 2026-11-02 | ended |
| Chlorophyll, Sentinel-3 OLCI | `S3A_OLCI_Chlorophyll_a`, `S3B_OLCI_Chlorophyll_a` | `GoogleMapsCompatible_Level7` | 7 | `png` | 2026-11-07 | |

Example (recorded, all 256 x 256, zoom 7, x 37, y 48, date 2026-11-07):

```
https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_NOAA20_CorrectedReflectance_TrueColor/default/2026-11-07/GoogleMapsCompatible_Level9/7/48/37.jpg
https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/GHRSST_L4_MUR_Sea_Surface_Temperature/default/2026-11-07/GoogleMapsCompatible_Level7/7/48/37.png
https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_NOAA20_Chlorophyll_a/default/2026-11-07/GoogleMapsCompatible_Level7/7/48/37.png
```

True-colour tile: JPEG, 12.7 KB; SST: 8-bit palette PNG with transparency, 5.2 KB; chlorophyll: 8-bit palette PNG, 13.7 KB. Tile 7/48/37 spans longitude -75.94 to -73.13 and latitude 38.82 to 40.98.

Layer time dimensions in the capabilities are ISO 8601 repeating intervals such as `2026-02-08/2026-11-07/P1D` (several ranges per layer where the archive has gaps). The end of the last range is the newest available day.

### Missing from GIBS

- No Kd490 or other water-clarity layer was found under the identifiers searched (`Kd`, `Attenuation`, `Turbidity`); use ERDDAP (`docs/providers/erddap.md`) for Kd490 and draw it on device.
- No surface-current layer. The SST and chlorophyll layers are colourised imagery: the pixel colour maps back to a value only through the colour map, with quantisation (SST steps of 0.15 degree_C). Values for the point inspector and the edge detector come from ERDDAP grids, not from these tiles.
- No bathymetry layer was looked for.

## Re-check, 2026-11-16

The capabilities document was fetched once (5,842,410 bytes, 1,330 layers, `cache-control: public, max-age=1800`, served by CloudFront) and the layers below were read from it; an excerpt of the eleven layers is the fixture `wmts-epsg3857-best-capabilities.excerpt-2026-11-16.xml`. The end of the last `<Dimension>` range is the newest date.

| Layer id | Advertised on 2026-11-16 | Range end | Matrix set |
|---|---|---|---|
| `VIIRS_NOAA20_CorrectedReflectance_TrueColor` | yes | 2026-11-16 (partial day) | `GoogleMapsCompatible_Level9`, JPEG |
| `VIIRS_NOAA21_CorrectedReflectance_TrueColor` | yes | 2026-11-16 | Level9, JPEG |
| `VIIRS_SNPP_CorrectedReflectance_TrueColor` | yes, ended | 2026-11-02 | Level9, JPEG |
| `MODIS_Aqua_CorrectedReflectance_TrueColor` | yes | 2026-11-16 | Level9, JPEG |
| `MODIS_Terra_CorrectedReflectance_TrueColor` | yes | 2026-11-17 (the date runs ahead of UTC) | Level9, JPEG |
| `GHRSST_L4_MUR_Sea_Surface_Temperature` | yes | 2026-11-15 | `GoogleMapsCompatible_Level7`, PNG |
| `GHRSST_L4_MUR25_Sea_Surface_Temperature` | yes | 2026-11-15 | Level6, PNG |
| `VIIRS_NOAA20_Chlorophyll_a` | yes | 2026-11-16 | Level7, PNG |
| `VIIRS_NOAA21_Chlorophyll_a` | yes | 2026-11-16 | Level7, PNG |
| `OCI_PACE_Chlorophyll_a` | yes | 2026-11-16 | Level7, PNG |
| `S3A_OLCI_Chlorophyll_a`, `S3B_OLCI_Chlorophyll_a` | yes | 2026-11-15 | Level7, PNG |
| `MODIS_Aqua_L2_Chlorophyll_A`, `MODIS_Terra_L2_Chlorophyll_A` | yes | 2026-11-16 | Level7, PNG |
| `VIIRS_SNPP_L2_Chlorophyll_A`, `VIIRS_SNPP_L2_Sea_Surface_Temp_Day` | yes, ended | 2026-11-02 | Level7, PNG |
| `MODIS_Aqua_L2_Sea_Surface_Temp_Day`, `MODIS_Terra_L2_Sea_Surface_Temp_Day` | yes | 2026-11-16 | Level7, PNG |

All ids listed in the table above for NOAA-20, NOAA-21, MODIS and PACE are still advertised; none is missing. Layers carry several `<Dimension>` ranges where the archive has gaps; only the end of the last range was used here. A NOAA-21 chlorophyll tile for 2026-11-15 (zoom 7, x 37, y 48) answered `200 image/png`, 256 x 256 (fixture), with `layer-identifier-actual: VIIRS_NOAA21_Chlorophyll_a_v2022.0_NRT` and `layer-time-actual: 2026-11-15T00:00:00Z`.

Clarity layers: the identifier search of 2026-11-16 (`kd`, attenuation, turbidity, clarity, Secchi, SPM, CDOM, Rrs, IOP, backscatter, absorption, transparency, water quality, euphotic, diffuse) found no Kd490 or water-clarity layer. The only hits were unrelated (CERES diffuse flux, MERRA-2 turbulence, particulate matter in air). PACE appears as chlorophyll, true colour, PAR and orbit tracks only. Kd490 comes from ERDDAP (`erddap.md`).

## Authentication, limits, availability

- No authentication. The documentation page fetched (`access-basics`) lists no key, quota or rate limit and no usage policy; none was encountered in about 15 tile requests. A numeric limit is **unverified**. On 2026-11-16 the GIBS documentation pages `access-basics` and `access-advanced-topics` were read again and state no rate limit, quota, throttling or usage policy (the advanced page mentions only the browser's per-domain connection limit under domain sharding); a search of public sources found no official figure. About 3 requests to `gibs.earthdata.nasa.gov` on 2026-11-16 (capabilities, one tile, one repeat) showed no rate-limit headers (`cache-control`, `access-control-allow-origin` and CloudFront headers only). A community forum post (Earthdata Forum, "Recommendations to avoid Blacklisting") mentions a limit of 20 to 60 requests a minute whose service is not stated, and could not be re-read (404); treat the figure as unverified.
- Failure modes seen: `404` (no data for that date), `400` (zoom deeper than the set). No 5xx.
- Latency: the VIIRS and MODIS true-colour and MUR layers had the previous UTC day by 14:20 UTC. Today's tile exists but is incomplete.

## License and attribution

GIBS imagery is NASA Earth science data under NASA's open data policy ("full and open sharing of data", `ows:Fees` and `ows:AccessConstraints` in the capabilities are both `none`). NASA's GIBS page asks for this acknowledgement:

> We acknowledge the use of imagery provided by services from NASA's Global Imagery Browse Services (GIBS), part of NASA's Earth Science Data and Information System (ESDIS).

The MUR layers also carry the JPL acknowledgement: "These data were provided by JPL under support by NASA MEaSUREs program." No "not for navigation" text is supplied by GIBS; the app adds its own wording to the SST and chlorophyll layers.

Re-checked 2026-11-18: unchanged. `https://nasa-gibs.github.io/gibs-api-docs/` ("Data Use Guidance and Acknowledgements") carries the same acknowledgement text quoted above and "NASA promotes full and open sharing of data". The JPL MUR acknowledgement was not read again on this date.

## What was not verified

- A numeric rate limit or fair-use threshold.
- Zoom limits of layers other than the ones tested by tile request (the table values come from the capabilities `TileMatrixSetLink`).
- Alpha behaviour of the chlorophyll PNG (it is an 8-bit palette image; whether the palette marks no-data transparent was not decoded).
