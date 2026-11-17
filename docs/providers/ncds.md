# NOAA Chart Display Service (NCDS)

Verified on 2026-11-08 between 14:22 and 14:35 UTC (structure points on 2026-11-17, in the section of that name) with `curl`, from Coast Survey's GIS data and services page (`https://nauticalcharts.noaa.gov/data/gis-data-and-services.html`; the older `.../data/enc-display-services.html` address answers 404) and from the services' own metadata. Fixtures: `tests/fixtures/raw/ncds/` (index in its `README.md`).

**Not for navigation.** The REST service reports `documentInfo.comments: "Not to be used for navigation."`. Show that wording with every NCDS layer.

## Services that exist today

Host: `https://gis.charttools.noaa.gov/arcgis/rest/services` (ArcGIS Server 10.9 / 10.91 behind IIS and an ArcGIS Web Adaptor). No key, no account.

| Service | URL | Verified |
|---|---|---|
| NOAA Chart Display, REST (dynamic export) | `{host}/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/MapServer` | yes: `?f=json` metadata, `/export` image |
| NOAA Chart Display, WMS 1.3.0 | `{host}/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/WMSServer` | yes: GetCapabilities, GetMap |
| NOAA Chart Display, WMTS (pre-rendered tile cache, Web Mercator) | `{host}/MarineChart_Services/NOAACharts/MapServer/WMTS` (capabilities at `.../WMTS/1.0.0/WMTSCapabilities.xml`) | yes: capabilities, tiles at two zooms |
| NOAA Chart Display, MBTiles (offline) | download page `https://distribution.charts.noaa.gov/ncds/index.html`; files `https://distribution.charts.noaa.gov/ncds/mbtiles/ncds_{region}.mbtiles` | yes: region list, `HEAD`, first 32 KB; no full download |
| ECDIS Display Service (IHO S-52 symbols, a second look) | `{host}/MCS/ENCOnline/MapServer/exts/MaritimeChartService/MapServer` and `.../WMSServer` | REST metadata only |
| MBTiles region index (feature layer behind the download page) | `{host}/MarineChart_Services/ncds_tilecache_metadata/MapServer/0` | yes: `query` |

The NOAA Chart Display Service renders ENC data with the traditional paper-chart symbols and colours; the ECDIS display service uses IHO symbology. The raster chart tile services that NCDS replaces (RNC tile service, seamless RNC) were retired in 2021 to 2022 (as reported by Coast Survey news items found by search; the notices were not re-read).

## Layers: depth areas, contours, soundings, wrecks

The service is a Maritime Chart Service (S-57 ENC rendering). It does not expose one layer per S-57 object class. Its 13 layers are display groups (REST `layers`, WMS `Layer` names `0` to `12`):

| Id | Name |
|---|---|
| 0 | Information about the chart display |
| 1 | Natural and man-made features, port features |
| 2 | Depths, currents, etc |
| 3 | Seabed, obstructions, pipelines |
| 4 | Traffic routes |
| 5 | Special areas |
| 6 | Buoys, beacons, lights, fog signals, radar |
| 7 | Services and small craft facilities |
| 8 | Data quality |
| 9 | Low accuracy |
| 10 | Additional chart information |
| 11 | Shallow water pattern |
| 12 | Overscale warning |

- Depth areas, depth contours and spot soundings are drawn together in layer 2; a GetMap for `LAYERS=2,3` over lower Manhattan returns depth-area tints, contour lines and sounding numbers (fixture `noaa-chart-display-wms-layers2-3-battery-256.png`). Wrecks, obstructions and pipelines belong to layer 3 by its name. Fish havens, restricted and special areas would be in layer 5. **Unverified**: the exact class-to-group mapping (the layers cannot be listed one by one: `.../MapServer/2?f=json` answers `Invalid URL`), and which of these appear at which scale.
- Display is scale-dependent. In the Web Mercator tile cache, a tile at zoom 14 over The Battery shows only generalised land and water fill and place names; a tile at zoom 16 shows soundings (US charts: numbers in feet or fathoms as charted) and contour lines.
- Consequence for the app's "structure" layer (§6.6): the service gives a picture, not data. REST `identify` and `query` do not work on it (see "Structure points", checked 2026-11-17); WMS `GetFeatureInfo` answers one clicked pixel at a time, and the sibling ENC Direct to GIS services return real point features.

## Web Mercator tiles and URL templates (verified)

Coordinate systems accepted by the WMS: `EPSG:3857`, `EPSG:102100` (Esri's code for the same projection), `EPSG:4326` and others. Maximum image size `14400 x 14400`. Formats `image/png`, `image/png8`, `application/pdf`. `TRANSPARENT=TRUE` gives an RGBA PNG.

**1. Pre-rendered tiles (WMTS), use first.** 256 x 256 RGBA PNG, EPSG:3857, tile matrix set `default028mm`, levels 0 to 18, extent latitude -64.85 to 76.8. The levels are offset from Web Mercator zoom by two: level 0 has scale denominator 139,770,566 (Web Mercator zoom 2) and a 5 x 3 tile matrix. The grid origin is the usual top-left of the world square, so tile columns and rows equal XYZ x and y.

```
https://gis.charttools.noaa.gov/arcgis/rest/services/MarineChart_Services/NOAACharts/MapServer/WMTS/tile/1.0.0/MarineChart_Services_NOAACharts/default/default028mm/{level}/{y}/{x}.png
level = z - 2     (z 2 to 20)
```

The template needs arithmetic on `z`, so a plain `MKTileOverlay(urlTemplate:)` cannot express it; the overlay must override `url(forTilePath:)` (or the core builds the URL from `z`, `x`, `y`). Verified tiles:

- `.../default/default028mm/12/6160/4823.png` = zoom 14, x 4823, y 6160 (The Battery): 200, `image/png`, 9.5 KB.
- `.../default/default028mm/14/24640/19291.png` = zoom 16, x 19291, y 24640: 200, 12.4 KB, with soundings and contours.

Tile headers: `cache-control: public, max-age=86400`, an `etag`. Coverage is the whole chart extent; a tile outside charted areas is blank (not tested).

**2. Dynamic WMS GetMap (any bbox, any layer set).** Build one request per Web Mercator tile from its bounds in metres (`BBOX=minx,miny,maxx,maxy`):

```
https://gis.charttools.noaa.gov/arcgis/rest/services/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/WMSServer?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS={ids}&STYLES=&CRS=EPSG:3857&BBOX={minx},{miny},{maxx},{maxy}&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE
```

`{ids}` is a comma list of group ids (for example `2,3`). Verified: 256 x 256 RGBA PNG, 39 KB for layers 2,3 over lower Manhattan, 0.3 to 0.5 s; all layers at 512 x 512 was 250 KB, 1.1 s. WMS 1.3.0 uses x,y order for `EPSG:3857`.

**3. Dynamic REST export.** Same picture, different parameters:

```
{host}/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/MapServer/export?bbox={minx},{miny},{maxx},{maxy}&bboxSR=102100&imageSR=102100&size=256,256&format=png&transparent=true&layers=show:2,3&f=image
```

Verified (200, `image/png`; for layers `show:2,3` the bytes were identical to the WMS image). The MapServer reports `supportedImageFormatTypes: "PNG, PDF"`, `spatialReference.wkid: 102100`, `singleFusedMapCache: false` (no Esri tile cache on this endpoint).

## MBTiles (offline)

- The download page draws a map of 47 regions (`ncds_01a` to `ncds_31b`, sizes 35 MB to 777 MB, 21.5 GB in all). The region list with direct URLs is in the feature layer `ncds_tilecache_metadata` (`/query?where=1%3D1&outFields=*&returnGeometry=false&f=json`; fields `name`, `date_creat`, `size` in MB, `baseline` = the download URL). A point query (`geometry=-74.015,40.70&geometryType=esriGeometryPoint&inSR=4326`) returns the region: The Battery is in `ncds_03` (577 MB, `https://distribution.charts.noaa.gov/ncds/mbtiles/ncds_03.mbtiles`).
- **The files are old.** Every region record says `date_creat` "05 28 2024", and `HEAD` on `ncds_21`, `ncds_12`, `ncds_13` returned `last-modified: Tue, 28 May 2024` although the page says baseline packages are "updated weekly". The files accept `Range` requests (`accept-ranges: bytes`; `206` for the first 32 KB).
- Format: SQLite 3 with the standard MBTiles layout (`metadata`, `map`, `images`, `keymap` tables and a `tiles` view of `zoom_level, tile_column, tile_row, tile_data`). **Unverified**: the `metadata` values (tile format, zoom range, bounds, attribution), because the table rows were not within the bytes fetched; rows are TMS (y counted from the south).
- Size makes a region too big to bundle or to fetch on mobile data without the user asking; offer it as an opt-in download per region over Wi-Fi.

## Availability seen during the check (UTC, 2026-11-08)

All failures were **HTTP 200 with a wrong body**, never a 5xx status:

- 14:22 to 14:23: the capabilities documents, two tiles and two GetMap images were served on the first try. (The tile responses carry a `date` header about ten minutes behind the real time; the server clock, or a cache, is off, so do not use it for freshness.)
- About 14:23 to 14:25: `?f=json` on the NOAA Chart Display REST MapServer, on `NOAACharts/MapServer` and on `ncds_tilecache_metadata/MapServer/0` answered 200 with the 155-byte body `{"error":{"code":500,"message":"Error handling service request :Could not find service. Service may be stopped or it may not be configured.","details":[]}}` on about half the tries; repeating the same request a moment later succeeded (the metadata query needed 5 tries). `NOAACharts/MapServer?f=json` returned this error on every try, although the same service's WMTS tiles were served normally.
- 14:31 to 14:32: ten rounds of three small requests (WMS GetMap, WMTS tile, REST export). WMS: 2 of 10 returned an HTML page (`Application Error` from "ArcGIS Web Adaptor": "Could not access any server machines. Please contact your system administrator.") with status 200; WMTS: 1 of 10; REST export: 0 of 10.
- No `503` was observed in this check. Third parties have reported intermittent `503` responses; no official outage notice was found on the Coast Survey page.

Client rule: a response is a tile only if the `content-type` is `image/png`; treat a 200 with `text/html` or a JSON `error` body as a failure, retry once with back-off, then show the last cached tile or nothing, and cache for at least a day.

## License and attribution

- No terms of use are stated on the Coast Survey page or in the capabilities (`Fees` and `AccessConstraints` are empty). The data are NOAA ENC data, a US government work. Credit "NOAA Office of Coast Survey, NOAA Chart Display Service (ENC)". Keep the "not for navigation" note.
- Updates: the page states that the ENC data and chart images are updated weekly. Chart tiles show no date in the response; the app cannot say how old a tile is beyond the cache header.

## Structure points (checked 2026-11-17)

Checked on 2026-11-17 with `curl` and `User-Agent: underbite-scout/0.1 (provider docs check)`, one request every four seconds or more, about 90 requests in all. Fixtures: `tests/fixtures/raw/ncds/` (lines under "Structure points" in its `README.md`).

**Verdict: the NOAA Chart Display MapServer gives a picture only through REST; points are available through WMS `GetFeatureInfo` (a pixel click whose answer carries no coordinates, so not a viewport list) and, as real queryable point layers, through the ENC Direct to GIS services on the same host.** ENC Direct is a different NOAA service from NCDS, found here and not yet a decision for the app.

### Requests on the Maritime Chart Service (`.../MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService`)

| Request | Result | Status |
|---|---|---|
| REST `MapServer/identify` (point; `layers=all`, `visible`, `top`, `all:0..5`; input 4326 or 102100; tolerance 3 to 20) | HTTP 200, `{"error":{"code":500,"message":"Error in handling operation identify"}}` on every one of seven tries with six parameter variants. This message is stable; the transient "Could not find service" body is a different one | **fails** |
| REST `MapServer/{n}/query`, `MapServer/{n}?f=json` | error 400 `Invalid URL`: the 13 groups are not addressable as layers | **fails** |
| REST `MapServer/query` (with `layers=3`) | error 400 `Unable to complete Query operation`, details `database not enabled`. The `Query` capability in the metadata is therefore not usable | **fails** |
| REST `MapServer/layers?f=json` | the same 3,053 bytes as `MapServer?f=json`: the group list without fields | works, no new information |
| REST `MapServer/find` (`searchText`, `layers=0`, `contains=true`, `returnGeometry=true`, `sr=4326`) | works. One pseudo-layer, id 0 "S57 Cells", holds every S-57 object of every US chart cell. Matches are a substring of an attribute value such as `Object name`, across the whole country (no spatial filter). `layerDefs`, `searchFields` and a search by object class (`WRECKS`) have no effect. Answers can be large: "Wreck" returned 333 objects (511 KB), "Reef" 2.7 MB. A wreck or obstruction without a name cannot be found. The New York charts name no fish haven; `Okaloosa Fish Haven 15` shows the shape (`OBSTRN`, category "fish haven") | works, unusable for "near me" |
| WMS 1.3.0 `GetFeatureInfo`, `INFO_FORMAT=text/xml` or `text/html` | works (below) | works |

### WMS GetFeatureInfo

- Parameters: the `GetMap` set plus `QUERY_LAYERS` (group ids), `I`, `J` (pixel column and row, origin top left), `INFO_FORMAT` (`text/xml` or `text/html`; `application/vnd.ogc.gml` untried) and `FEATURE_COUNT`. Request example, the Lower Bay at 8 m per pixel:

```
https://gis.charttools.noaa.gov/arcgis/rest/services/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/WMSServer?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&LAYERS=3&QUERY_LAYERS=3&STYLES=&CRS=EPSG:3857&BBOX=-8236000,4948000,-8228000,4956000&WIDTH=1024&HEIGHT=1024&I=903&J=306&INFO_FORMAT=text/xml&FEATURE_COUNT=20
```

- Response (`text/xml`): `<GetFeatureInfoResponse>` with one `<Feature dataset="US5NY1BE.000" cscl="22000" rcid="1038" code="159" acronym="WRECKS" description="Wreck" lnam="..." geometryType="point">` per object and one `<Attribute acronym="CATWRK" description="Category of wreck" value="5" mappedValue="wreck showing any portion of hull or superstructure"/>` per attribute. Coded values come as `value` and `mappedValue`. `cellName`, `compilationScale` and `geometryType` (`point`, `line`, `area`) close each feature. **There is no geometry and no coordinate in the answer**: the position is the clicked pixel. `text/html` carries the same table with the words only, no codes.
- Every answer also carries the `M_COVR` coverage polygon of the chart cell. An answer with nothing else is 834 bytes; a bare `<GetFeatureInfoResponse>` root (90 bytes) came for a 1 x 1 pixel image.
- `QUERY_LAYERS` filters. At one Lower Bay pixel group 1 returned `MORFAC`, `SLCONS`, `BUAARE`, `LNDARE`; group 2 `DEPCNT`, `DEPARE`; group 3 `WRECKS`; group 5 `RESARE`, `CTNARE`; groups 0 to 12 together returned all of them in one 19.7 KB answer.
- Pixel exactness: a point wreck was hit at its drawn pixel and 8 pixels below it, and missed 3 and 8 pixels to the side (8 m per pixel). It is a click on a symbol, not a radius search. A 1 x 1 or 8 x 8 pixel image over a large bounding box did not turn it into an area query (nothing but `M_COVR`).
- Attributes seen: `WRECKS` has `CATWRK`, `QUASOU`, `VALSOU`, `WATLEV`, `SORDAT`, `SORIND`; `OBSTRN` has `CATOBS` ("foul area"), `VALSOU`, `WATLEV`; `DEPARE` has `DRVAL1`, `DRVAL2`; `DEPCNT` has `VALDCO`; `RESARE` has `CATREA`, `RESTRN`, `INFORM`.
- A missing depth is an empty string (`value=""`), not a number. A negative `DRVAL1` (`-1.6`) is a drying height.
- **Unit and datum are not stated.** No attribute, no WMS capabilities text and no REST metadata gives metres, feet or a datum. S-57 stores depths in metres relative to the cell's sounding datum (IHO convention, found by search, not a NOAA statement here); the cell's datum objects (`M_VDAT`, `M_SDAT`) were not returned for New York. Treat unit and datum as **unverified** and do not assume MLLW.

### Which group draws which class, and from what scale

Observed by `GetFeatureInfo` at one place (the Lower Bay, cell `US5NY1BE`, compiled at 1:22,000) and by `GetMap` of the single group; not exhaustive.

| Group | Id | Seen |
|---|---|---|
| Natural and man-made features, port features | 1 | `LNDARE`, `BUAARE`, `SLCONS`, `MORFAC` |
| Depths, currents, etc | 2 | `DEPARE` (depth range), `DEPCNT` (contour); soundings are drawn here |
| Seabed, obstructions, pipelines | 3 | `WRECKS` (points), `OBSTRN` (points and areas, "Foul" labels), cable areas drawn; fish havens are `OBSTRN` objects, so group 3 draws them (**inferred**, no fish haven in the test places) |
| Special areas | 5 | `RESARE`, `CTNARE` (restricted and caution areas) |

Scale: the service draws the cells of the band that suits the scale, not by a fixed zoom. For group 3 over the Lower Bay: wreck symbols at about 2 m per pixel (zoom 16) and at 8 m per pixel (zoom 14); a rich image at 23 and 35 m per pixel (about 1:84,000 and 1:126,000); an **empty image** at 38 m per pixel (about 1:137,000); a few depth areas only at 78 m per pixel (about 1:280,000). The thresholds follow each cell's compilation scale, so they differ per place; a client should not hide structure behind a fixed zoom.

### ENC Direct to GIS, the point source (not NCDS)

- Host `https://gis.charttools.noaa.gov/arcgis/rest/services/encdirect`; `https://encdirect.noaa.gov/arcgis/rest/services/encdirect` is the host Coast Survey's page `https://www.nauticalcharts.noaa.gov/learn/encdirect/` names (the same wreck count, 590, came from both, though the second answered the first try with the transient error). The page lists MapServers `enc_overview`, `enc_general`, `enc_coastal`, `enc_approach`, `enc_harbour`, `enc_berthing` by scale band, says the data are "updated every Saturday night" and "not certified for navigation", and states no licence, no attribution text, no unit and no vertical datum. Service descriptions: `enc_harbour` 1:5,000 to 1:50,000, `enc_approach` 1:50,001 to 1:150,000, `enc_coastal` 1:150,001 to 1:600,000; "This map service is not intended for navigation purpose."
- Capabilities `Map,Query,Data`, `supportedQueryFormats` "JSON, geoJSON", `maxRecordCount` 1000, ArcGIS 10.91. Unlike NCDS there is one layer per S-57 class and geometry type, for example in `enc_harbour`: 32 `Harbor.Fishing_Facility_point`, 33 `Harbor.Obstruction_point`, 34 `Harbor.Underwater_Awash_Rock_point`, 36 `Harbor.Wreck_point`, 99 `Obstruction_line`, 156 `Obstruction_area`, 158 `Wreck_area`. **The layer ids differ per band**: `enc_approach` has 35 fishing facility, 36 obstruction, 37 rock, 39 wreck; `enc_coastal` has 30 obstruction, 31 rock, 33 wreck. Read the ids from each service's `?f=json`; never copy them across bands.
- A query that works (wrecks of the Lower Bay):

```
https://gis.charttools.noaa.gov/arcgis/rest/services/encdirect/enc_harbour/MapServer/36/query?f=json&geometry=-74.10,40.55,-73.95,40.70&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=*&returnGeometry=true&outSR=4326&resultRecordCount=5
```

  `f=geojson` also works (content type `application/geo+json`; `exceededTransferLimit: true` when cut off). `returnCountOnly=true` gave `{"count":590}` for wrecks, 986 obstruction points and 274 awash rocks in the harbour band over the box `-74.20,40.35,-73.70,40.75`. The approach band held no wreck in `-73.90,39.90,-73.30,40.40`; the coastal band returned wrecks there (for example `VALSOU` 20.1, cell `US3NY1AE`). Paging with `resultOffset` was not tried.
- Wreck point fields: `OBJECTID`, `OBJL` (159), `CATWRK` (text, **cut to 30 characters**: "wreck showing any portion of h"), `CONRAD`, `CONVIS`, `EXPSOU`, `HEIGHT`, `OBJNAM`, `QUASOU` (a code as text), `SOUACC`, `TECSOU`, `VALSOU` (double, `null` when unknown), `VERACC`, `VERDAT`, `VERLEN`, `WATLEV` (a code as a number), `INFORM`, `SCAMIN`, `SORDAT` (`YYYYMMDD` or `YYYYMM`), `SORIND`, `DSNM` (chart cell). Obstruction points add `CATOBS` (text such as "snag/stump"), `CONDTN`, `NATCON`, `NATQUA`, `PRODCT`, `NATSUR`. Coded fields are mixed: some are domain text, some a number, some a code in a string; `CATWRK` carried the value "non-dangerous wreck (test)" in the coastal band.
- Cross-check: the wreck at about 40.6006 N 73.9218 W with `VALSOU` 2.6 from the WMS pixel query is `OBJECTID` 81133 here, at `x -73.9218963, y 40.6005615`, cell `US5NY1BE.000`, `SORDAT` 20131220 in both services. A second wreck in the same box has `VALSOU` 2.4.
- Unit and datum: the layer metadata gives none, `VERDAT` is `null` for every New York wreck returned, and the datum layers `Sounding_Datum_area` (224, empty at the Battery and the Lower Bay) and `Vertical_Datum_of_Data_area` (225, 149 features in the US, none near New York) did not say. **Unit and datum unverified**; the S-57 convention is metres, which the service never states. Do not label a depth in feet or against MLLW without a statement.
- Fish havens: none in the harbour band of the Bight (`CATOBS='fish haven'` count 0). They exist as `OBSTRN` objects elsewhere (Florida, in the `find` fixture). New York's artificial reefs were not found in the ENC layers checked.
- Availability: the same random HTTP 200 "Could not find service" bodies as NCDS; on 2026-11-17 about one request in four failed, in bursts, and each failure cleared on a retry within one or two tries.

### Other NOAA wreck sources found (not verified as usable)

- `https://gis.charttools.noaa.gov/arcgis/rest/services/public_wrecks/Wrecks_And_Obstructions/MapServer` (named in search results as combining ENC and AWOIS wrecks and obstructions): the `public_wrecks` folder answers HTTP 200 with error 499 `Token Required`, the service itself error 500 "not started". Not public today.
- Search results also named `https://gis.ngdc.noaa.gov/arcgis/rest/services/GulfDataAtlas/ShippingAndNavigation/MapServer/2` (Gulf of Mexico) and `https://coast.noaa.gov/arcgis/rest/services/OceanReports/WrecksAndObstructions/MapServer/0`; neither was requested. Third-party copies (HIFLD on `maps.nccs.nasa.gov`, a Rutgers server) exist; their licences were not read.

### Licence and attribution for structure data

As for NCDS: no terms on the pages read; US government ENC data; credit "NOAA Office of Coast Survey, ENC Direct to GIS" (or "NOAA Chart Display Service (ENC)"); keep "not for navigation". The ENC Direct page's only disclaimer: "This service is for informational purposes only and does not constitute an endorsement by the U.S. Government."

## What was not verified

- Exact scale thresholds per class and per ENC band (only the Lower Bay was probed), and the complete class-to-group mapping.
- `GetFeatureInfo` as GML, and rocks (`UWTROC`) in group 3.
- ENC Direct bands `enc_overview`, `enc_general`, `enc_berthing`, the line and area wreck and obstruction layers, and paging with `resultOffset`.
- The depth unit and vertical datum of every depth these services return (never stated in their responses).
- MBTiles metadata (zoom range, tile format) and the full-file download.
- The ECDIS display WMS.
- Behaviour of the WMTS outside charted areas.
