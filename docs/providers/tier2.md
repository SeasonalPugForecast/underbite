# Tier-2 data sources: survey

Verified on **2026-11-08** with one working request per source (a few sources needed two or three
tries to find a working query). All requests were made anonymously with the User-Agent
`underbite-scout/0.1 (source survey)`. Recorded responses are in
`tests/fixtures/raw/tier2/` and indexed, with exact URLs, in `tests/fixtures/raw/tier2/README.md`.
Anything not seen directly is marked **unverified**, with the reason.

This is a survey, not a parser specification: each section gives enough to decide whether to build an
adapter and what the first request looks like. A parser needs the full field list from the recorded
fixture and the vendor's own schema.

## Summary

| Source | Base URL | Auth | Verdict (2026-11-08) |
|---|---|---|---|
| USACE CWMS Data API | `https://cwms-data.usace.army.mil/cwms-data` | none | usable with caveats |
| USBR RISE | `https://data.usbr.gov/rise/api` | none | usable with caveats |
| TVA lake levels and release schedules | `https://www.tva.com/environment/lake-levels` (web page only) | n/a | not usable |
| EPA CyAN | `https://cyan.epa.gov/cyan/cyano/` | none (anonymous requests accepted) | usable with caveats |
| NOAA HAB forecasts | several (see section) | none | usable with caveats (Gulf CSV feed only; other regions are bulletins) |
| NOAA GLERL ERDDAP (GLSEA, GLCFS) | `https://apps.glerl.noaa.gov/erddap` | none | usable with caveats (SST and ice yes; GLCFS currents and waves not present) |
| OpenStreetMap Overpass | `https://overpass-api.de/api/interpreter` | none | usable with caveats |
| PAD-US (USGS) | `https://services.arcgis.com/v01gqwM5QqNysAAi/arcgis/rest/services/PADUS_Public_Access/FeatureServer` | none | usable |
| GBIF occurrence API | `https://api.gbif.org/v1/occurrence/search` | none | usable with caveats |
| iNaturalist API | `https://api.inaturalist.org/v1` | none for reads | usable with caveats |
| USGS NAS | `https://nas.er.usgs.gov/api/v2` | none | usable with caveats |
| NY Socrata stocking `e52k-ymww`, `d9y2-n436` | `https://data.ny.gov/resource/<id>.json` | none (app token optional) | usable with caveats |
| Copernicus Marine WMTS | `https://wmts.marine.copernicus.eu/teroWmts` | none for WMTS (verified); subsetting **unverified** | usable with caveats |
| DFO IWLS | `https://api-iwls.dfo-mpo.gc.ca/api/v1` | none | usable |
| ECCC hydrometric | `https://api.weather.gc.ca/collections/hydrometric-realtime/items` | none | usable |
| Water Quality Portal | `https://www.waterqualitydata.us/data/{Station,Result}/search` | none | usable with caveats (history only) |
| USGS 3DHP (National Map) | `https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/MapServer` | none | usable |

Cross-cutting findings:

- Two sources sit behind bot challenges or return non-JSON errors to scripted clients: `www.tva.com`
  (Cloudflare challenge, HTTP 403 for every path including `robots.txt`) and the iNaturalist
  documentation pages (HTTP 403; the API itself is fine).
- Times: CWMS returns epoch milliseconds; RISE, IWLS, ECCC, GLERL and Socrata (where dated) are ISO 8601
  in UTC; CyAN uses `MM-DD-YYYY HH:MM:SS:S UTC` strings plus epoch milliseconds; NOAA HAB CSV uses ISO 8601
  with explicit offsets (UTC and -05:00 mixed in one file).
- Several services publish in feet, acre-feet or cfs by default (CWMS `EN`, RISE, Socrata sizes in
  inches); conversion happens in the core.
- Coordinates leave the device only rounded (project rule); every request below takes a bounding box or a
  rounded point, none needs a precise spot.

---

## USACE CWMS Data API

Verdict: **usable with caveats** (2026-11-08).

- Base: `https://cwms-data.usace.army.mil/cwms-data`. Version selected by header
  `Accept: application/json;version=2`. The OpenAPI document at `/cwms-data/swagger-docs` reports title
  "CWMS Data API", version `2026.09.22-b`.
- Auth: none for reads. The OpenAPI document lists API keys and OpenID Connect for write operations and
  office-scoped access; not needed for the requests below.
- Limits: none documented; operations list HTTP 429 but no numbers. No rate-limit headers seen.
- Terms: no license or terms text in the OpenAPI info section (**unverified**). Content is produced by a US
  federal agency; no attribution text found. Credit "U.S. Army Corps of Engineers" in the source line.

Requests used:

```text
GET /cwms-data/offices                                         (list of offices)
GET /cwms-data/catalog/TIMESERIES?office=NAP&like=Beltzville\..*&page-size=40
GET /cwms-data/timeseries?office=NAP&name=Beltzville.Elev.Inst.15Minutes.0.Rev-DCP
      &begin=2026-11-08T12:00:00Z&end=2026-11-08T14:00:00Z&unit=SI
```

Response (fixture `cwms-nap-beltzville-elev-2026-11-08.json`):

| Field | Meaning |
|---|---|
| `name`, `office-id` | time-series identifier (location.parameter.type.interval.duration.version) and office |
| `units` | `m` with `unit=SI`; the default (`EN`) returned `ft` |
| `time-zone` | `US/Eastern` (the series' local zone); the values themselves are UTC epoch |
| `interval`, `interval-offset` | ISO 8601 duration (`PT15M`) and offset in minutes |
| `value-columns` | `date-time` (epoch **milliseconds**, UTC), `value` (double or `null`), `quality-code` (int) |
| `values` | rows `[epoch_ms, value, quality]`; missing value is `null` with quality `5` in the two newest rows seen (the meaning of quality codes is **unverified**) |
| `vertical-datum-info` | native datum (`NAVD-88`), station elevation in `unit`, offsets to other datums |
| `page`, `page-size`, `total` | cursor paging; default page size 500 |

Findings:

- Pool elevation was found at the Corps' Philadelphia District (Beltzville Lake, Pennsylvania): series
  `Elev`, `Stor`, `Flow-In`, `Precip`, `%-FloodControl`. No outflow (`Flow-Out`) series was catalogued for
  Beltzville, so release data is not available there; tailwater series exist at other projects
  (**unverified**).
- The New York District (`NAN`) catalogue returned no `Elev` series (only air temperature, precipitation
  and unregulated flow), so there is no pool-level data near the New York test places.
- The catalogue's `extents.latest-time` was stale (2026-09-14 for every series seen) while the
  `timeseries` endpoint returned data up to 2026-11-08 13:30 UTC. Do not use catalogue extents to decide
  whether a series is current.
- Series names use office-specific conventions (`Rev-DCP`, `Ccp-Rev`, `Computed`): a catalogue of
  location-to-series mappings has to be curated per office.

## USBR RISE

Verdict: **usable with caveats** (2026-11-08).

- Base: `https://data.usbr.gov/rise/api` (JSON:API / Hydra; `docs.jsonld` describes the schema).
- Auth: none. Limits: none documented or seen in headers (**unverified**: the `/rise-api` documentation
  page had no readable content). Terms: each catalogue item carries the disclaimer "Data are provisional
  and subject to revision unless otherwise noted..."; no attribution text found.

Requests used:

```text
GET /rise/api/location/337                              (Fresno Reservoir and Dam, Montana)
GET /rise/api/catalog-record/2307                       (lists catalogItems 17, 354, 355, ...)
GET /rise/api/catalog-item/355                          (Lake/Reservoir Elevation, ft, daily)
GET /rise/api/result?itemId=355&dateTime[after]=2026-11-01&itemsPerPage=10&order[dateTime]=DESC
```

Result attributes (fixture `rise-fresno-reservoir-elevation-2026-11.json`):

| Field | Meaning |
|---|---|
| `dateTime` | ISO 8601 UTC; daily items are stamped 07:00Z (local midnight, MST) |
| `result` | number in the catalogue item's `parameterUnit` (`ft` for elevation, `af` storage, `cfs` release) |
| `status`, `resultAttributes` | `null` in the rows seen |
| `itemId`, `locationId`, `parameterId`, `sourceCode` | joins to catalogue entries |
| `createDate`, `updateDate` | server timestamps |

Findings:

- 838 locations and 8,922 catalogue items at the time of the check. Daily timestep for Fresno; the item
  also lists `temporalEndDate`, which was one day behind the date of the check.
- Query filters on `catalog-item` and `location` that I tried (`parameterId`, `locationId`, `itemTitle`,
  `q`) were ignored; only `id=` filtered. Navigate location -> catalog-record -> catalog-item instead.
  `result` without `itemId` timed out at 30 seconds.
- Reservoir coverage is western (Bureau of Reclamation projects); nothing near the New York test places.

## TVA lake levels and release schedules

Verdict: **not usable** (2026-11-08).

- `https://www.tva.com/environment/lake-levels` and every other `www.tva.com` path tried, including
  `/robots.txt`, returned HTTP 403 with a Cloudflare "Just a moment..." challenge page to a scripted
  client. `https://api.tva.gov/` returned HTTP 526. The challenge was not worked around.
- A web search for a documented API, ArcGIS service or data download found none. TVA publishes
  elevations, outflows and planned release schedules through the lake levels web page, the TVA Lake Info
  phone app and a telephone line. Search snippets of the page state that next-day release schedules are
  usually available by 6 p.m. local time, that lake-level data are unavailable 6-8 p.m. Eastern on the
  third Tuesday of each month, and that schedules can change without notice (**unverified**: page not
  readable by script).
- No fixture. Options left open: USGS gauges at tailwaters (already in the USGS source) and, for projects
  owned by the Corps in the same region, CWMS (**unverified**).

## EPA CyAN (cyanobacteria index)

Verdict: **usable with caveats** (2026-11-08).

- EPA's CyAN page states the Android app ended after 2024-12-31 and that "the CyAN REST API will remain
  unchanged and continue to provide processed cyanobacteria index (CI) estimate data with no planned
  changes at this time". The page does not publish the API's URL or schema.
- The working endpoint was found in the script of the official web app
  (`https://qed.epa.gov/cyanweb/`): `https://cyan.epa.gov/cyan/cyano/location/data/{lat}/{lon}/all`
  with `?type=olci&frequency=weekly` or `frequency=daily`. It answered anonymously.
  The web app also calls `/cyan/app/api/user` and `/cyan/app/api/user/register` for saved locations;
  these were not used and no registration was made.
- Limits: none documented. Terms: the web app states "This experimental web application provides
  provisional satellite derived measures of cyanobacteria, which may contain errors and should be
  considered a research tool." Show that wording near the data. No license text found.

Request and response (fixture `cyan-oneida-lake-weekly.json`, Oneida Lake, New York, used as the inland
lake):

```text
GET https://cyan.epa.gov/cyan/cyano/location/data/43.20/-76.00/all?type=olci&frequency=weekly
```

| Field | Meaning |
|---|---|
| `metaInfo.locationName` | nearest mapped waterbody ("Maple Bay" for this point); `"Unknown Location"` with empty `outputs` when the point is not in one of the ~2,000 mapped lakes |
| `metaInfo.queryDate`, `requestTimestamp` | strings `MM-DD-YYYY HH:MM:SS:S UTC` (and `...Long` epoch milliseconds) |
| `outputs[].imageDate` / `imageDateLong` | date of the 7-day (weekly) composite |
| `outputs[].cellConcentration`, `maxCellConcentration` | cells/mL (unit label taken from the web app; mean and maximum over the valid cells) |
| `outputs[].validCellsCount` | number of 300 m cells that had a valid retrieval |
| `satelliteImageType`, `satelliteImageFrequency` | `OLCI`, `Weekly` or `Daily` |

Findings: points at New Croton Reservoir (41.20, -73.79) and Lake Champlain (44.55, -73.30) returned
"not a lake or not yet available". The Oneida Lake point returned 16 weekly records from 2021-04-03 to
2026-03-28 and none after, so records appear only when cells are retrievable (cloud, ice and season); the
seasonal pattern is **unverified**. The base path is an undocumented interface that EPA says it keeps;
treat it as best effort.

## NOAA harmful algal bloom (HAB) forecasts

Verdict: **usable with caveats** (2026-11-08). Only the Gulf of Mexico respiratory-irritation forecast is
a machine-readable feed; the other regions publish bulletins or viewers.

| Region | Delivery found | URL |
|---|---|---|
| Florida and Texas (Gulf) | zip of CSV files, model run every 3 hours | `https://nccospublicstor.blob.core.windows.net/hab-data/no_Explorer/rif_model/csv/RIFv2_currentmodelrun.zip` |
| Lake Erie | PDF bulletin (current) plus HTML product pages; severity-index XLSX | `https://cdn.coastalscience.noaa.gov/hab-data/forecasts/LErie_forecast/web_latest/current_bulletin.pdf` (HTTP 200, `application/pdf`, 3.7 MB, last modified 2026-11-08 11:57 GMT) |
| Gulf of Maine | HTML nowcast/forecast page and a weekly PDF dashboard, published only when a bloom is present | `https://nccospublicstor.blob.core.windows.net/hab-data/forecasts/gulfofmaine/dashboard_GMaine.pdf` (not fetched) |
| Pacific Northwest | NANOOS HAB bulletin web page (partner product); no feed described | `https://www.nanoos.org/products/habs/forecasts/home.php` (HTTP 200, HTML) |

No JSON, GeoJSON, KML or shapefile feed is listed for any region on NOAA's HAB forecast page; the listed
downloads are the Gulf CSV zip, the Lake Erie severity-index spreadsheet and archived bulletins at NCEI.
NOAA's NCCOS pages carried a notice on 2026-11-08 that the site "will not be updated" during a government
closure, although the CSV zip was modified the same day (`Last-Modified: 2026-11-08 14:11 GMT`).

Gulf CSV (fixture `noaa-hab-gulf-rif-currentmodelrun.zip`, 28 KB): one file per forecast hour
(`..._forecast_valid_01hrs.csv`, 04, 07, ... 31), about 55 beaches each, plus `RIFv2_README.txt` and
`RIFv2_data_dictionary.csv`. Columns: `Forecast` (`Very low|Low|Moderate|High`), `Sample Used`,
`Cell Count Category`, `Sample Time` (UTC), `Wind Speed` (knots), `Wind Dir` (degrees), `Wind Category`
(`Offshore|Light|Medium|High`), `Wind Time` (UTC), `Shoreline Angle`, `Lat`, `Lon`, `Confidence`
(0.0, 0.1 or 0.9), `Location Name`, `Model Run Time` (US Eastern, -05:00), `Cell Count Source`.
The bundled README credits the cell counts to the Florida Fish and Wildlife Research Institute, HABscope,
Pinellas County and the Sanibel-Captiva Conservation Foundation, and the wind to NDFD.

Limits and terms: none stated; NOAA content is public domain. Credit "NOAA National Centers for Coastal
Ocean Science" and the cell-count sources above. Not applicable to the New York test places (no HAB
forecast covers them).

## NOAA GLERL ERDDAP (GLSEA and GLCFS)

Verdict: **usable with caveats** (2026-11-08).

- Base: `https://apps.glerl.noaa.gov/erddap` (ERDDAP 2.31.1, response header `Erddap-Server`). About 328
  datasets listed in `tabledap/allDatasets`.
- Auth: none. Limits: none documented. License text from the datasets' metadata: "The data may be used
  and redistributed for free but is not intended for legal use, since it may contain inaccuracies.
  Neither the data Contributor, ERD, NOAA, nor the United States Government, nor any of their employees
  or contractors, makes any warranty ..." Credit "NOAA Great Lakes Environmental Research Laboratory
  (CoastWatch Great Lakes Node)".
- **GLCFS currents, waves and water levels are not served here.** The only hydrodynamic datasets found
  are long-term reanalyses for Lakes Michigan, Superior and Ontario (`LM_fvcom_temp`, `LS_fvcom_temp`,
  `fvcom_o_temp_test`, and `*_h_zeta`). Operational forecasts live elsewhere (CO-OPS OFS / NODD)
  (**unverified**, not tested).
- Useful live datasets: `GLSEA_ACSPO_GCS` (daily sea surface temperature, 2006-12-11 to 2026-11-07 on
  the check date), `GL_Ice_Concentration_GCS`, `glsea_avgtemps_3` (daily lake-average temperature),
  imagery datasets (`GL_MODIS_True_Color`, `GL_VIIRS_True_Color`, `GL_Water_CTI_IMG` turbidity index).
- Titles for Lake Ontario products changed in 2026-09 to "Lake America" (for example `LO_SST_FP_o2`,
  `LA_MODIS_True_Color`); dataset IDs keep the `LO_` prefix. Key parsers on dataset IDs, not titles.

Request (fixture `glerl-glsea-acspo-sst-erie-2026-11-07.json`, a 0.1-degree box off Buffalo):

```text
GET /erddap/griddap/GLSEA_ACSPO_GCS.json?sst[(2026-11-07T12:00:00Z)][(42.80):1:(42.90)][(-79.00):1:(-78.90)]
```

Response: ERDDAP table JSON, `columnNames` `time` (ISO UTC), `latitude`, `longitude`, `sst`; `columnUnits`
`UTC`, `degrees_north`, `degrees_east`, `degree_C`; grid spacing about 0.014 degrees; missing value
`_FillValue = -99999.0` (rows with the fill value appear as `null` in JSON). Coverage: latitude 38.87 to
50.61, longitude -92.42 to -75.88.

## OpenStreetMap Overpass

Verdict: **usable with caveats** (2026-11-08).

- Public instance: `https://overpass-api.de/api/interpreter` (GET or POST with form field `data`).
  Other public instances are listed on the OSM wiki (`overpass.private.coffee`, regional servers); none
  tried.
- Auth: none. Usage policy (OSM wiki "Overpass API" page and the Overpass documentation): fewer than
  about 10,000 queries and 1 GB per day on the main instance, and "for regular use, divide those numbers
  by 100"; app usage counts toward the total of all its users; cache and rate-limit; pause 30 seconds after
  HTTP 429 or 406; no parallel scripts; apps should send a `User-Agent` or `Referer` that identifies them;
  slot-based limits with HTTP 429 (rate) and 504 (resources); default query timeout 180 s and memory
  512 MiB (use `[timeout:..]` and `[maxsize:..]`). `/api/status` timed out once at 20 s.
- On first try the server answered HTTP 502 with a body saying it is at capacity and that "This public
  endpoint is intended for browser-based and human-operated tools. For automated or high-volume use, run a
  local Overpass API instance." The same query succeeded 20 seconds later (HTTP 200, 7 KB).
- License: ODbL 1.0; the response itself carries `osm3s.copyright` ("The data included in this document is
  from www.openstreetmap.org. The data is made available under ODbL."). Attribution: "(c) OpenStreetMap
  contributors". Derived databases that are redistributed must stay under ODbL.

Request (fixture `overpass-nyharbor-slipways-piers.json`, 30 elements around New York Harbor):

```text
[out:json][timeout:25];
(nwr["leisure"="slipway"](40.62,-74.12,40.76,-73.95);
 nwr["man_made"="pier"]["name"](40.69,-74.03,40.73,-73.98););
out center tags 30;
```

Response: `version`, `generator`, `osm3s.timestamp_osm_base` (UTC), and `elements[]` with `type`
(`node|way|relation`), `id`, `lat`/`lon` for nodes or `center` for ways, and `tags`. Tags seen include
`man_made=pier`, `leisure=slipway`, `name`, `access`, `fishing`. Elements carry no user names because
`meta` was not requested.

Caveat: direct requests from every phone would count toward one shared quota. Plan on a regional cache or a
self-hosted or paid instance before shipping this to users.

## PAD-US (USGS)

Verdict: **usable** (2026-11-08).

- USGS publishes PAD-US 4.1 (DOI 10.5066/P96WBCHS) and lists nine ArcGIS Online feature layer views on
  its web-services page. The REST URL is not printed on that page; it comes from the ArcGIS item
  (`https://www.arcgis.com/sharing/rest/content/items/c91a5655a1be428daeb778888e60db24`):
  `https://services.arcgis.com/v01gqwM5QqNysAAi/arcgis/rest/services/PADUS_Public_Access/FeatureServer`.
  The item was last modified 2026-07-30.
- Layer 0: polygons, `maxRecordCount` 2000, query formats `JSON, geoJSON, PBF`.
- Auth: none. Limits: none documented (ArcGIS Online throttling applies). Terms: the item's license text
  is a no-warranty disclaimer with a pointer to the PAD-US Data Overview for terms of use; USGS asks for
  the citation "U.S. Geological Survey (USGS) Gap Analysis Project (GAP), 2024, Protected Areas Database
  of the United States (PAD-US) 4.1, https://doi.org/10.5066/P96WBCHS". Public domain US government data.

Request (fixture `padus-public-access-croton-point.json`, envelope around Croton-on-Hudson, ten rows):

```text
GET .../PADUS_Public_Access/FeatureServer/0/query?geometry=-73.90,41.17,-73.86,41.23
    &geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects
    &outFields=Unit_Nm,Pub_Access,GAP_Sts,MngTp_Desc,MngNm_Desc,DesTp_Desc,ST_Name,GIS_Acres,BndryName
    &returnGeometry=false&resultRecordCount=10&f=json
```

Fields (layer 0): `Unit_Nm` unit name, `Pub_Access` public access, `GAP_Sts` GAP status code (1 to 4, as
text), `MngTp_Desc` and `MngNm_Desc` manager type and name, `DesTp_Desc` designation type, `Category`,
`FeatClass`, `IUCN_Cat`, `ST_Name`, `GIS_Acres`, `BndryName`. Geometry is Web Mercator (wkid 102100) unless
`outSR=4326` is requested. Rows seen include "Croton Gorge County Park" (`OA`, GAP status 4, county),
"Croton Gorge Unique Area" (`OA`, status 2, State Department of Conservation) and "Westchester Land Trust
Easement" (`XA`).

`Pub_Access` values seen are `OA`, `UK`, `XA`; the layer defines no coded-value domain, so the mapping
(open, unknown, closed, and restricted) comes from the field description and is **unverified** for the
two-letter codes. A single-point query at Croton Point returned no features, so use an envelope or a small
buffer. `exceededTransferLimit` was `true` at ten rows: page with `resultOffset`.

## GBIF occurrence API

Verdict: **usable with caveats** (2026-11-08).

- Base: `https://api.gbif.org/v1/occurrence/search`. Auth: none for searches. Responses carry
  `cache-control: public, max-age=600`.
- Limits (GBIF API documentation): rapid or numerous searches may receive HTTP 429, the threshold varies with
  load and no rate is guaranteed; for work expected to run over 15 minutes use the download API; set a
  `User-Agent` that holds a URL or email address. A maximum `limit` and maximum `offset` are not stated on the
  page read (**unverified**).
- License: set per record in the `license` field (CC0 1.0, CC BY 4.0, or CC BY-NC 4.0 in the sample).
  GBIF asks users to cite the data; the occurrence records repeat `datasetKey` for dataset-level
  citation.

Request (fixture `gbif-occurrence-striped-bass-nyharbor.json`):

```text
GET /v1/occurrence/search?scientificName=Morone%20saxatilis&decimalLatitude=40.4,41.0
    &decimalLongitude=-74.4,-73.6&limit=3&hasCoordinate=true
```

Response: `offset`, `limit`, `endOfRecords`, `count` (348), `results[]`. Fields the app would read:
`key`, `gbifID`, `scientificName`, `taxonKey`, `speciesKey`, `decimalLatitude`, `decimalLongitude`,
`coordinateUncertaintyInMeters`, `eventDate` (local time without offset), `year`, `month`, `day`,
`basisOfRecord` (`HUMAN_OBSERVATION`), `datasetKey`, `datasetName`, `license` (a URL), `media[]`
(each with its own `license`). The three sample records all come from "iNaturalist research-grade
observations" and are CC BY-NC 4.0. In the fixture, observer and rights-holder names were replaced
with `REDACTED`.

## iNaturalist API

Verdict: **usable with caveats** (2026-11-08).

- Base: `https://api.inaturalist.org/v1`. Auth: none for reads (JWT for writes). Official limits, from the
  API's own description (`/v1/swagger.json`): "we throttle API usage to a max of 100 requests per minute,
  though we ask that you try to keep it to 60 requests per minute or lower, and to keep under 10,000
  requests per day. If we notice usage that has serious impact on our performance we may institute blocks
  without notification." Terms: use is subject to the iNaturalist Terms of Service and Privacy Policy;
  "The API is intended to support application development, not data scraping."
- The web page "API recommended practices" returned HTTP 403 to a scripted client (**unverified**).
- Licenses are per observation (`license_code`) and per photo (`photos[].license_code`); a null value means
  all rights reserved. Photos on `inaturalist-open-data.s3.amazonaws.com` carry open licenses; photos on
  `static.inaturalist.org` do not. The API filter `license=cc-by,cc-by-nc,cc0` (and `photo_license=...`)
  restricts to reusable records. The first, unfiltered query returned an observation with no license.
- Coordinates: `obscured: true` observations carry a public coordinate that is spread over a cell of about
  27 km (`public_positional_accuracy` 27908 m in the sample). Do not draw these as exact points.
- The `fields` parameter I tried did not shrink the response: three observations were about 240 KB.
  Reduce the response in the adapter or find the supported syntax (**unverified**).

Request (fixture `inaturalist-observations-striped-bass-nyharbor.json`, reduced to the fields below):

```text
GET /v1/observations?taxon_name=Morone%20saxatilis&quality_grade=research
    &license=cc-by,cc-by-nc,cc0&photo_license=cc-by,cc-by-nc,cc0
    &nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&per_page=3&order_by=observed_on
```

| Field | Meaning |
|---|---|
| `total_results`, `page`, `per_page` | paging |
| `results[].id`, `uuid`, `uri` | identifiers and observation page |
| `observed_on` (date), `time_observed_at` (with offset), `observed_time_zone` | when |
| `geojson.coordinates` | `[lon, lat]`, possibly obscured |
| `positional_accuracy`, `public_positional_accuracy`, `obscured`, `taxon_geoprivacy` | location quality |
| `quality_grade` | `research` in this query |
| `license_code` | `cc-by`, `cc-by-nc`, `cc0` or null |
| `taxon.id`, `name`, `rank`, `preferred_common_name` | identification |
| `photos[].url`, `license_code` | `square.jpg` variant URL; swap the size name for other sizes (square 75 px, small 240, medium 500, large 1024, original 2048) |

## USGS Nonindigenous Aquatic Species (NAS)

Verdict: **usable with caveats** (2026-11-08).

- Base: `https://nas.er.usgs.gov/api/v2` (the documentation prints `http://`; HTTPS works). Documentation:
  `https://nas.er.usgs.gov/api/documentation.aspx`. Served through Cloudflare. Auth: none. Limits: none
  documented.
- Endpoints: `/occurrence/{key}` and `/occurrence/search`. Search parameters: `species_ID`, `group`, `family`,
  `genus`, `species`, `state`, `county`, `huc8`, `huc10`, `huc12`, `year` (range with a comma), `month`, `day`,
  `recordType`, `spatialAcc`, `yrAcc`, `refID`, `status`, `callback`, `format`; paging with `limit` and
  `offset`. **There is no bounding-box or radius parameter**: spatial filtering means state, county or
  HUC, then filtering by coordinate in the adapter.
- Terms: the site's data disclaimer says the data "vary in accuracy, scale, completeness, extent of
  coverage and origin" and asks that anyone using the data in a publication contact NAS. Suggested
  citation: "U.S. Geological Survey. [year]. Nonindigenous Aquatic Species Database. Gainesville, Florida.
  Accessed [date]."

Request (fixture `usgs-nas-occurrence-snakehead-ny.json`):

```text
GET /api/v2/occurrence/search?state=NY&genus=Channa&limit=3
```

Response: `endOfRecords` (string `"false"`), `count`, `offset`, `limit`, `results[]`. Fields: `key`,
`speciesID`, `group`, `family`, `genus`, `species`, `scientificName`, `commonName`, `state`, `county`,
`locality`, `decimalLatitude`, `decimalLongitude`, `latLongSource`, `latLongAccuracy`, `huc8`/`huc10`/`huc12`
with names, `date` (text, e.g. `2005-7`), `year`, `month`, `day` (null when unknown), `status`
(`established`, `collected`, ...), `comments`, `recordType`, `freshMarineIntro`, `UUID`, `references[]`.
Note: `endOfRecords` is a string, and numeric fields can be `null`. The sample returned northern snakehead
records in Queens (Meadow Lake, Flushing Meadows).

## New York State stocking data (Socrata)

Verdict: **usable with caveats** (2026-11-08). Both datasets exist and answer.

| Dataset | Title | Rows updated | Fields |
|---|---|---|---|
| `e52k-ymww` | Fish Stocking Lists (Actual): Beginning 2011 | 2026-06-24 (response header `X-SODA2-Truth-Last-Modified`) | `year`, `county`, `waterbody`, `town`, `month`, `number`, `species`, `size_inches` |
| `d9y2-n436` | Current Season Spring Trout Stocking | 2026-03-19 | `year`, `dec_region`, `county`, `town`, `waterbody`, `date`, `number`, `species_name`, `size_inches` |

- Base: `https://data.ny.gov/resource/<id>.json` (SoQL: `$where`, `$limit`, `$order`, `$select`).
  Metadata at `https://data.ny.gov/api/views/<id>.json`. Auth: none; an app token is optional. Throttling
  of requests without a token is described in Socrata's own documentation and was not tested
  (**unverified**). Responses are served through Socrata with `X-SODA2-*` headers.
- Attribution field: "NYSDEC" (`e52k-ymww`) and "NYS DEC" (`d9y2-n436`). Neither dataset has a license
  field. The portal's OPEN-NY Terms of Use (`https://data.ny.gov/dataset/OPEN-NY-Terms-Of-Use/77gx-ii52`,
  PDF) say that the terms contain no requirement to attribute, re-post the terms or obtain pre-approval,
  that the State grants a "non-exclusive, revocable license", that State seals and graphics are not
  licensed, and that content is provided without warranty. Crediting NYS DEC is still advisable.
- Dataset limitations quoted from metadata: "The Fish Stocking Lists are updated approximately mid-May the
  year following the actual stocking. The number of stocked fish is an estimate, not an actual count."
  The latest year in `e52k-ymww` is 2025 (the 2026 stocking will appear about mid-May 2027). `d9y2-n436`
  is "planned" stocking, updated each mid-March for the current spring (latest year 2026); its
  limitation text says actual stocking may differ. Neither can say what was stocked this autumn.
- Format quirks: every value is a JSON string, including `number` and `size_inches`; `month` is an English
  month name; in `d9y2-n436`, `date` is free text ("March - third week") and `size_inches` is text with
  units ("8 inches", "12 -14 inches"), while in `e52k-ymww` it is a numeric string. **No coordinates in
  either dataset**: a spot must be matched by county and waterbody name, so a geocoded waterbody
  catalogue is a prerequisite.

Requests (fixtures `socrata-ny-e52k-ymww-westchester-2025.json`, `socrata-ny-d9y2-n436-westchester.json`):

```text
GET https://data.ny.gov/resource/e52k-ymww.json?$where=county='Westchester' AND year=2025&$limit=5&$order=month
GET https://data.ny.gov/resource/d9y2-n436.json?$where=county='Westchester'&$limit=5
```

The sample rows are Kensico Reservoir, Pocantico River and Dogwood Pond (actual, April 2025), and Cross
River Reservoir (planned, March 2026).

## Copernicus Marine WMTS

Verdict: **usable with caveats** (2026-11-08). No account was created.

- The WMTS service does **not** require a login: `GetCapabilities` and `GetTile` both returned HTTP 200 to
  anonymous requests.
- Base: `https://wmts.marine.copernicus.eu/teroWmts`, KVP encoding only, tile matrix set `EPSG:3857`
  (`TILEMATRIX` is the plain zoom number), tiles are 256 x 256 PNG with
  `cache-control: max-age=2592000` (30 days) and `access-control-allow-origin: *`.
- Capabilities: the service-wide document (`.../teroWmts?SERVICE=WMTS&REQUEST=GetCapabilities`) is about
  **70 MB**; do not fetch it at run time. Per-dataset documents are small, for example (73 KB):
  `.../teroWmts/GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy_anfc_0.083deg_P1D-m_202406?SERVICE=WMTS&REQUEST=GetCapabilities`.
  Layer identifiers have the form `<product>/<dataset>/<variable>`; `TIME` dimension for that dataset is
  `2022-06-01T00:00:00Z/2026-11-17T00:00:00Z/P1D` with default `2026-11-09T00:00:00.000Z`; styles are
  `cmap:viridis` (and others, with `,logScale` variants); legends via `GetLegend` (SVG or JSON);
  `GetFeatureInfo` returns JSON, HTML, XML or GML.
- That 2D daily dataset exposes `zos` (sea surface height), `mlotst`, `tob`, `sob`, `pbo` and sea-ice
  variables, but no sea surface temperature or current layers. Those are in other datasets of the same
  product (**unverified**, not listed).
- Example tile (fixture `copernicus-marine-wmts-zos-z5-9-12-2026-11-08.png`, the New York bight at zoom 5):

```text
GET /teroWmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0
    &LAYER=GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy_anfc_0.083deg_P1D-m_202406/zos
    &STYLE=cmap:viridis&FORMAT=image/png&TILEMATRIXSET=EPSG:3857
    &TILEMATRIX=5&TILEROW=12&TILECOL=9&TIME=2026-11-08T00:00:00.000Z
```

- Terms (Copernicus Marine licence page): free of charge, worldwide, non-exclusive, perpetual; value-added
  work must credit "Generated using E.U. Copernicus Marine Service Information; <DOI links>", and
  redistribution of products "E.U. Copernicus Marine Service Information; <DOI links>", clearly visible on
  the page that gives access to the products. Licensees must keep usage records available on notice (a
  clause that passes to downstream licences), and the service-level text says products are free until
  30 June 2028. The licence text does not say whether anonymous WMTS access is covered by the same
  terms (**unverified**).
- Subsetting through the Copernicus Marine Toolbox or the data API still needs an account; it was not
  tested because registering was out of scope (**unverified**).
- Not for navigation: the products are modelled fields, not nautical data.

## DFO IWLS (Canada)

Verdict: **usable** (2026-11-08).

- Base: `https://api-iwls.dfo-mpo.gc.ca/api/v1`; Swagger UI at `.../swagger-ui/index.html`. Auth: none.
- Limits (Canadian Hydrographic Service web-services page, `tides.gc.ca`): up to 3 requests per second and
  30 requests per minute per client IP, HTTP 429 beyond. Maximum range per request: 1 week for 1-minute
  data, 3 weeks for 3-minute data, 1 month for all other resolutions. Quality flags: `1` good, `2` not
  evaluated or unknown, `3` questionable or suspect.
- Terms: the page says the service is available "free of charge under license" and refers to the Canadian
  Hydrographic Service licence agreement. No attribution text is given and the agreement text was not read
  (**unverified**). A credit line "Canadian Hydrographic Service, Fisheries and Oceans Canada" is advisable.
- No New York coverage; the examples use Vancouver (station 07735).

Requests (the matching test fixtures are synthetic, see `tests/fixtures/raw/dfo-iwls/README.md`):

```text
GET /api/v1/stations?code=07735
GET /api/v1/stations/5cebf1de3d0f4a073c4bb943/data?time-series-code=wlp-hilo
    &from=2026-11-08T00:00:00Z&to=2026-11-09T12:00:00Z
GET /api/v1/stations/5cebf1de3d0f4a073c4bb943/data?time-series-code=wlo
    &from=2026-11-08T12:00:00Z&to=2026-11-08T13:00:00Z&resolution=FIFTEEN_MINUTES
```

| Field | Meaning |
|---|---|
| station `id`, `code`, `officialName`, `latitude`, `longitude`, `provinceCode`, `operating`, `type` | station record |
| station `timeSeries[]` | `code` (`wlo` observed, `wlp` predictions, `wlp-hilo` high/low predictions), `id`, `nameEn`, `nameFr` |
| data `eventDate` | ISO 8601 UTC |
| data `value` | metres above chart datum (**unverified** against docs: the page read does not state the datum) |
| data `qcFlagCode` | string `"1"`, `"2"`, `"3"` as above |
| data `reviewed` | boolean |
| data `timeSeriesId` | joins to the station's `timeSeries[]` |

## ECCC hydrometric (Canada)

Verdict: **usable** (2026-11-08).

- Base: `https://api.weather.gc.ca/collections/hydrometric-realtime/items` (OGC API Features served by
  pygeoapi 0.20.0; the service root lists `hydrometric-stations`, `hydrometric-realtime`,
  `hydrometric-daily-mean`, `hydrometric-monthly-mean`, `hydrometric-annual-statistics` and
  `hydrometric-annual-peaks`; `https://api.weather.gc.ca/collections?f=json` is about 320 KB).
- Auth: none. Limits: none stated in the licence or hydrometric pages read. Real-time data cover the last
  30 days, from over 2,100 stations; update frequency is not stated on the page, and the recorded series is
  in 5-minute steps.
- License: Environment and Climate Change Canada Data Services End-use Licence (version 2.1.1 on the
  licence page): worldwide, royalty-free, perpetual, non-exclusive, commercial use included. Attribution:
  "Data Source: Environment and Climate Change Canada", or, where several attributions are impractical,
  "Contains information licenced under the Data Server End-use Licence of Environment and Climate Change
  Canada." Link to the licence where possible.
- No New York coverage; the example is the Bow River at Calgary (station `05BH004`).

Request (fixture `eccc-hydrometric-realtime-05BH004.json`):

```text
GET /collections/hydrometric-realtime/items?STATION_NUMBER=05BH004&f=json&limit=4&sortby=-DATETIME
```

| Property | Meaning |
|---|---|
| `STATION_NUMBER`, `STATION_NAME`, `PROV_TERR_STATE_LOC` | station |
| `DATETIME` | UTC, ISO 8601 with `Z` |
| `DATETIME_LST` | local standard time with offset (`-07:00`) |
| `LEVEL` | water level (metres by ECCC convention; **unverified**: the page read does not print units) |
| `DISCHARGE` | discharge (m3/s by ECCC convention; **unverified**) |
| `LEVEL_SYMBOL_EN/FR`, `DISCHARGE_SYMBOL_EN/FR` | data qualifiers, `null` when none |
| geometry | Point `[lon, lat]` |

## Water Quality Portal

Verdict: **usable with caveats** (2026-11-08): history and lake profiles, not live data.

- Base: `https://www.waterqualitydata.us/data/{Station|Result}/search` (legacy WQX 2.2 profiles), CSV with
  `mimeType=csv`, `zip=no`. A `HEAD` request returns counts in the headers `Total-Site-Count`,
  `Total-Activity-Count`, `Total-Result-Count`: use it to size a download before fetching it.
- Auth: none. Rate limits and license or citation text are not on the documentation page read
  (**unverified**). Request-size limit: long queries should be POSTed with a JSON body.
- **Important change:** the documentation says USGS data added since 2024-03-11 are available only in the
  WQX 3.0 profiles ("the beginning of limited accessibility for USGS data" in the legacy ones). The WQX 3.0
  route `https://www.waterqualitydata.us/wqx3/Result/search?...&dataProfile=basicPhysChem` returned HTTP 200
  with a CSV header and then timed out after 60 seconds, so its behaviour is **unverified**. A parser should
  target the WQX 3.0 profile once it is confirmed and treat the legacy profile as frozen at about March 2024
  for USGS.
- Hudson and Croton coverage: 54 monitoring locations with water temperature results within
  -74.00,41.15,-73.80,41.30 (fixture `wqp-stations-hudson-croton-temperature.csv`), but only one
  temperature result since 2022-01-01 in the legacy profile (fixture
  `wqp-results-hudson-croton-temperature-since-2022.csv`: 26.4 deg C at USGS-411729073562401 on
  2023-07-19). Station `USGS-01374310` (Hudson River at Peekskill) returned no temperature results for
  2015-2020 or 2025.

Requests:

```text
GET /data/Station/search?bBox=-74.00,41.15,-73.80,41.30&characteristicName=Temperature, water&mimeType=csv&zip=no
GET /data/Result/search?bBox=-74.00,41.15,-73.80,41.30&characteristicName=Temperature, water
    &startDateLo=01-01-2022&mimeType=csv&dataProfile=narrowResult&zip=no
```

Result columns used: `ActivityStartDate` (YYYY-MM-DD), `ActivityStartTime/Time`,
`ActivityStartTime/TimeZoneCode` (`EST`), `MonitoringLocationIdentifier`, `CharacteristicName`,
`ResultMeasureValue` (text), `ResultMeasure/MeasureUnitCode` (`deg C`), `ResultStatusIdentifier`
(`Preliminary`), `USGSPCode` (`00010`), `ProviderName`. Dates in the query are `MM-DD-YYYY`.

## USGS 3DHP (National Map hydrography)

Verdict: **usable** (2026-11-08). Geometry, flow direction, waterbodies and query limits are in `docs/providers/hydro.md`.

- Base: `https://hydro.nationalmap.gov/arcgis/rest/services`. Services listed: `3DHP_all`
  (`FeatureServer` and `MapServer`), `nhd`, `NHDPlus_HR`, `wbd` (`FeatureServer` and `MapServer`).
  `3DHP_all/MapServer` layers: 20, 30, 40 hydrolocations; **50 Flowline**; **60 Waterbody**; 80 Catchment;
  and spatial-metadata layers. `maxRecordCount` 2500. Credits line: "USGS TNM / NGTOC - 3D National
  Hydrographic Program (3DHP.) Data refreshed September 4, 2026."
- Auth: none. Limits: none documented. Terms (service description): "Use Constraints: None. All data are
  open and non-proprietary", with the caveats that features may not reflect current conditions and the data
  should not be used for critical applications or site-specific regulatory determinations;
  "Acknowledgment of the U.S. Geological Survey would be appreciated."

Request (fixture `usgs-3dhp-flowline-croton.json`, flowlines named in GNIS near Croton Gorge):

```text
GET /arcgis/rest/services/3DHP_all/MapServer/50/query?where=gnisidlabel IS NOT NULL
    &geometry=-73.87,41.20,-73.84,41.23&geometryType=esriGeometryEnvelope&inSR=4326
    &spatialRel=esriSpatialRelIntersects
    &outFields=id3dhp,gnisid,gnisidlabel,featuretypelabel,lengthkm,streamorder,mainstemid,hydrosequence,dnhydrosequence
    &returnGeometry=false&resultRecordCount=5&f=json
```

Fields: `id3dhp` (feature id), `gnisid` and `gnisidlabel` (GNIS name, "Croton River" for the main stem and
"Purdy Brook" for a tributary), `featuretypelabel` ("Channel Line", "Waterbody Connector", "Surface
Connector"), `lengthkm`, `streamorder` (7 for the Croton River reach), `mainstemid` (a geoconnex.us URI that
links to the mainstem reference), `hydrosequence`/`dnhydrosequence`/`uphydrosequence` and `levelpath` (for
upstream and downstream tracing), `featuredate` (epoch milliseconds). Many flowlines are unnamed
(`gnisidlabel` null); filter on it to find the named river.
