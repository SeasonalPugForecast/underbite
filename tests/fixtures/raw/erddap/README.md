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

## Recorded on 2026-11-16 (Kd490 re-check)

Recorded with `User-Agent: underbite-scout/0.1 (provider docs check)`, one request about every 8 seconds per host. Responses are byte-for-byte what the server returned; the subset files use stride 6 (0.045 degrees on the 750 m grid) where noted.

- `noaacwn21viirskd490nswusdaily.das`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.das` (NOAA-21 VIIRS Kd490, 750 m sector US00).
- `noaacwn21viirskd490nswusdaily-time-2026-10-20-to-last.json`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?time%5B(2026-10-20):1:(last)%5D` (axis ends 2026-11-14T19:22:22Z; times are not at 12:00Z).
- `noaacwn20viirskd490nswusdaily-time-2026-10-20-to-last.json`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwN20VIIRSkd490NSWUSDaily.json?time%5B(2026-10-20):1:(last)%5D` (axis ends 2026-11-10T19:52:32Z).
- `noaacwn21viirskd490nswusdaily-kd490-ny-subset-2026-11-11.json`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?kd_490%5B(2026-11-11T19:27:55Z)%5D%5B(0.0)%5D%5B(40.0):6:(40.8)%5D%5B(-74.2):6:(-73.0)%5D` (486 rows, 313 with a value; latitude descending).
- `noaacwn21viirskd490nswusdaily-kd490-erie-subset-2026-11-10.json`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.json?kd_490%5B(2026-11-10T18:59:54Z)%5D%5B(0.0)%5D%5B(41.4):6:(42.0)%5D%5B(-83.5):6:(-82.5)%5D` (western Lake Erie; 322 rows, 94 with a value).
- `noaacwnppviirskd490daily.das`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPVIIRSkd490Daily.das` (time axis now 2025-11-16 to 2026-11-12).
- `noaacwnppviirskd490daily-time-2026-10-20-to-last.json`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPVIIRSkd490Daily.json?time%5B(2026-10-20):1:(last)%5D` (ends 2026-11-12T12:00:00Z; 2026-10-31 and 2026-11-01 are absent).
- `noaacwnppviirskd490daily-kd490-subset-2026-11-12.json`: `https://coastwatch.noaa.gov/erddap/griddap/noaacwNPPVIIRSkd490Daily.json?kd_490%5B(2026-11-12T12:00:00Z)%5D%5B(0.0)%5D%5B(40.0):(40.8)%5D%5B(-74.2):(-73.0)%5D` (726 rows, 5 with a value).
- `erdmh1kd4901day-r2022nrt.das`: `https://coastwatch.pfeg.noaa.gov/erddap/griddap/erdMH1kd4901day_R2022NRT.das` (Aqua MODIS Kd_490, global 4 km).
- `erdmh1kd4901day-r2022nrt-time-2026-11-05-to-last.json`: `https://coastwatch.pfeg.noaa.gov/erddap/griddap/erdMH1kd4901day_R2022NRT.json?time%5B(2026-11-05):1:(last)%5D` (ends 2026-11-15T12:00:00Z).
- `coastwatch-noaa-alldatasets-kd-search-2026-11-16.csv`: `https://coastwatch.noaa.gov/erddap/tabledap/allDatasets.csv?datasetID,title,maxTime,dataStructure&title=~".*(kd|Kd|attenuation|Attenuation|turbid|Turbid|KD).*"` (percent-encoded by the client; the dataset list with newest time, 2026-11-16).
- `pfeg-alldatasets-kd-search-2026-11-16.csv`: `https://coastwatch.pfeg.noaa.gov/erddap/tabledap/allDatasets.csv?datasetID,title,maxTime&title=~".*(Kd|kd|KD|ttenuation|urbid|PACE|OLCI|Secchi).*"` (the same on the pfeg node).
