# Raw fixtures: tier-2 sources

One line per file: file name, exact request URL (no keys were used), recording date. All recorded
2026-11-08 unless stated. Files keep the source's own format; where a file was reduced, the line says so.

- `cwms-nap-beltzville-elev-2026-11-08.json`: `https://cwms-data.usace.army.mil/cwms-data/timeseries?office=NAP&name=Beltzville.Elev.Inst.15Minutes.0.Rev-DCP&begin=2026-11-08T12:00:00Z&end=2026-11-08T14:00:00Z&unit=SI` with header `Accept: application/json;version=2` (2026-11-08)
- `rise-fresno-reservoir-elevation-2026-11.json`: `https://data.usbr.gov/rise/api/result?itemId=355&dateTime%5Bafter%5D=2026-11-01&itemsPerPage=10&order%5BdateTime%5D=DESC` (2026-11-08)
- `cyan-oneida-lake-weekly.json`: `https://cyan.epa.gov/cyan/cyano/location/data/43.20/-76.00/all?type=olci&frequency=weekly` (2026-11-08)
- `noaa-hab-gulf-rif-currentmodelrun.zip`: `https://nccospublicstor.blob.core.windows.net/hab-data/no_Explorer/rif_model/csv/RIFv2_currentmodelrun.zip` (2026-11-08; model run 2026-11-08 09:00 EST; zip as served, 11 forecast CSVs plus README and data dictionary)
- `glerl-glsea-acspo-sst-erie-2026-11-07.json`: `https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.json?sst[(2026-11-07T12:00:00Z)][(42.80):1:(42.90)][(-79.00):1:(-78.90)]` (2026-11-08; brackets are percent-encoded on the wire)
- `overpass-nyharbor-slipways-piers.json`: `https://overpass-api.de/api/interpreter` with form field `data=[out:json][timeout:25];(nwr["leisure"="slipway"](40.62,-74.12,40.76,-73.95);nwr["man_made"="pier"]["name"](40.69,-74.03,40.73,-73.98););out center tags 30;` (2026-11-08)
- `padus-public-access-croton-point.json`: `https://services.arcgis.com/v01gqwM5QqNysAAi/arcgis/rest/services/PADUS_Public_Access/FeatureServer/0/query?geometry=-73.90,41.17,-73.86,41.23&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=Unit_Nm,Pub_Access,GAP_Sts,MngTp_Desc,MngNm_Desc,DesTp_Desc,ST_Name,GIS_Acres,BndryName&returnGeometry=false&resultRecordCount=10&f=json` (2026-11-08)
- `gbif-occurrence-striped-bass-nyharbor.json`: `https://api.gbif.org/v1/occurrence/search?scientificName=Morone%20saxatilis&decimalLatitude=40.4,41.0&decimalLongitude=-74.4,-73.6&limit=3&hasCoordinate=true` (2026-11-08; observer and rights-holder names replaced by `REDACTED`, key order and everything else as served)
- `inaturalist-observations-striped-bass-nyharbor.json`: `https://api.inaturalist.org/v1/observations?taxon_name=Morone%20saxatilis&quality_grade=research&license=cc-by,cc-by-nc,cc0&photo_license=cc-by,cc-by-nc,cc0&nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&per_page=3&order_by=observed_on` (2026-11-08; reduced to the fields the app would read, with no user data; the full response is about 240 KB for three observations)
- `usgs-nas-occurrence-snakehead-ny.json`: `https://nas.er.usgs.gov/api/v2/occurrence/search?state=NY&genus=Channa&limit=3` (2026-11-08)
- `socrata-ny-e52k-ymww-westchester-2025.json`: `https://data.ny.gov/resource/e52k-ymww.json?$where=county='Westchester' AND year=2025&$limit=5&$order=month` (2026-11-08)
- `socrata-ny-d9y2-n436-westchester.json`: `https://data.ny.gov/resource/d9y2-n436.json?$where=county='Westchester'&$limit=5` (2026-11-08)
- `copernicus-marine-wmts-capabilities-glo-phy-anfc-0.083deg-P1D-m.xml`: `https://wmts.marine.copernicus.eu/teroWmts/GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy_anfc_0.083deg_P1D-m_202406?SERVICE=WMTS&REQUEST=GetCapabilities` (2026-11-08; no login)
- `copernicus-marine-wmts-zos-z5-9-12-2026-11-08.png`: `https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy_anfc_0.083deg_P1D-m_202406/zos&STYLE=cmap:viridis&FORMAT=image/png&TILEMATRIXSET=EPSG:3857&TILEMATRIX=5&TILEROW=12&TILECOL=9&TIME=2026-11-08T00:00:00.000Z` (2026-11-08; no login)
- `dfo-iwls-stations-07735.json`: `https://api-iwls.dfo-mpo.gc.ca/api/v1/stations?code=07735` (2026-11-08)
- `dfo-iwls-07735-wlp-hilo-2026-11-08.json`: `https://api-iwls.dfo-mpo.gc.ca/api/v1/stations/5cebf1de3d0f4a073c4bb943/data?time-series-code=wlp-hilo&from=2026-11-08T00:00:00Z&to=2026-11-09T12:00:00Z` (2026-11-08)
- `eccc-hydrometric-realtime-05BH004.json`: `https://api.weather.gc.ca/collections/hydrometric-realtime/items?STATION_NUMBER=05BH004&f=json&limit=4&sortby=-DATETIME` (2026-11-08)
- `wqp-stations-hudson-croton-temperature.csv`: `https://www.waterqualitydata.us/data/Station/search?bBox=-74.00,41.15,-73.80,41.30&characteristicName=Temperature,%20water&mimeType=csv&zip=no` (2026-11-08)
- `wqp-results-hudson-croton-temperature-since-2022.csv`: `https://www.waterqualitydata.us/data/Result/search?bBox=-74.00,41.15,-73.80,41.30&characteristicName=Temperature,%20water&startDateLo=01-01-2022&mimeType=csv&dataProfile=narrowResult&zip=no` (2026-11-08)
- `usgs-3dhp-flowline-croton.json`: `https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/MapServer/50/query?where=gnisidlabel%20IS%20NOT%20NULL&geometry=-73.87,41.20,-73.84,41.23&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=id3dhp,gnisid,gnisidlabel,featuretypelabel,lengthkm,streamorder,mainstemid,hydrosequence,dnhydrosequence&returnGeometry=false&resultRecordCount=5&f=json` (2026-11-08)

No fixture for TVA (no machine-readable endpoint reachable), NOAA Lake Erie, Gulf of Maine and Pacific
Northwest bulletins (PDF or web pages only), and the Water Quality Portal WQX 3.0 route (timed out).
