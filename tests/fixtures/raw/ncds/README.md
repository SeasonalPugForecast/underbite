# Raw fixtures: NOAA Chart Display Service

All recorded on 2026-11-08 (between 14:20 and 14:32 UTC) with `User-Agent: underbite-scout/0.1 (provider docs check)`. The ArcGIS Server behind these URLs intermittently answers HTTP 200 with an error body; every file below holds a good response (see docs/providers/ncds.md).

- `noaa-chart-display-rest-mapserver.json`: `https://gis.charttools.noaa.gov/arcgis/rest/services/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/MapServer?f=json`
- `noaa-chart-display-wms-capabilities.xml`: `https://gis.charttools.noaa.gov/arcgis/rest/services/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/WMSServer?SERVICE=WMS&REQUEST=GetCapabilities` (WMS 1.3.0)
- `noaa-chart-display-wms-layers2-3-battery-256.png`: `https://gis.charttools.noaa.gov/arcgis/rest/services/MCS/NOAAChartDisplay/MapServer/exts/MaritimeChartService/WMSServer?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=2,3&STYLES=&CRS=EPSG:3857&BBOX=-8243208,4965255,-8234302,4974067&WIDTH=256&HEIGHT=256&FORMAT=image/png&TRANSPARENT=TRUE` (depths and seabed groups, lower Manhattan and the East River)
- `noaa-charts-wmts-capabilities.xml`: `https://gis.charttools.noaa.gov/arcgis/rest/services/MarineChart_Services/NOAACharts/MapServer/WMTS/1.0.0/WMTSCapabilities.xml`
- `noaa-charts-wmts-z14-x4823-y6160.png`: `https://gis.charttools.noaa.gov/arcgis/rest/services/MarineChart_Services/NOAACharts/MapServer/WMTS/tile/1.0.0/MarineChart_Services_NOAACharts/default/default028mm/12/6160/4823.png` (TileMatrix 12 is Web Mercator zoom 14; The Battery)
- `noaa-charts-wmts-z16-x19291-y24640.png`: `https://gis.charttools.noaa.gov/arcgis/rest/services/MarineChart_Services/NOAACharts/MapServer/WMTS/tile/1.0.0/MarineChart_Services_NOAACharts/default/default028mm/14/24640/19291.png` (TileMatrix 14 is zoom 16; soundings and contours off the Battery)
- `ncds-tilecache-metadata-query.json`: `https://gis.charttools.noaa.gov/arcgis/rest/services/MarineChart_Services/ncds_tilecache_metadata/MapServer/0/query?where=1%3D1&outFields=*&returnGeometry=false&f=json` (the list behind the MBTiles download page: 43 regions with download URLs and sizes)
