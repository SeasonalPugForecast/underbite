# NOAA harmful algal bloom (HAB) forecasts

Verified on **2026-11-17** (UTC) with `curl` and the NCCOS forecast pages. This note extends the summary in `tier2.md`. Fixtures: `tests/fixtures/raw/hab/` (index in its `README.md`). Items that could not be checked are marked **unverified**.

Only the Gulf of Mexico respiratory irritation forecast (RIF) is a machine-readable feed. Every other region publishes a PDF bulletin or a viewer. No HAB product covers the New York test places.

## Gulf of Mexico respiratory irritation forecast

| Item | Value |
|---|---|
| URL | `https://nccospublicstor.blob.core.windows.net/hab-data/no_Explorer/rif_model/csv/RIFv2_currentmodelrun.zip` (Azure blob; no key; `Content-Type: application/octet-stream`) |
| Size, freshness | 16.9 KB; `Last-Modified: Tue, 17 Nov 2026 08:09:14 GMT` (the model run is 03:00 US Eastern, 08:00 UTC; the file appeared 9 minutes later). NOAA says the model runs every 3 hours. |
| Contents | 11 forecast CSV files `RIFv2_modelrun_YYYYMMDDHHMM-EST_forecast_valid_NNhrs.csv` with `NN` = 01, 04, 07, ..., 31, plus `RIFv2_data_dictionary.csv` and `RIFv2_README.txt` |
| Validity time | the `Wind Time` column: for the run 2026-11-17 03:00 EST (08:00Z), `valid_01hrs` is 08:00Z, `valid_04hrs` 11:00Z, `valid_13hrs` 20:00Z, `valid_31hrs` 2026-11-18 14:00Z. So the file `NN` is valid `NN - 1` hours after the run time (UTC), in 3-hour steps to 30 hours. |
| Rows | 22 beaches per file on 2026-11-17 (the earlier check on 2026-11-08 found about 55); 242 rows over 11 files |
| Authentication, limits | none stated. The file is the current run only; keep a local copy per run if a history is wanted. |
| Terms | none stated on the product page; NOAA content is public domain. Credit "NOAA National Centers for Coastal Ocean Science (NCCOS)" and, as the bundled README asks implicitly, the cell-count sources: Florida Fish and Wildlife Research Institute (FWRI), HABscope, Pinellas County, and the Sanibel-Captiva Conservation Foundation, with wind from the NOAA Meteorological Development Laboratory's National Digital Forecast Database. |

### Data dictionary (`RIFv2_data_dictionary.csv`, read in full) with what the file shows

Columns of the dictionary: Field, Description, Data type, Acceptable values, Required, Accepts null. All fields are required and none accepts null.

| Field | Type | Acceptable values (dictionary) | Observed 2026-11-17 |
|---|---|---|---|
| `Forecast` | string | `Very low`, `Low`, `Moderate`, `High` | only `Very low` |
| `Sample Used` | int | >= 1; "an artifact filler field from previous RIF Model version" | 0 to 21 (a row index; ignore) |
| `Cell Count Category` | string | `None`, `Very low`, `Low a`, `Low b`, `Medium`, `High` | `Not Present` (231) and `Very low` (11): **`Not Present` is not in the dictionary** |
| `Sample Time` | datetime | `YYYY-MM-DDTHH:MN:SS+00:00`, UTC; when the cell count was collected | 2026-11-11 to 2026-11-16 |
| `Wind Speed` | float | >= 0, knots, NDFD forecast | 18.0, 25.0 |
| `Wind Dir` | int | 0 to 360, degrees | floats such as 67.5 and 90.0 (not integers) |
| `Wind Category` | string | `Offshore`, `Light`, `Medium`, `High` | `Offshore` 176, `High` 58, `Medium` 8 |
| `Wind Time` | datetime | UTC, the time the forecast wind is valid | one value per file |
| `Shoreline Angle` | float | 0 to 360; "the 90 degree offshore angle for the beach location" | 75.3, 76.2, ... |
| `Lat` | float | 0 to 32 | 26.8 to 30.7 |
| `Lon` | float | -98.5 to -79.5 | -86.2 to -80.1 |
| `Confidence` | float | 0.0, 0.1 or 0.9; a weighting from the source and age of the sample | 0.9 (176 rows) and 0.0 (66 rows); 0.0 coincided with a `Medium` or `High` wind category in this file |
| `Location Name` | string | any | for example `Clearwater Beach`, `Longboat Pass Boat Ramp (Sarasota Bay)`, `Mullet Key; Gulf Pier`; names may contain `;` and parentheses and are quoted when they contain commas |
| `Model Run Time` | datetime | `YYYY-MM-DDTHH:MN:SS-05:00` "United States Eastern Time" | `2026-11-17T03:00:00-05:00` (a fixed -05:00 in November; the offset in summer is **unverified**: use the offset in the string) |
| `Cell Count Source` | string | `fwri`, `habscope`, `pinellas`, `sccf` | `fwri_sftp` (165) and `pinellas` (77): **`fwri_sftp` is not in the dictionary** |

Header row: `Forecast,Sample Used,Cell Count Category,Sample Time,Wind Speed,Wind Dir,Wind Category,Wind Time,Shoreline Angle,Lat,Lon,Confidence,Location Name,Model Run Time,Cell Count Source`. The CSV has no cell count column (the dictionary refers to "Cell Count" but the file carries only the category).

### README (`RIFv2_README.txt`, read in full)

Cell counts come from FWRI (`myfwc.com`), HABscope (`habscope.gcoos.org`), Pinellas County (`pinellascounty.org/environmental/red-tide.htm`) and SCCF (`sccf.org`). Wind is NDFD. "Presence of Karenia Brevis in ocean water with on-shore winds creates the conditions for respiratory irritation on the beach. This model uses the beach shoreline angle, reported cell count and forecasted wind speed and direction to predict irritation levels at the location. Possible respiratory forecasts include 'Very low', 'Low', 'Moderate', and 'High.'" It states the model "is hosted and run at NOAA NOS NCCOS every 3 hours".

Parser rules: tolerate unknown category and source strings; treat the dictionary as a hint, not a closed set; match the beach by name and coordinates; take the validity from `Wind Time`.

### Other Gulf Coast links (page `https://coastalscience.noaa.gov/science-areas/habs/hab-forecasts/gulf-coast`)

- Florida Respiratory Forecast viewer and Texas viewer (ArcGIS Experience, `experience.arcgis.com/experience/1ef2c961393948f3861d5ec9831250ee`).
- Archive of past forecasts at NCEI: `https://www.ncei.noaa.gov/access/metadata/landing-page/bin/iso?id=gov.noaa.nodc:NOS-HABOFS-FL-TX`; HABSOS interactive map: `https://www.ncei.noaa.gov/products/harmful-algal-blooms-observing-system`.
- Guide: `https://cdn.coastalscience.noaa.gov/page-attachments/habs/GulfofMexico_HAB_Forecast_Manual_Final.pdf`.
- Observations by the states: `https://myfwc.com/research/redtide/statewide/` (FWC) and `https://tpwd.texas.gov/landwater/water/environconcerns/hab/redtide/status.phtml` (TPWD); Mote beach conditions `https://visitbeaches.org/`.

## Current bulletin URLs by region

| Region | Product | URL | State on 2026-11-17 |
|---|---|---|---|
| Lake Erie | current bulletin (PDF) | `https://cdn.coastalscience.noaa.gov/hab-data/forecasts/LErie_forecast/web_latest/current_bulletin.pdf` | HTTP 200, `application/pdf`, 3.4 MB, `Last-Modified: Tue, 17 Nov 2026 09:39:14 GMT` although the season text says "11/19/2025 is the final day of the 2025 Lake Erie HAB forecast season" and the bloom season is "typically July to October". The page's own label reads "Model Last Updated: 2026-11-17 04 AM EST". Treat an off-season bulletin as not a forecast. |
| Lake Erie | severity index 2000 to 2025 (XLSX) | `https://nccospublicstor.blob.core.windows.net/hab-data/bulletins/lake-erie/2025/NOAA_NCCOS_2000to2025_Curated_LE_Annual_Severity.xlsx` | listed on the page; not fetched |
| Lake Erie | page | `https://coastalscience.noaa.gov/science-areas/habs/hab-forecasts/lake-erie/` | CSV, shapefile, KML and GeoJSON: none listed. Archive: `https://www.ncei.noaa.gov/access/metadata/landing-page/bin/iso?id=gov.noaa.nodc:NOS-HABOFS-LakeErie`. Early-season projections weekly from May. |
| Gulf of Maine | weekly dashboard (PDF), only when a bloom is present | `https://nccospublicstor.blob.core.windows.net/hab-data/forecasts/gulfofmaine/dashboard_GMaine.pdf` | HTTP 200, `application/pdf`, 178.5 KB, `Last-Modified: Tue, 05 May 2026 17:02:55 GMT` (stale) |
| Gulf of Maine | nowcast and forecast simulation (viewer) | `https://coastalscience.noaa.gov/science-areas/habs/hab-forecasts/gulf-of-maine/nowcast-forecast-simulation/` | HTML viewer; not fetched |
| Pacific Northwest | NANOOS HAB bulletin (partner) | `https://www.nanoos.org/products/habs/forecasts/home.php` | HTTP 200, HTML |
| California | SCCOOS HAB bulletin (partner) | `https://sccoos.org/california-hab-bulletin/` | listed on NOAA's page; not fetched |
| Gulf Coast | respiratory forecast (CSV zip) | see above | machine-readable |

NOAA lists no JSON, GeoJSON, KML or shapefile for any region. Short-term forecasts are "once or twice weekly" according to the NOAA page except the Gulf RIF (every 3 hours). A Lake Erie or Gulf of Maine bulletin can be linked from a spot card; parsing a PDF is not planned.

## Government closure banner

Every `coastalscience.noaa.gov` page read on 2026-11-17 carried the banner "The U.S. Government is closed. This site will not be updated.", followed by a note that NOAA websites needed to protect lives and property are maintained. The Gulf CSV zip, the Lake Erie PDF and the pages themselves were modified on 2026-11-16 and 2026-11-17 all the same, so the banner does not stop the data feeds. Do not treat the banner text as a data outage; judge freshness by `Last-Modified` and the run time in the file.

## Findings

- Not usable for the New York test places or any region outside the Gulf Coast forecast area (Florida and Texas); the beaches in the 2026-11-17 file are on the Florida coast (26.8 to 30.7 N, 80.1 to 86.2 W).
- The number of beaches per file varies from run to run (about 55 on 2026-11-08, 22 on 2026-11-17); a client must not assume a fixed set.
- The Gulf feed has no history; a short-term use is "respiratory irritation today at the nearest beach", shown only within a distance threshold of a listed beach.

## Licence, terms and attribution

Re-checked 2026-11-18: unchanged. `https://coastalscience.noaa.gov/science-areas/habs/hab-forecasts/gulf-coast/` states no terms. The NOAA disclaimer page (`https://www.noaa.gov/disclaimer`) answered HTTP 403 to `curl` (**unverified**); the public-domain statement above rests on the NOAA and NWS pages read for the other notes.
