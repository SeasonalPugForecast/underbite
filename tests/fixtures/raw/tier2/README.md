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
- `gbif-occurrence-striped-bass-nyharbor.synthetic.json`: hand-made, the shape of `https://api.gbif.org/v1/occurrence/search?scientificName=Morone%20saxatilis&decimalLatitude=40.4,41.0&decimalLongitude=-74.4,-73.6&limit=3&hasCoordinate=true` with invented records (CC0, no real observer, URL or position)
- `inaturalist-observations-striped-bass-nyharbor.synthetic.json`: hand-made, the v1 shape of `https://api.inaturalist.org/v1/observations` for the same box with three invented observations (CC0, no real observer, URL, photo id or position)
- `usgs-nas-occurrence-snakehead-ny.json`: `https://nas.er.usgs.gov/api/v2/occurrence/search?state=NY&genus=Channa&limit=3` (2026-11-08)
- `socrata-ny-e52k-ymww-westchester-2025.json`: `https://data.ny.gov/resource/e52k-ymww.json?$where=county='Westchester' AND year=2025&$limit=5&$order=month` (2026-11-08)
- `socrata-ny-d9y2-n436-westchester.json`: `https://data.ny.gov/resource/d9y2-n436.json?$where=county='Westchester'&$limit=5` (2026-11-08)
- `copernicus-marine-wmts-capabilities-glo-phy-anfc-0.083deg-P1D-m.xml`: `https://wmts.marine.copernicus.eu/teroWmts/GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy_anfc_0.083deg_P1D-m_202406?SERVICE=WMTS&REQUEST=GetCapabilities` (2026-11-08; no login)
- `copernicus-marine-wmts-zos-z5-9-12-2026-11-08.png`: `https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy_anfc_0.083deg_P1D-m_202406/zos&STYLE=cmap:viridis&FORMAT=image/png&TILEMATRIXSET=EPSG:3857&TILEMATRIX=5&TILEROW=12&TILECOL=9&TIME=2026-11-08T00:00:00.000Z` (2026-11-08; no login)
- `eccc-hydrometric-realtime-05BH004.json`: `https://api.weather.gc.ca/collections/hydrometric-realtime/items?STATION_NUMBER=05BH004&f=json&limit=4&sortby=-DATETIME` (2026-11-08)
- `wqp-stations-hudson-croton-temperature.csv`: `https://www.waterqualitydata.us/data/Station/search?bBox=-74.00,41.15,-73.80,41.30&characteristicName=Temperature,%20water&mimeType=csv&zip=no` (2026-11-08)
- `wqp-results-hudson-croton-temperature-since-2022.csv`: `https://www.waterqualitydata.us/data/Result/search?bBox=-74.00,41.15,-73.80,41.30&characteristicName=Temperature,%20water&startDateLo=01-01-2022&mimeType=csv&dataProfile=narrowResult&zip=no` (2026-11-08)
- `usgs-3dhp-flowline-croton.json`: `https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/MapServer/50/query?where=gnisidlabel%20IS%20NOT%20NULL&geometry=-73.87,41.20,-73.84,41.23&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=id3dhp,gnisid,gnisidlabel,featuretypelabel,lengthkm,streamorder,mainstemid,hydrosequence,dnhydrosequence&returnGeometry=false&resultRecordCount=5&f=json` (2026-11-08)

No fixture for TVA (no machine-readable endpoint reachable), NOAA Lake Erie, Gulf of Maine and Pacific
Northwest bulletins (PDF or web pages only), and the Water Quality Portal WQX 3.0 route (timed out).

## Licence notes

- `overpass-nyharbor-slipways-piers.json`: data © OpenStreetMap contributors, ODbL 1.0, <https://www.openstreetmap.org/copyright>. The extract remains under the ODbL.
- `noaa-hab-gulf-rif-currentmodelrun.zip`: a NOAA NCCOS product (public domain) that also carries cell counts from third parties named in docs/providers/hab.md (FWRI, HABscope, Pinellas County, SCCF), whose terms are unstated.
- The DFO IWLS, GBIF and iNaturalist files above are synthetic (hand-made, invented values).
