# Bathymetry: NOAA NCEI DEM global mosaic and GEBCO

Verified on 2026-11-08 (UTC) with `curl`, from each service's own metadata and the NCEI and GEBCO web pages. Fixtures: `tests/fixtures/raw/bathymetry/` (index in its `README.md`).

**Not for navigation.** Both providers say so (quotes under "License and attribution"). Neither grid is a chart: depths are relative to land and geodetic datums, not to chart datum, and resolution varies.

## NCEI DEM global mosaic (Esri ImageServer)

Base: `https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer` (ArcGIS Image Server 11.5). No key, no account.

### What the service is

- One mosaic dataset of many bathymetric-topographic DEMs, global extent (-180 to 180, -90 to 90, EPSG:4326). Mosaic method `ByAttribute`, operator `First`, ordered by `ZOrder`, so the finest source covering a point is the one returned. Capabilities: `Image,Metadata,Catalog,Mensuration`.
- Pixel type `F32` (32-bit float), one band. Service statistics: minimum -9982.01, maximum 6180.25, mean -1891.33 (metres). Native pixel size of the finest sources 3.086e-5 degree (1/9 arc-second, about 3.4 m); coarser sources and overviews fill the rest.
- Maximum image size per request `20000 x 20000` pixels; `maxDownloadImageCount` 20; `maxRecordCount` is in the fixture.
- Sources that cover The Battery (query result, in priority order): `ncei19_n40x75_w074x25_2015v1` (1/9 arc-second, vertical datum NAVD 88, completed 2021-08-20), `ETOPO_2022_v1_15s_surface_elev` (15 arc-second), `crm_vol1_2023` (Coastal Relief Model, 1 arc-second, vertical datum EGM 2008), then overviews. The `VerticalDatum` attribute per source is therefore not constant. Metadata links in the query result point to NCEI geoportal pages (`.../metadata/geoportal/rest/metadata/item/gov.noaa.ngdc.mgg.dem:999919/html`; the page returned 404 when fetched, so the source metadata was not read).

### Units and sign

Elevation in **metres, positive up**: land above zero, water below zero (depth is the negative of the value). Checked in the 240 x 160 test export: The Battery cell +1.68 m (land edge), Ambrose approach -29.6 m, Sandy Hook Bay -3.1 m, offshore south-east corner -46.8 m, Manhattan +34.4 m; the box minimum is -83.8 m and the maximum +114.8 m. A point `identify` at 40.70 N, 74.015 W returned `value` "-7.51293" at native resolution. Vertical datum: NAVD 88 or EGM 2008 or geoid-referenced, depending on the source (table above). Charted depths (NCDS soundings) are relative to mean lower low water, so values differ from the chart by the local datum offset plus the survey's age; do not label DEM values as chart depth.

### `exportImage` (raw values)

```
{base}/exportImage?bbox={minLon},{minLat},{maxLon},{maxLat}&bboxSR=4326&imageSR=4326&size={w},{h}&format=tiff&pixelType=F32&interpolation=RSP_BilinearInterpolation&noData=-9999&f=image
```

Verified request for the test box:

```
https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer/exportImage?bbox=-74.2,40.0,-73.0,40.8&bboxSR=4326&imageSR=4326&size=240,160&format=tiff&pixelType=F32&interpolation=RSP_BilinearInterpolation&noData=-9999&f=image
```

Result: `200`, `image/tiff`, 263,000 bytes, 0.36 s. Structure of the TIFF (read from its tags):

- Classic TIFF (magic 42), little-endian, 240 x 160, 1 sample per pixel, 32 bits, `SampleFormat` 3 (IEEE float), compression 1 (**none**), **tiled** 128 x 128 (four tiles of 65,536 bytes; the image is padded to 256 x 256, so the file is about 70 % larger than 240 x 160 x 4 bytes).
- GeoTIFF tags: `ModelPixelScale` 0.005 x 0.005 degree (the box divided by `size`), `ModelTiepoint` at the north-west corner (-74.2, 40.8), `GeoKeyDirectory` for GCS WGS 84 (EPSG:4326), `GDAL_NODATA` ASCII "-9999". Rows run north to south.
- No cell in the test box equals the no-data value.

A reader needs TIFF with tiles and float32 samples; GDAL is not needed. **Unverified**: decoding with a particular Rust crate (the data were decoded here with a short script that reads the tile offsets).

Other output formats, for the same box:

- `format=lerc` (Esri LERC): `200`, `application/octet-stream`, 83,344 bytes (a third of the TIFF). The blob starts with `CntZImage ` and version 11, the original LERC (LERC1) layout rather than LERC2. A decoder for that layout is required; **unverified** that common LERC libraries read it.
- `f=json`: returns `{"href": ".../arcgisoutput/DEM_mosaics/DEM_global_mosaic_ImageServer/_ags_<id>.tif", "width", "height", "extent", "scale"}`; the image is then a second request. Not needed.
- `f=image` with `format=png` or `jpgpng` returns a colour image, not elevations. Not tested.

### Web Mercator

`exportImage` accepts `bboxSR=3857&imageSR=3857`. Verified: bbox `-8243208,4965255,-8234302,4974067`, `size=128,128`, `format=tiff&pixelType=F32` returned `200`, 66,700 bytes; `ModelPixelScale` 69.578 m. Template for a Web Mercator tile `z/x/y` (tile size `s = 40075016.6856 / 2^z` metres):

```
{base}/exportImage?bbox={minx},{miny},{maxx},{maxy}&bboxSR=3857&imageSR=3857&size=256,256&format=tiff&pixelType=F32&noData=-9999&f=image
minx = -20037508.3428 + x*s     maxx = minx + s
maxy =  20037508.3428 - y*s     miny = maxy - s
```

Tiles built this way are not images for `MKTileOverlay`; they are elevation grids to feed the on-device shading and contour code, so request them per visible tile (an overlay subclass or the core builds the URL) and cache by (z, x, y). A 256 x 256 Float32 TIFF is 263 KB (padding to tiles); 128 x 128 is 67 KB. For contours of a lake or harbour prefer one request for the whole area at the working resolution over many tiles.

### Other requests

- Point value: `{base}/identify?geometry={lon},{lat}&geometryType=esriGeometryPoint&returnGeometry=false&returnCatalogItems=false&f=json` returns `{"objectId":0,"name":"Pixel","value":"-7.51293","location":{...}}` (value as a string).
- Source rasters at a point: `{base}/query?geometry={lon},{lat}&geometryType=esriGeometryPoint&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=Name,CellsizeArcseconds,VerticalDatum,DateCompleted,MetadataURL&returnGeometry=false&f=json` (fixture).

### Limits and availability

- No limit or usage policy is stated by the service (`copyrightText` empty). One 0.4 s response per export; about eight requests were made. A rate limit is **unverified**. Keep requests per map view to a handful and cache.
- The metadata marks vertical datum per source; coverage near US coasts is the best (1/9 arc-second CUDEM tiles); open water falls back to ETOPO 2022 at 15 arc-second.

## GEBCO WMS

Base: `https://wms.gebco.net/mapserv` (MapServer, run by the British Oceanographic Data Centre for GEBCO). No key, no account. WMS 1.3.0 verified; coordinate systems `EPSG:4326`, `EPSG:3395` and `EPSG:3857`; maximum image 4096 x 4096. GetMap lists many formats (`image/png`, `image/jpeg`, `image/tiff`, vector tiles, ...), but all are renderings: `format=image/tiff` for `GEBCO_LATEST` returned an 8-bit RGB TIFF (3 samples of 8 bits), not elevations.

| Layer | Content |
|---|---|
| `GEBCO_LATEST` | GEBCO grid, shaded relief |
| `GEBCO_LATEST_2` | GEBCO grid colour-shaded for elevation |
| `GEBCO_LATEST_SUB_ICE_TOPO`, `GEBCO_LATEST_2_sub_ice_topo` | same, with ice-sheet bed topography |
| `GEBCO_LATEST_3`, `GEBCO_LATEST_TID`, `GEBCO_LATEST_TID_2` | other renderings and the Type Identifier grid (what data type each cell is based on) |

The capabilities state that the layers follow the newest grid: currently the GEBCO_2026 Grid, global, 15 arc-second (about 450 m at the equator), and that "GEBCO's grids are mainly deeper water data sets and do not contain detailed bathymetry in shallower water areas." Use it only as the offshore fallback.

URL template (Web Mercator, verified):

```
https://wms.gebco.net/mapserv?service=WMS&version=1.3.0&request=GetMap&layers=GEBCO_LATEST_2&styles=&crs=EPSG:3857&bbox={minx},{miny},{maxx},{maxy}&width=256&height=256&format=image/png&transparent=true
```

For `MKTileOverlay`, compute the bbox from `z/x/y` as above (the template names `{z}`, `{x}`, `{y}` only substitute into a path, not into a bounding box, so an overlay subclass overrides `url(forTilePath:)`). Recorded: z8 x75 y96, bbox `-8296780.7,4852834.1,-8140237.8,5009377.1`: `200`, `image/png`, 22.6 KB, 0.8 s, `cache-control: max-age=2630000, public`, `access-control-allow-origin: *`. The image is a rendering; this WMS gives no elevation values. A `GetFeatureInfo` request at one pixel on `GEBCO_LATEST_2` returned "Search returned no results" and on `GEBCO_LATEST` the error `LayerNotDefined`; a value lookup through this service is **unverified** (not offered for those layers in the tests made).

The raw grid (NetCDF, GeoTIFF, global, gigabytes) is available from `https://www.gebco.net/data-products-gridded-bathymetry-data`; its download interface and subsetting were **not examined**.

## License and attribution

- NCEI DEMs: NCEI states "Products derived primarily from NCEI data are not subject to copyright protection in the United States" and asks users to cite NCEI. For the Coastal Relief Model (`crm_vol1_2023`): "The U.S. CRM cannot be used for navigation." Citation given for the 2023 CRM: NOAA National Centers for Environmental Information (2023), Coastal Relief Models (CRMs), NOAA NCEI, doi:10.25921/5ZN5-KN44. Suggested credit: "Bathymetry: NOAA NCEI (DEM global mosaic, Coastal Relief Model, CUDEM). Not for navigation." Terms on the ImageServer itself: none stated.
- GEBCO grid: "The GEBCO Grid is placed in the public domain and may be used free of charge." Users may copy, adapt and use it commercially but must credit the source and must not suggest endorsement by GEBCO, IHO or IOC. "The GEBCO Grid should NOT be used for navigation or for any other purpose involving safety at sea." The WMS asks for this acknowledgement (capabilities abstract): "Imagery reproduced from the GEBCO_2026 Grid, GEBCO Compilation Group (2026) GEBCO 2026 Grid (doi:10.5285/4f68d5c7-45eb-f999-e063-7086abc036fa)", and its `AccessConstraints` say: "Imagery from this WMS is not to be used for navigation or any purpose relating to safety at sea."
- If the release year changes the credit text changes; read it from the capabilities of the day rather than hard-coding the year.

Re-checked 2026-11-18: unchanged. NCEI (`https://www.ncei.noaa.gov/products/coastal-relief-model`): "The U.S. CRM cannot be used for navigation."; products derived primarily from NCEI data "are not subject to copyright protection in the United States"; the 2023 citation (doi:10.25921/5ZN5-KN44) is current. GEBCO (`https://www.gebco.net/data-products/gridded-bathymetry-data` and `.../gridded-bathymetry/terms-of-use`): public domain and free of charge, commercial use allowed, the source must be acknowledged, no suggestion of official status or endorsement by GEBCO, IHO or IOC, and "should NOT be used for navigation or for any other purpose involving safety at sea". The current grid is GEBCO_2026 with the attribution "GEBCO Bathymetric Compilation Group 2026 (2026). The GEBCO_2026 Grid - a continuous terrain model for oceans and land at 15 arc-second intervals. doi:10.5285/4f68d5c7-45eb-f999-e063-7086abc036fa", the same DOI as the WMS credit above. The WMS capabilities were not fetched again.

## What was not verified

- Rate limits for either service.
- LERC1 decoding by a third-party library; any Rust TIFF decoder against the fixture.
- Vertical datum of the DEM at each point beyond the three sources listed; the source metadata pages.
- GEBCO value lookup and raw grid download.
- `format=png` and other exportImage output types.
