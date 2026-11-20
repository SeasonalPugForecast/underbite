# Swimming: rip currents, beach water quality, lifeguards, wind and waves, UV

Checked on 2026-11-20 (UTC, 22:00 to 22:20) with `curl` and the sources' own pages. Fixtures: `tests/fixtures/raw/swimming/` (index in its `README.md`). About 90 requests in all, one at a time, none repeated more than twice. `User-Agent: Underbite-scout/0.1 (data source check)` was sent; no key, account or cookie was used. Test places: New York Harbor, Rockaway (40.58, -73.82), Sandy Hook (40.43, -73.98) and Chicago (41.90, -87.62).

**Season matters.** The check ran out of season. Every New York, New Jersey and Chicago beach product was empty or had stopped in September or October 2026, so no in-season value was seen for the test places. In-season field values are known only from Boston (Boston still issues a surf forecast in November). Anything that depends on in-season behaviour is marked **unverified**.

## Summary

| Item | Source | Verdict |
|---|---|---|
| Rip current risk by beach zone, today and tomorrow | NWS Marine Beach Forecast map service (ArcGIS REST) | **usable**; the best structured source; empty out of season |
| Rip current risk, text | `api.weather.gov` SRF product and `Rip Current Statement` / `Beach Hazards Statement` alerts | **usable**; the alerts fit the existing alert path; SRF text needs a parser; seasonal |
| Hourly rip current probability (NWPS) | `polar.ncep.noaa.gov/nwps` | **not usable**: no data API, images and an undocumented station list only; the sample was 11 days old |
| NWS experimental beach pages JSON | `weather.gov/source/<wfo>/beachforecast/` | **don't use**: undocumented, stale |
| Beach bacteria, national | EPA BEACON 2.0 and the Water Quality Portal | **partly usable**: Water Quality Portal has an API but lags months; BEACON has no documented API |
| Beach bacteria, New York City | NYC Open Data `2xir-kwzz` | **usable as history only**: no coordinates, no advisory status, April to September |
| Beach bacteria, New York State parks | `data.ny.gov` `wwwd-za77` | **usable** (Jones Beach, Robert Moses and others), with `beach_status`; no coordinates |
| Beach bacteria, New Jersey | `njbeaches.org` | **unverified**: only a form-driven download endpoint was found; its parameters were not confirmed |
| Chicago beaches | `data.cityofchicago.org` | **usable in season**: sensors hourly, lab results daily; the advisory datasets have stopped |
| Guarded beaches | OSM through Overpass | **hint only**: lifeguard tags are rare and nameless |
| Wind, pressure and wave map layers | nowCOAST, NDFD, WaveWatch III, Open-Meteo | no change since `gridded-weather.md`; **use the PacIOOS ERDDAP grids** |
| UV index | Open-Meteo `uv_index` | **use**; the EPA UV API is keyed by ZIP code or city, **don't use** |

Safety wording is covered at the end.

## 1. Rip current risk

### 1.1 NWS Marine Beach Forecast map service

The structured form of the surf zone forecast. An Esri ArcGIS Server 11.3 `MapServer` with no key.

| Item | Value |
|---|---|
| Base | `https://mapservices.weather.noaa.gov/vector/rest/services/outlooks/marine_beachforecast/MapServer` (a sibling `marine_beachforecast_summary` exists and was not read) |
| Layers | 129. A group layer per WFO, each with `<WFO> Day 1` and `<WFO> Day 2` polygon layers. Day 1 is 1200 UTC today to 1200 UTC the next day, Day 2 the following 24 hours. New York (OKX) group is layer 90 (Day 1 is 91, Day 2 is 92), Philadelphia/Mount Holly (PHI) 96 (97, 98), Chicago (LOT) 15 (16, 17), Boston 0 (1, 2). The last group, 126, is `No_Beach_Forecast_WFO` (layers not read). |
| Capabilities | `Map,Query,Data`; `maxRecordCount` 2000; no key; no rate-limit header seen |
| Geometry | one polygon per NWS public forecast zone along the shore. OKX Day 1 has 5 polygons, PHI 5, LOT 5, a sample of layer 22 has 4 |
| Auth | none |
| Licence | the service metadata `copyrightText` reads "NOAA, National Weather Service, NWS's Marine, Tropical and Tsunami Services Branch". NWS content is public domain (see `nws-marine.md`); no further terms were found |

Fields on each Day layer (all strings, `null` when the office has no current product):

| Field | Meaning, example (Boston, 2026-11-20) |
|---|---|
| `id` | the public zone id in lower case: `maz016`, `nyz178`, `njz014`, `ilz104` |
| `beachname` | `Eastern Norfolk Area Beaches`, `Southern Queens Beaches`, `Eastern Monmouth Beaches`, `Central Cook Area Beaches` |
| `siteid`, `sitename` | `box`, `WFO Boston/Norton MA` style |
| `rip` | rip current risk: `Low` was seen; the categories in the product are Low, Moderate, High |
| `uv`, `surf`, `wtemp`, `maxtemp`, `weather`, `winds`, `tstorm`, `wspout` | text from the forecast: `Low`, `Around 2 feet.`, `in the lower 50s` |
| `period` | `Rest of today` |
| `productdat`, `producttim` | `11/20/2026`, `409 PM EST` (local text of the product) |
| `srfprod` | link to the text product on `forecast.weather.gov` |
| `idp_ingestdate` | epoch milliseconds of the ingest, UTC (`1795209453000` is 2026-11-20 21:37 UTC for Boston) |

The zone ids are those in the SRF text (`MAZ022-`), so one zone key joins the map service, the text product and the alerts' `affectedZones`.

Request used (Rockaway, point in polygon, coordinates rounded to two decimals):

```text
GET <base>/91/query?f=json&geometry=-73.82,40.58&geometryType=esriGeometryPoint&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=id,beachname,siteid,rip,uv,surf,wtemp,period,srfprod,idp_ingestdate&returnGeometry=false
```

Answers for the test places (all 2026-11-20):

| Place | Layer | Zone | Values | Ingest |
|---|---|---|---|---|
| Rockaway | 91 (OKX) | `nyz178` Southern Queens Beaches | all fields `null` | 2026-09-28 11:57 UTC |
| Sandy Hook | 97 (PHI) | `njz014` Eastern Monmouth Beaches | all `null` | 2026-10-01 23:27 UTC |
| Chicago (41.90, -87.62) | 16 (LOT) | none at the point; `distance=3000&units=esriSRUnit_Meter` returns `ilz104` Central Cook Area Beaches | all `null` | 2026-10-01 |

Response size 1.0 to 1.3 KB per point query. Notes:

- The zones follow the shoreline, so a point a little offshore or inland misses; use a search distance of a few kilometres or take the nearest zone. A dropped marker 3 km from the coast would match the same zone.
- Rounded points work (the requests above used two decimals); the whole zone polygon would also work as a bounding-box query and keeps the exact point on the device.
- Out of season the zone still exists but the values are `null`. Treat `null` as "no forecast issued", never as "low".
- The service and layer ids change with the WFO list; read `?f=json` at startup or key by `siteid` and `Day 1` name.
- Updated by the WFO whenever the surf zone forecast is issued (Boston: 21:09 UTC product, 21:37 UTC ingest).
- In-season values for New York, New Jersey and Chicago: **unverified** (out of season).

### 1.2 api.weather.gov: SRF products and alerts

Base, headers, cache and `User-Agent` handling are in `nws-marine.md`.

- `GET /products/types/SRF` returned 346 products from 19 issuing offices, 133 KB, newest 2026-11-20 21:22 UTC (San Diego), oldest 2026-11-13. Cache: `max-age=115, s-maxage=120`.
- `GET /products/types/SRF/locations` lists the 19 location ids: BOX, BRO, CAR, CRP, EKA, GUM, HFO, JAX, LOX, MFL, MFR, MLB, MOB, MTR, PQR, SGX, SJU, TAE, TBW.
- **OKX, PHI, LOT and AKQ are not in the list**, and `GET /products/types/SRF/locations/OKX` returned an empty `@graph`. These offices stop issuing the product after the summer season (New York's last `beaches.json` write was 2026-09-27). The product reappears in the season; its dates are **unverified**.
- `GET /products/types/SRF/locations/BOX` gave 13 products (5 KB); `GET /products/{id}` gave one (21 KB, `productText`).
- The product text (WMO `FZUS51 KBOX`) has one block per public zone, with the zone line `MAZ022-211100-`, the beach list ("Including the beaches of Chatham, Eastham, and Provincetown"), and for each period `Rip Current Risk*`, `Surf Height`, `Thunderstorm Potential`, `UV Index`, `Water Temperature`, `Weather`, `High Temperature`, `Winds` and `Tides`. Risk values `Low`, `Moderate`, `High` with a footer that says "The risk for rip currents is low, however, life-threatening rip currents often occur in the vicinity of groins, jetties, reefs, and piers." The Beach Forecast map service above carries the same values already split into fields, so the text parser is not needed for the first version.
- Alerts: `Rip Current Statement`, `High Surf Advisory` and `Beach Hazards Statement` are event names in `/alerts/types` (see `nws-marine.md`). On 2026-11-20 `GET /alerts/active?event=Rip%20Current%20Statement` returned 10 alerts (62 KB), all Florida, Puerto Rico and the Texas coast; `High Surf Advisory` 8 and `Beach Hazards Statement` 1 (not stored). `GET /alerts/active?point=` for Rockaway, Sandy Hook and Chicago returned no alerts.
- A Rip Current Statement carries `severity: Moderate`, `description` ("* WHAT...High Rip Current Risk, life-threatening rip current. * WHERE... * WHEN... * IMPACTS...") and an `instruction` with escape advice. The description is the wording to show, with the sender.
- Chicago and the Great Lakes: LOT issues `Beach Hazards Statement` and `High Surf Advisory` in the swimming season; none was active, so the Great Lakes wording is **unverified**.

### 1.3 NWPS rip current probability (NOAA/NWS experimental model)

- The probabilistic rip current model runs with the Nearshore Wave Prediction System. It gives a probability from 0 to 100 percent, hourly out to 144 hours, for stations along the coast of the 31 WFOs listed in the viewer source; the NWS Wilmington surf matrix page (`https://www.weather.gov/ilm/surfmatricesinfo`) says the model "is known to have a high bias" and links the product description document `https://www.weather.gov/media/ilm/PDD_ExpSurfForecastMatrix_Ext2024.pdf` (not read).
- Access: the NWPS viewer `https://polar.ncep.noaa.gov/nwps/viewer.shtml` (Esri JavaScript, source `https://polar.ncep.noaa.gov/nwps/js/esri_viewer_20251207.js`). It reads, per office, a text file `https://polar.ncep.noaa.gov/nwps/images/rtimages/validation/<WFO>1.rip` and, per station, a PNG `.../validation/nwps_<wfo>_ripprob_stat<N>.png`. Nothing else was found: no THREDDS, ERDDAP, GeoJSON, JSON or GRIB for the rip probability. A search of NOAA's pages found no such endpoint either.
- `OKX1.rip` (51 KB) has 673 rows such as `"okx_ripprob_stat623|40.579|-73.817|Rip Current Station 623|OKX|red|HIGH",`; `PHI1.rip` has 131 rows. The last two fields are a colour and a category (`gray`/`LOW`, `yellow`/`MODERATE`, `red`/`HIGH`); what time the category refers to is not stated. In the OKX file 310 stations were HIGH, 81 MODERATE and 282 LOW; in the PHI file 120 were HIGH and 11 MODERATE.
- Nearest stations: Rockaway station 623 at 40.579, -73.817 (0.3 km); Sandy Hook station 129 (PHI) at 40.433, -73.980 (0.3 km). `LOT1.rip` answered 404: the Great Lakes are not covered. The viewer source lists 31 offices for rip output: TBW, MFL, MHX, BOX, SGX, HFO, SJU, AKQ, OKX, CAR, MLB, JAX, CHS, ILM, PHI, GYX, KEY, TAE, MOB, HGX, GUM, BRO, CRP, LCH, LIX, LOX, MTR, EKA, MFR, PQR, SEW. No Great Lakes office is among them.
- The time series for station 623 is an image (166 KB). Its title read "NWPS WFO-OKX: Rip Current Probability 2026/11/09 12Z" although the file's `Last-Modified` was 2026-11-20 21:57 GMT: the plotted run was 11 days old. Thresholds in the plot: moderate risk at 25 percent, high risk at 50 percent. Whether the `.rip` colour comes from the same old run is **unverified**.
- Verdict: **not usable**. There is no documented machine-readable feed, the observed output was stale, the model has a stated high bias, and the files are internals of a viewer. If NWS publishes a data endpoint, re-check; the Beach Forecast map service (1.1) is the supported route to the forecasters' own rip risk.

### 1.4 weather.gov beach pages (experimental)

`https://www.weather.gov/source/<wfo>/beachforecast/beaches.json` and `config.json` feed the "Experimental Beach Forecast Webpage" (`https://www.weather.gov/beach/okx` redirects to `/beach/forecast?site=okx`). `config.json` lists beach points (OKX 18 beaches such as `Rockaway Beach 40.584490,-73.810195`; LOT 4: Waukegan Municipal, North Avenue Beach, Lake Street Beach, Indiana Dunes State Park); `beaches.json` has `riprisk`, `uvindex`, `surftemp` per beach key. The OKX `beaches.json` had `Last-Modified: Sun, 27 Sep 2026 08:57:51 GMT` and still said `"riprisk": "high"`, `"surftemp": "lower 70s"` in November. Undocumented, stale after the season, no terms: **don't use**. The Beach Forecast map service carries the same data with a timestamp and `null` for no product.

## 2. Beach water quality and advisories

Every source here is seasonal and every one lags. None gives a verdict that water is safe.

### 2.1 EPA BEACON 2.0 and the Water Quality Portal

- BEACON 2.0 (`https://beacon.epa.gov/ords/beacon2/r/beacon_apex/beacon2/`) is a web application. EPA's BEACON pages describe a user guide, a quarterly download of the beach locations ("locational dataset") and monitoring data taken from the Water Quality Portal. No REST API for BEACON was found; the old `https://watersgeo.epa.gov/beacon2/` answered 403. Notification data (advisories, closures) comes from the states, most of which report to EPA once a year, so it is not current. EPA's catalogue lists the update frequency as irregular. The dataset page `https://catalog.data.gov/dataset/beach-advisory-and-closing-online-notification-beacon-system` was not opened in full; the locational dataset's file format and licence are **unverified**.
- Water Quality Portal (WQP): `https://www.waterqualitydata.us/data/{Station|Result}/search?...`, no key. Request used:

```text
GET https://www.waterqualitydata.us/data/Result/search?bBox=-74.05,40.40,-73.75,40.62&characteristicName=Enterococcus&startDateLo=08-25-2025&startDateHi=09-05-2025&mimeType=csv&zip=no&dataProfile=narrowResult
```

  200, 22 KB, 46 rows (CSV, `dataProfile=narrowResult`: `OrganizationIdentifier` `21NYBCH` "New York State Department of Health", `ActivityStartDate`, `MonitoringLocationIdentifier`, `CharacteristicName` `Enterococcus`, `ResultMeasureValue` `171.0`, `ResultMeasure/MeasureUnitCode` `MPN`, `ResultDetectionConditionText` `Present Below Quantification Limit` when no value). `HEAD` returns `Total-Site-Count`, `Total-Activity-Count` and `Total-Result-Count`, so a count costs nothing. Counts for the NY/NJ box, characteristic `Enterococcus`: 2026-01-01 on: 14 results at 7 sites; 2025-01-01 on: 613 at 50 sites; 2024-01-01 on: 1,208 at 52; 2020-01-01 on: 2,996 at 59. Since 2026-09-01: 0. So the portal trails the sampling season by months and cannot show today's water.
- A bounding box and a date range work; there is no point query (use a small box). Per-organisation and per-county filters exist (not tried). No rate limit or header was seen; the portal's usage policy and licence text were not read: **unverified**. The data are agency monitoring results ("provided as is").
- Verdict: **history and a map of stations only**. Not a live advisory source.

### 2.2 New York City (DOHMH)

- `https://data.cityofnewyork.us/resource/2xir-kwzz.json` ("Beach Water Samples", Socrata SODA, no key, `$limit`, `$order`, `$where` work). Fields: `sample_id`, `sample_date` (local date, no time), `beach_name`, `sample_location` (`Left`, `Center`, `Right`), `enterococci_results` (absent when no result), `units_or_notes` (`MPN/100 ml`). Newest sample 2026-09-29; the description says inspections run April through September. Updated 2026-10-05 (catalogue). 48 distinct beach names, including nine Rockaway segments (`ROCKAWAY BEACH 59TH - 80TH`), Coney Island, Orchard Beach, South Beach and many private clubs; spelling varies (`BREEZY POINT - 219TH STREET.`, `SEA GATE 42ND` and `SEAGATE BEACH - 42ND STREET`).
- **No coordinates and no advisory or closure status** in the dataset. The city's advisory page and map (`https://a816-dohbesp.nyc.gov/IndicatorPublic/beaches`) describe advisories; no API was found for them. Matching a spot to a beach needs a name table maintained by hand.
- The NYC beach season is Memorial Day to Labor Day (the dataset description says sampling April to September).
- Licence: NYC Open Data terms of use; the catalogue's `license` field is empty. Attribution text: **unverified**.
- Verdict: **usable only as recent results** next to a beach name the user picks; do not derive a verdict from it.

### 2.3 New York State parks

- `https://data.ny.gov/resource/wwwd-za77.json` ("Beach Water Testing: Beginning 2015", Socrata, no key). Fields: `region`, `state_park_beach`, `waterbody`, `category`, `date_sample_taken`, `results_modifier` (`<`, `=`), `results`, `indicator_bacteria` (`Enterococci`), `beach_status`, `status_reason`. Example: `Jones Beach SP Zach's Bay`, `Atlantic Ocean`, 2026-10-19, `<` 10, `Off - Season`, `Closed for end of season`. Latest samples on the Long Island parks: Jones Beach 2026-09-22 (Zach's Bay 2026-10-19), Robert Moses 2026-09-22, Hither Hills 2026-09-23, Orient 2026-09-09.
- `beach_status` is the only advisory-like field found (values seen: `Off - Season`; in-season values **unverified**).
- No coordinates. Publisher contact `news@parks.ny.gov` appears in the metadata. Licence field empty; open data terms of the state portal apply (not read).
- Sandy Hook is a National Park Service beach and is in none of the NY datasets.

### 2.4 New Jersey

- `https://njbeaches.org/` (NJDEP and Rutgers, a Next.js application behind Apache) describes two downloads in its page script: `/api/public/data/download/waterQuality` and `/api/public/data/download/incident`, both `GET` with `downloadType`, `program`, `county`, `municipality`, `lha`, `beachId`, `locationType`, `stationType`, `startDate` and `endDate`. One request, `.../download/incident/?downloadType=incident&county=Monmouth&startDate=2026-09-01&endDate=2026-09-30`, was redirected (308) to the path without the trailing slash and then answered 404 (HTML), so the parameter values the server expects were not found. No documentation, terms or licence was found. The state also sends monitoring data to EPA's WQX, so it appears in the Water Quality Portal (2.1) under a New Jersey organisation (**unverified**: 2.1 requests did not break results down by organisation).
- Status: **unverified**; no recorded fixture. Ask the NJDEP Bureau of Marine Water Monitoring for a documented feed before building on the form endpoint.

### 2.5 Chicago Park District (data.cityofchicago.org)

Socrata SODA 2, no key for these reads (an app token raises the throttle; not needed for a few requests). Response headers carried no rate-limit counters; Socrata's general policy is not read: **unverified**. Licence: the catalogue says `See Terms of Use` (the city's open data terms; attribution `Chicago Park District`).

| Dataset | Id | What | Freshness on 2026-11-20 |
|---|---|---|---|
| Beach Water Quality - Automated Sensors | `qmqz-2xku` | hourly `water_temperature` (°C), `turbidity` (NTU as stated in the data page; **unit unverified**), `wave_height` (m), `wave_period` (s), `transducer_depth`, `battery_life`, per `beach_name` (`Ohio Street Beach`) | catalogue `data_updated_at` 2026-11-20 21:45 UTC, but the newest measurement is `2026-09-25T23:00:00` (local, no zone); the data page says sensors run in summer only |
| Beach Lab Data | `2ivx-z93u` | qPCR DNA (`dna_sample_1_reading`, `dna_reading_mean`) and culture E. coli results, with beach coordinates | newest DNA sample `2026-09-07T11:53:00`; culture results end 2016-09-20 |
| Beach E. coli Predictions | `xvsz-3xcj` | predicted E. coli (CFU per 100 ml) by `beach_name`, `prediction_source`, with latitude and longitude | newest row 2025-09-01: not updated for a season |
| Beach Swim Advisories | `t62e-8nvc` | historical swim advisories (`swim_advisory` Y/N, `predicted_level`, `probability`) | historical-only; newest 2016-09-22 |
| Beach Water and Weather Sensor Locations | `g3ip-u8rb` | sensor positions | 2015; not requested |

- Missing-value marker in the sensor table: `-100000` (seen in `wave_height` and `wave_period`). Treat it as missing.
- Request used: `GET https://data.cityofchicago.org/resource/qmqz-2xku.json?$limit=5&$order=measurement_timestamp DESC` (1.4 KB). The `$where`, `$select`, `$group` clauses work as in SoQL. Filtering by place needs a name join to the lab data coordinates (`location` object in `2ivx-z93u`, `xvsz-3xcj`), since the sensor table has none.
- The lab-data description cites a rapid qPCR test and a US EPA recommended threshold of 1000 calibrator cell equivalents (the text is cut in the stored metadata); the park district decides on advisories, and the live advisory is shown on its beach page (`https://www.chicagoparkdistrict.com/parks-facilities/beaches`, not read). The structured advisory flag stopped in 2016 (`t62e-8nvc`) and the prediction table in 2025, so **there is no current machine-readable swim advisory** for Chicago.
- Verdict: **use the sensor water temperature and waves in season** (a better local temperature than any model); show lab and advisory data only as links to the park district.

## 3. Guarded and designated beaches (OSM)

Overpass main instance, one request at a time (two 502 answers came back from a cold start; waits of 40 to 60 s then succeeded; see `overpass.md` for the policy). Query (counts, rounded boxes):

```text
[out:json][timeout:60];
(nwr["leisure"="bathing_place"](S,W,N,E);)->.a; .a out count;
(nwr["natural"="beach"](S,W,N,E);)->.b; .b out count;
(nwr["natural"="beach"]["supervised"="yes"](S,W,N,E);)->.c; .c out count;
(nwr["natural"="beach"]["lifeguard"="yes"](S,W,N,E);)->.d; .d out count;
(nwr["emergency"="lifeguard"](S,W,N,E);nwr["emergency"="lifeguard_station"](S,W,N,E);nwr["lifeguard"]["lifeguard"!="no"]["natural"!="beach"](S,W,N,E);)->.e; .e out count;
(nwr["natural"="beach"]["swimming"="no"](S,W,N,E);)->.f; .f out count;
(nwr["natural"="beach"]["access"~"private|no"](S,W,N,E);)->.g; .g out count;
```

| Tag | NY/NJ ocean 40.40,-74.05,40.65,-73.70 | Chicago 41.65,-87.80,42.05,-87.50 |
|---|---|---|
| `leisure=bathing_place` | 0 | 0 |
| `natural=beach` | 122 | 62 |
| `natural=beach` + `supervised=yes` | 0 | 0 |
| `natural=beach` + `lifeguard=yes` | 0 | 2 |
| `emergency=lifeguard` or `lifeguard_station` (or `lifeguard=*` on other objects) | 12 | 5 |
| `natural=beach` + `swimming=no` | 0 | 0 |
| `natural=beach` + `access=private` or `no` | 1 | 10 |

(OSM database timestamp 2026-11-20T22:11:21Z for the NY/NJ request.) Findings from the 146 elements in `overpass-swim-tags-nynj-ocean.json`:

- Only 46 of the 122 beaches have a `name`; 85 have a `surface`. The beaches of the Rockaway peninsula within 3 km of Rockaway include one named `Rockaway Beach`; nearby ones are unnamed.
- The one `supervised` tag is `10:00-18:00` on `Coney Island Beach` (a `leisure=beach_resort`), not `yes`; a parser expecting `yes` would miss it. `lifeguard=no` occurs on one private beach.
- All 12 `emergency=lifeguard` objects are unnamed points or small ways: they are stands, not a statement about a beach. 13 `leisure=beach_resort` objects are mostly private clubs.
- Verdict: **don't present OSM as "guarded beach"** and don't draw a guarded or unguarded state from it. Use `natural=beach` for drawing the beach, `emergency=lifeguard` for optional stand markers marked "from OpenStreetMap, may be incomplete", and send people to the beach operator for lifeguard hours. OSM data are ODbL; credit "OpenStreetMap contributors" (see `overpass.md`).

## 4. Wind, pressure and waves on the map

Nothing in this re-check changes `gridded-weather.md`.

- `https://mapservices.weather.noaa.gov/raster/rest/services/NDFD` still holds only `NDFD_temp` (ArcGIS 11.3). The other raster folders listed 2026-11-20: `air_quality, climate, hazards, NDFD, obs, outlooks, precip, snow`; `raster/obs` holds `mrms_qpe, NWM_Land_Analysis, rfc_qpe, usnic_ims_snow_ice_1km`; `raster/outlooks` holds `winter_weather_outlook`; `vector/obs` holds `surface_obs` among six services, and `vector/outlooks` holds `marine_beachforecast`, `marine_beachforecast_summary` and others. No wind, pressure or wave service. `vector/obs/surface_obs` was not opened (it may carry observed wind; **unverified**).
- Recommendation unchanged: PacIOOS ERDDAP `ncep_global_lon180` for wind and sea-level pressure (0.5 degree, 3-hourly) and `ww3_global_lon180` for waves (0.5 degree, hourly), rendered on the device from griddap subsets; Copernicus Marine WMTS for finer waves. Neither was tested again here. Open-Meteo stays a per-spot source.
- Projection and time-dimension details for those services are in `gridded-weather.md` (griddap subsets in latitude/longitude, time axis in the dataset).

## 5. UV index

- Open-Meteo: `GET https://api.open-meteo.com/v1/forecast?latitude=40.58&longitude=-73.82&hourly=uv_index,uv_index_clear_sky&daily=uv_index_max&forecast_days=2&timezone=auto`. 200, 1.8 KB. The point was snapped to 40.59108, -73.81388 (the response says where). `hourly.uv_index` is a unitless float (`hourly_units.uv_index` is an empty string): 0.0 at night, 3.1 at 12:00 local on 2026-11-20; `daily.uv_index_max` 3.1 and 2.95. Rate limit headers on this endpoint: `X-RateLimit-Limit: 600.0`, `X-RateLimit-Window: 60`, `X-RateLimit-Cost: 1.0`. Licence, attribution and the free-tier limits are in `open-meteo-marine.md`. A two-decimal point works (the app's usual rounding). No key.
- EPA Envirofacts UV (`https://data.epa.gov/efservice/getEnvirofactsUVHOURLY/ZIP/10001/JSON`, 1.2 KB; `getEnvirofactsUVDAILY/CITY/New%20York/STATE/NY/JSON`, 85 bytes). Keyed by ZIP code or by city and state, no coordinates; the hourly answer carries `DATE_TIME` such as `Nov/20/2026 07 AM` (local, no zone) and an integer `UV_VALUE`. A ZIP code would be a different kind of location data from a rounded coordinate and would need a reverse lookup. **Don't use.** Limits and licence: not found, **unverified**.
- The NWS surf zone forecast also has a `UV Index` (words: Low, Moderate, ...), available through the Beach Forecast map service (1.1) in season.
- The UV index from a model is a clear-sky-plus-cloud estimate for the grid cell; it is not a statement about sunburn risk for a person.

## Privacy

- Beach Forecast map service: a point rounded to two decimals worked; a distance search lets the point be rounded further. Whole-zone layers can be fetched without sending any point.
- `api.weather.gov` alerts accept `?point=`; use the zone id from the map service instead of sending coordinates when possible (the existing note covers rounding).
- Overpass, WQP and Socrata take a bounding box; send the cell, not the spot.
- Open-Meteo: two-decimal rounding works.

## Safety wording

- Never show a beach, a water quality value or a rip current risk as "safe". "Low" rip current risk is the forecast's word; the NWS text itself adds that life-threatening rip currents often occur near groins, jetties, reefs and piers. Show the category with the forecaster's own caution, the issuing office and the issue time. `null` means no forecast, which must read "no forecast issued", not "low".
- Bacteria results are lagged point samples. Present them as "last sample, date, value, source" with the local health department as the authority and a link. Do not colour them green. Do not derive a swim advisory from a value.
- The NWPS probability has a stated high bias and was stale; it is not shown.
- OSM lifeguard tags are incomplete; absence of a tag is not absence of lifeguards, and presence is not hours of service.
- Great Lakes beaches have rip and structural currents too; the surf zone forecast covers them only in season (**unverified** for Chicago in season).
- Alerts are NWS products and carry their sender; display `event`, `headline` and the instruction text unaltered. These and the wording above are in the app's voice, not Luna's.

## Unverified

- In-season values and field spellings for OKX, PHI and LOT layers of the Beach Forecast map service, and the exact dates the offices start and stop issuing.
- Whether `Rip Current Statement` and `Beach Hazards Statement` are issued for the Chicago shore in the season.
- EPA BEACON's locational dataset URL, format and licence; the Water Quality Portal usage policy and licence text; whether New Jersey results are in the portal.
- The parameter values of the njbeaches.org download endpoints.
- NYC Open Data and Chicago Socrata rate limits and required attribution text.
- Units of the Chicago `turbidity` column.
- The `vector/obs/surface_obs` map service contents.
