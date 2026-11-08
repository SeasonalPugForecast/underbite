# NOAA Chart Display Service (NCDS)

Verified on 2026-11-08 between 14:22 and 14:35 UTC with `curl`, from Coast Survey's GIS data and services page (`https://nauticalcharts.noaa.gov/data/gis-data-and-services.html`; the older `.../data/enc-display-services.html` address answers 404) and from the services' own metadata. Fixtures: `tests/fixtures/raw/ncds/` (index in its `README.md`).

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
- Consequence for the app's "structure" layer (§6.6): the service gives a picture, not data. Picking wrecks or obstructions as points needs a different source or the REST `identify`/`query` capability (the service advertises `Map,Query`; **unverified**: no `identify` or feature query was tried).

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

## What was not verified

- Class-to-group mapping and scale thresholds of the display groups.
- Point features through REST `identify` or `query`.
- WMS `GetFeatureInfo` (capabilities advertise `text/html` and `text/xml`).
- MBTiles metadata (zoom range, tile format) and the full-file download.
- The ECDIS display WMS.
- Behaviour of the WMTS outside charted areas.
