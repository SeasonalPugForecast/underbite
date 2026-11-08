# Raw fixtures: ERDDAP

All recorded on 2026-11-08 with `User-Agent: underbite-scout/0.1 (provider docs check)`. Responses are byte-for-byte what the server returned.

- `jplmursst41-sst-subset-2026-11-07.json`: `https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.json?analysed_sst%5B(last)%5D%5B(40.0):5:(40.8)%5D%5B(-74.2):5:(-73.0)%5D` (time resolved to 2026-11-07T09:00:00Z; stride 5 = 0.05 degrees; 17 x 25 rows; land is `null`).
- `jplmursst41.das`: `https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.das` (dataset attributes, 2026-11-08).
- `jplmursst41-sst-wms-4326-2026-11-07.png`: `https://coastwatch.pfeg.noaa.gov/erddap/wms/jplMURSST41/request?service=WMS&version=1.3.0&request=GetMap&layers=jplMURSST41:analysed_sst&crs=EPSG:4326&bbox=40.0,-74.2,40.8,-73.0&width=384&height=256&format=image/png&transparent=true&time=2026-11-07T09:00:00Z&styles=` (384 x 256 RGBA).
- `jplmursst41-sst-transparentpng-2026-11-07.png`: `https://coastwatch.pfeg.noaa.gov/erddap/griddap/jplMURSST41.transparentPng?analysed_sst%5B(2026-11-07T09:00:00Z)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D` (121 x 81 RGBA, one pixel per grid cell).
- `ucsdhfre6-currents-subset-2026-11-08t08.json`: `https://coastwatch.pfeg.noaa.gov/erddap/griddap/ucsdHfrE6.json?water_u%5B(last)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D,water_v%5B(last)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D` (time resolved to 2026-11-08T08:00:00Z).
- `ucsdhfre6.das`: `https://coastwatch.pfeg.noaa.gov/erddap/griddap/ucsdHfrE6.das` (2026-11-08).
- `noaacwn20viirschladaily-chla-subset-2026-11-04.json`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwN20VIIRSchlaDaily.json?chlor_a%5B(2026-11-04T12:00:00Z)%5D%5B(0.0)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D` (full resolution, 22 x 33 cells, latitude descending, cloud gaps are `null`).
- `noaacwnppviirskd490daily-kd490-subset-2026-11-04.json`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPVIIRSkd490Daily.json?kd_490%5B(2026-11-04T12:00:00Z)%5D%5B(0.0)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D` (same grid; the same request to `nesdisVHNkd490Daily` on coastwatch.pfeg.noaa.gov answered 302 to this URL).
