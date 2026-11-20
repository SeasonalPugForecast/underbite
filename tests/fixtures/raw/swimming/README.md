# Swimming and beach sources: recorded responses

Recorded 2026-11-20 (UTC) with `curl`, `User-Agent: Underbite-scout/0.1 (data source check)`. Notes: `docs/providers/swimming.md`. No key, account or cookie was used. Files are the raw bytes of the response unless the line says otherwise.

## NWS surf zone forecast and rip current statements (api.weather.gov)

- `srf-types.json`: `https://api.weather.gov/products/types/SRF` (2026-11-20, 346 products, 19 issuing offices)
- `srf-locations.json`: `https://api.weather.gov/products/types/SRF/locations` (2026-11-20)
- `srf-list-BOX.json`: `https://api.weather.gov/products/types/SRF/locations/BOX` (2026-11-20, 13 products)
- `srf-list-OKX.json`: `https://api.weather.gov/products/types/SRF/locations/OKX` (2026-11-20, empty `@graph`: New York issues no SRF in November)
- `srf-product-BOX.json`: `https://api.weather.gov/products/1a4178fe-91f7-4a78-ae1d-38082c9dbb45` (2026-11-20, Boston SRF text, the only office near the test area with a current one)
- `alerts-active-rip-current-statement-us.geo.json`: `https://api.weather.gov/alerts/active?event=Rip%20Current%20Statement` (2026-11-20, 10 alerts, all Florida, Puerto Rico and Texas)
- `alerts-active-point-rockaway.geo.json`: `https://api.weather.gov/alerts/active?point=40.58,-73.82` (2026-11-20, no features)

## NWS experimental beach forecast pages (weather.gov)

- `weathergov-beach-okx-config.json`: `https://www.weather.gov/source/okx/beachforecast/config.json` (2026-11-20)
- `weathergov-beach-okx-beaches.json`: `https://www.weather.gov/source/okx/beachforecast/beaches.json` (2026-11-20; `Last-Modified: Sun, 27 Sep 2026 08:57:51 GMT`, stale summer values)
- `weathergov-beach-lot-config.json`: `https://www.weather.gov/source/lot/beachforecast/config.json` (2026-11-20)
- `weathergov-beach-lot-beaches.json`: `https://www.weather.gov/source/lot/beachforecast/beaches.json` (2026-11-20, stale)

## NWS Marine Beach Forecast map service (mapservices.weather.noaa.gov)

Base: `https://mapservices.weather.noaa.gov/vector/rest/services/outlooks/marine_beachforecast/MapServer`

- `mapservices-marine-beachforecast-mapserver.json`: `<base>?f=json` (2026-11-20, service description and 129 layers)
- `mapservices-beachforecast-layer91-meta.json`: `<base>/91?f=json` (2026-11-20, field list of the New York WFO Day 1 layer)
- `mapservices-beachforecast-layer91-sample.json`: `<base>/91/query?f=json&where=1=1&outFields=*&returnGeometry=false&resultRecordCount=3` (2026-11-20, values null out of season)
- `mapservices-beachforecast-point-rockaway-okx-day1.json`: `<base>/91/query?f=json&geometry=-73.82,40.58&geometryType=esriGeometryPoint&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=id,beachname,siteid,rip,uv,surf,wtemp,period,srfprod,idp_ingestdate&returnGeometry=false` (2026-11-20)
- `mapservices-beachforecast-point-sandyhook-phi-day1.json`: the same on layer 97 with `geometry=-73.98,40.43` (2026-11-20)
- `mapservices-beachforecast-point-chicago-lot-day1.json`: the same on layer 16 with `geometry=-87.62,41.90&distance=3000&units=esriSRUnit_Meter` (2026-11-20; without `distance` the point matched nothing)
- `mapservices-beachforecast-box-day1-sample.json`: `<base>/1/query?f=json&where=1=1&outFields=id,beachname,siteid,productdat,producttim,rip,uv,surf,wtemp,period,idp_ingestdate&returnGeometry=false&resultRecordCount=2` (2026-11-20, an in-season example from Boston)
- `mapservices-beachforecast-box-rings.synthetic.json`: hand-made, in the service's JSON shape with `rings`: three zones, one with a moderate risk, one with no forecast and two outlines (the second with a hole), one with a high risk and no outline (not a recorded answer)
- `mapservices-raster-ndfd-folder.json`: `https://mapservices.weather.noaa.gov/raster/rest/services/NDFD?f=json` (2026-11-20, only `NDFD_temp`)

## NWPS rip current probability (polar.ncep.noaa.gov)

- `nwps-OKX1.rip.txt`: `https://polar.ncep.noaa.gov/nwps/images/rtimages/validation/OKX1.rip` (2026-11-20, 673 stations, text with quoted rows)
- `nwps-PHI1.rip.txt`: `https://polar.ncep.noaa.gov/nwps/images/rtimages/validation/PHI1.rip` (2026-11-20, 131 stations)
- `nwps-okx_ripprob_stat623.png`: `https://polar.ncep.noaa.gov/nwps/images/rtimages/validation/nwps_okx_ripprob_stat623.png` (2026-11-20, image titled with the 2026/11/09 12Z run)
- `https://polar.ncep.noaa.gov/nwps/images/rtimages/validation/LOT1.rip` answered 404 (not stored).

## Beach water quality

- `chi-sensors-meta.json`: `https://data.cityofchicago.org/api/views/qmqz-2xku` (2026-11-20)
- `chi-sensors-latest.json`: `https://data.cityofchicago.org/resource/qmqz-2xku.json?$limit=5&$order=measurement_timestamp DESC` (2026-11-20, newest row 2026-09-25)
- `chi-advisory-meta.json`: `https://data.cityofchicago.org/api/views/xvsz-3xcj` (2026-11-20)
- `chi-advisory-latest.json`: `https://data.cityofchicago.org/resource/xvsz-3xcj.json?$limit=5&$order=date DESC` (2026-11-20, newest row 2025-09-01)
- `chi-swim-advisories-latest.json`: `https://data.cityofchicago.org/resource/t62e-8nvc.json?$limit=6&$order=date DESC` (2026-11-20, historical-only dataset, newest row 2016-09-22)
- `chi-lab-data-meta.json`: `https://data.cityofchicago.org/api/views/2ivx-z93u` (2026-11-20)
- `chi-beach-lab-latest.json`: `https://data.cityofchicago.org/resource/2ivx-z93u.json?$where=dna_sample_timestamp IS NOT NULL&$order=dna_sample_timestamp DESC&$limit=2` (2026-11-20, newest 2026-09-07)
- `nyc-beach-samples-meta.json`: `https://data.cityofnewyork.us/api/views/2xir-kwzz` (2026-11-20)
- `nyc-beach-samples-latest.json`: `https://data.cityofnewyork.us/resource/2xir-kwzz.json?$limit=4&$order=sample_date DESC` (2026-11-20, newest 2026-09-29)
- `nys-parks-beach-meta.json`: `https://data.ny.gov/api/views/wwwd-za77` (2026-11-20)
- `nys-parks-beach-latest.json`: `https://data.ny.gov/resource/wwwd-za77.json?$limit=4&$order=date_sample_taken DESC` (2026-11-20, newest 2026-10-19)
- `wqp-enterococcus-nynj-2025-08-25.csv`: `https://www.waterqualitydata.us/data/Result/search?bBox=-74.05,40.40,-73.75,40.62&characteristicName=Enterococcus&startDateLo=08-25-2025&startDateHi=09-05-2025&mimeType=csv&zip=no&dataProfile=narrowResult` (2026-11-20, 46 rows)

## Beach and lifeguard tags (OpenStreetMap, Overpass main instance)

- `overpass-swim-counts-nynj-ocean.json`: POST to `https://overpass-api.de/api/interpreter`, seven `out count;` statements over the box 40.40,-74.05,40.65,-73.70 (2026-11-20, base timestamp 2026-11-20T22:11:21Z); the query is in `docs/providers/swimming.md`
- `overpass-swim-counts-chicago.json`: the same over 41.65,-87.80,42.05,-87.50 (2026-11-20)
- `overpass-swim-tags-nynj-ocean.json`: POST, `out tags center;` for `natural=beach`, `emergency=lifeguard`, any `lifeguard` and `leisure=beach_resort` over the NY/NJ box (2026-11-20, 146 elements)

## UV index

- `openmeteo-uv-rockaway.json`: `https://api.open-meteo.com/v1/forecast?latitude=40.58&longitude=-73.82&hourly=uv_index,uv_index_clear_sky&daily=uv_index_max&forecast_days=2&timezone=auto` (2026-11-20)
- `epa-uv-hourly-10001.json`: `https://data.epa.gov/efservice/getEnvirofactsUVHOURLY/ZIP/10001/JSON` (2026-11-20)
- `epa-uv-daily-nyc.json`: `https://data.epa.gov/efservice/getEnvirofactsUVDAILY/CITY/New%20York/STATE/NY/JSON` (2026-11-20)
