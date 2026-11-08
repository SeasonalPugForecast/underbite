# Raw fixtures: NCEI DEM global mosaic and GEBCO

All recorded on 2026-11-08 with `User-Agent: underbite-scout/0.1 (provider docs check)`.

- `dem-global-mosaic-imageserver.json`: `https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer?f=json`
- `dem-global-mosaic-export-f32.tif`: `https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer/exportImage?bbox=-74.2,40.0,-73.0,40.8&bboxSR=4326&imageSR=4326&size=240,160&format=tiff&pixelType=F32&interpolation=RSP_BilinearInterpolation&noData=-9999&f=image` (uncompressed tiled GeoTIFF, Float32, metres, 240 x 160)
- `dem-global-mosaic-export-f32.lerc`: `https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer/exportImage?bbox=-74.2,40.0,-73.0,40.8&bboxSR=4326&imageSR=4326&size=240,160&format=lerc&pixelType=F32&noData=-9999&f=image` (same box, Esri LERC)
- `dem-global-mosaic-query-battery.json`: `https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer/query?geometry=-74.015,40.70&geometryType=esriGeometryPoint&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=Name,ProductName,CellsizeArcseconds,VerticalDatum,DateCompleted,DemName,MetadataURL,LowPS,HighPS&returnGeometry=false&f=json` (source rasters that cover The Battery)
- `gebco-wms-capabilities-1.3.0.xml`: `https://wms.gebco.net/mapserv?request=GetCapabilities&service=WMS&version=1.3.0`
- `gebco-wms-getmap-3857-z8-tile.png`: `https://wms.gebco.net/mapserv?service=WMS&version=1.3.0&request=GetMap&layers=GEBCO_LATEST_2&styles=&crs=EPSG:3857&bbox=-8296780.7,4852834.1,-8140237.8,5009377.1&width=256&height=256&format=image/png&transparent=true` (Web Mercator tile z8 x75 y96)
