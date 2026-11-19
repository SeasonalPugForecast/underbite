# State lake contours and artificial reefs

Verified on 2026-11-17 (UTC) with `curl` against each service's REST metadata and queries, and against the agencies' own pages. Fixtures: `tests/fixtures/raw/state-maps/` (index in its `README.md`). Requests used `User-Agent: underbite-scout/0.1 (provider docs check)`, a few seconds apart; no key, account or token is needed by any source below.

Every state contour depth is **relative to the lake surface** (or to a stated gauge level), not to a datum. Record and show exactly what each source states; no datum is inferred. Contours are survey products of varying dates: show the survey date and "not for navigation".

## Choice of states

No statewide machine-readable lake contour layer for **New York** was found (see below), so the two other states were chosen on what was found live:

- **Indiana** (DNR Division of Fish and Wildlife): an ArcGIS FeatureServer of contour polylines for surveyed lakes, queryable by envelope, with GeoJSON output, survey date and legal gauge adjustment per lake.
- **Minnesota** (DNR): the largest statewide set (46,306 contour lines; lake outline and survey-metadata layers) on the state's ArcGIS Server, queryable by envelope, GeoJSON output.

Neither has ocean coast; Indiana (Lake Michigan) and Minnesota (Lake Superior) publish no artificial-reef site list that was found. Reefs are therefore covered for New York only.

## Verdicts

| Date | Source | Service type | Verdict |
|---|---|---|---|
| 2026-11-17 | Indiana DNR lake bathymetry | vector features (ArcGIS FeatureServer 11.5, polylines) | verified; usable, query by envelope, GeoJSON; licence unstated |
| 2026-11-17 | Minnesota DNR lake bathymetry | vector features (ArcGIS MapServer 11.5, layers with `query`, polylines) | verified; usable, query by envelope, GeoJSON; unit stated only in secondary metadata; DNR licence terms |
| 2026-11-17 | New York lake contours | none statewide; USGS data releases for New York City reservoirs (file downloads) | partly verified; not an online query service; skip for a service provider |
| 2026-11-17 | NYSDEC artificial reefs | Socrata open-data table (JSON, CSV, XML) | verified; 16 sites with corner coordinates |

**Both usable contour sources are vector feature services**, so a vector provider (a `statemaps` module that builds ArcGIS `query` requests and parses GeoJSON) is needed. Neither offers an image export that substitutes for the features (the Minnesota MapServer can export images, the Indiana service has no map capability).

## Indiana: Lake Bathymetry (IDNR)

- Base: `https://gisdata.in.gov/server/rest/services/Hosted/Lake_Bathymetry/FeatureServer` (ArcGIS Server 11.5, host `gisdata.in.gov`, the IndianaMap service host). Capabilities `Query,Sync` (no map image). Layers: `0` "Lake Bathymetry" (polyline), `1` "Lake Outlines" (polygon, NHD-style fields `gnis_name`, `areasqkm`, `elevation`; not used for depth). A second service `Lake_Bathymetry_RO` exists (read-only, credit USGS/IGIO/AECOM); it was not examined.
- Storage CRS: EPSG:26916 (NAD83 / UTM 16N). `outSR=4326` returns degrees. Query formats: `JSON`, `geoJSON`, `PBF`.
- Limits: `maxRecordCount` 2000; the layer holds 10,260 lines. `resultOffset` with `resultRecordCount` paged correctly (tested at offset 5000; `exceededTransferLimit: true` is returned when more rows exist) even though the layer JSON lists no `supportsPagination`; add `orderByFields=objectid` for a stable order.
- Envelope query (verified): `geometry=-86.31,39.11,-86.28,39.13&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects` returned 18 features (count only, with `returnCountOnly=true`).
- Request for one lake as GeoJSON: `{base}/0/query?where=lake_name%3D'Crooked Creek Lake'&outFields=*&outSR=4326&geometryPrecision=6&maxAllowableOffset=0.00002&f=geojson` (the unreduced geometry of this 10-acre lake was 760 KB, so request generalisation by zoom).
- Fields (layer 0): `contour` (double, depth), `line_type` (`INDEX` or `INTERMEDIATE`), `lake_name`, `lake_acres`, `lake_max_depth` (double, often null), `survey_date` (date, epoch milliseconds UTC), `gauge_adjusted_level` and `lake_elevation` (doubles, usually null), `huc_12`, `county1_name`/`county2_name`, `county1_num`/`county2_num`, `nhdid_waterbody`, `finfo_lakeid`, `objectid`, `globalid`.
- Unit and reference, from the layer description: "Lake_Bathymetry_DNR_IN is a polyine feature dataset of bathymetric contours for certain Indiana lakes, depicting the depths of the water with the shoreline of each lake as elevation 0. Contour intervals vary depending upon the lake. [...] The contour values of some lakes were adjusted to coincide with the lake's legal gauge level. Contour values are feet below the surface. Updated 4/16/19". Surveys are per lake, e.g. Crooked Creek Lake 2016-06-16 (`survey_date` 1466035200000), Kokomo Reservoir 2018-08-27.
- Coverage: "certain Indiana lakes" (not all); extent roughly -87.53,37.99 to -84.80,41.77.
- Service date: the layer's ETag `1776625121074` corresponds to 2026-04-19 UTC; the description text says 4/16/19.
- Licence and credit: the service carries no `licenseInfo` and no terms were found; `copyrightText` is "Indiana Department of Natural Resources, Division of Fish and Wildlife". The agency's printed lake depth maps are described as copyrighted with permission to reprint and use with credit to Indiana DNR (found by search, not re-read). Credit "Indiana DNR, Division of Fish and Wildlife"; **terms for the GIS layer are unverified**.
- Fixture: `in-lake-bathymetry-mill-pond-lake.synthetic.geojson` (invented lake, same properties and geometry types; the agency's data is not redistributed).

## Minnesota: Lake Bathymetry (MN DNR)

- Base: `https://enterprise.gisdata.mn.gov/aghost/rest/services/us_mn_state_dnr/water_lake_bathymetry/MapServer` (ArcGIS Server 11.5; the Minnesota Geospatial Commons service host; capabilities `Map,Data,Query`). Layers: `0` "Lake Bathymetric Contours" (polyline), `1` "Lake Bathymetric Outline" (polygon), `2` "Lake Bathymetric Metadata" (polygon, one per mapped lake), `3` "Lake Bathymetric Elevation Model" (raster; not queried). A copy exists on the Metropolitan Council server (`arcgis.metc.state.mn.us/.../GDRS/DNR_water_lake_bathymetric_contours`); on 2026-11-17 it answered "Service ... not started" and is not the source to use.
- Storage CRS: EPSG:26915 (NAD83 / UTM 15N). `outSR=4326` returns degrees. Query formats `JSON`, `geoJSON`, `PBF`.
- Limits: `maxRecordCount` 2000; layer 0 holds 46,306 lines. `resultOffset`/`resultRecordCount` paged (tested at 5000). Envelope query verified: `geometry=-93.09,46.18,-93.05,46.21&geometryType=esriGeometryEnvelope&inSR=4326` returned 15 features.
- Request for one lake: `{base}/0/query?where=dowlknum%3D'01000100'&outFields=*&outSR=4326&f=geojson` (8 lines, 66 KB for Pine Lake, Aitkin County).
- Fields, layer 0: `depth` (smallint, **negative** below the surface, `0` is the shoreline), `abs_depth` (smallint, the same as a positive number), `dowlknum` (string, the DNR lake number, eight digits), `lake_name`, `objectid`. Layer 2: `lake_name`, `dowlknum`, `mapid`, `filename`, `fldw_end` (string, end of fieldwork, e.g. `7/19/1990`), `data_src` (e.g. `ORIG MAP`), `area1`, `perimeter`, `lkbath_id`. Layer 1: outline polygons with `lake_name`, `dowlknum`.
- Unit: **the service does not state it.** DNR metadata for the lake DEM, found by search, reads "DEPTH ... represents lake depth in (negative) feet", and the companion shaded-relief image is "classified at 5-foot depth intervals"; the Pine Lake values 0, -5 ... -25 fit. Treat feet as documented elsewhere and mark it as such; the contour dataset's own metadata record could not be opened (the Geospatial Commons pages render with JavaScript and the old metadata URL answers 404).
- Reference: the depth is below the lake surface at the time of the original map; per the Commons record, contours "are derived from existing lake contour maps of variable dates" (digitised from DNR Lake Mapping Unit maps; outlines from 1991-92 aerial photography). Field work dates per lake are in layer 2 (`fldw_end`). Coverage: "statewide, however GIS products have only been completed for selected lakes" (the shaded-relief metadata, last updated 2016-07-26).
- Licence: DNR "General Geographic Data License Agreement" (`https://dnr.state.mn.us/wmas/kml/license.html`, the general agreement that applies to DNR data unless a dataset has its own): non-exclusive, non-sublicensable licence; data "as is"; "MNDNR must be acknowledged as having contributed data to the development of the product"; use "is not restricted" but data "may not be sold commercially or privately, without the written permission of MNDNR". The shaded-relief record says "Internal DNR use is unrestricted. External use is constrained by the standard, general DNR GIS Data License Agreement"; the contour dataset's own record could not be read, so a dataset-specific licence is **unverified**. The service has no `copyrightText` or `licenseInfo`. Credit: "Minnesota Department of Natural Resources" (Division of Fish and Wildlife, Section of Fisheries, as originator). A free app that does not sell the data is within the stated terms; bundling the data into a paid or sold product is not.
- Fixture: `mn-lake-bathymetry-alder-99000100.synthetic.geojson` (invented lake, same properties and geometry types).

## New York: lake contours

- No statewide NYSDEC or NYS GIS lake contour layer was found. The search was not exhaustive; treat "none found" as the state of the search on 2026-11-17.
- Federal alternative for New York City reservoirs (USGS data releases on ScienceBase, no account): New Croton Reservoir ("Geospatial bathymetry datasets for New Croton Reservoir, New York, 2017", item `5fa58d7ed34ed3698f92dd25`, surveyed 2017-06-28 to 2017-10-26, published 2021-06-16; shapefile of 2-foot contours, grids and tables; file names show feet NAVD 88, i.e. **elevation, not depth**) and the West of Hudson reservoirs (for example Neversink, item `58069258e4b0824b2d1d422e`, surveyed 2013 to 2015, 2-foot contours, native EPSG:26918). These are downloadable files, not query services; no fixture was recorded. The item JSON carries no rights text. Reservoir elevations must be converted to depth with a water-surface elevation (a separate time series in the release), so they do not fit a "depth below the lake surface" display without extra work. Verdict: not an online source; skip for the first release.

## New York: artificial reefs (NYSDEC)

- Dataset: "Artificial Reefs Managed by New York State Department of Environmental Conservation", open-data id `jsnj-ehv9` on `data.ny.gov` (Socrata). Page `https://data.ny.gov/d/jsnj-ehv9`; DEC program page `https://www.dec.ny.gov/outdoor/7896.html`.
- Requests: JSON `https://data.ny.gov/resource/jsnj-ehv9.json?$limit=50` (all rows; the table has 16: `$select=count(*)` returns `16`); column metadata `https://data.ny.gov/api/views/jsnj-ehv9.json`; downloads `https://data.ny.gov/api/v3/views/jsnj-ehv9/export.csv?accessType=DOWNLOAD` (also `query.json`, `query.xml`; listed in the catalogue, not requested). No app token was used; no rate limit is published for unauthenticated clients.
- Fields (strings unless noted): `reef_name`, `northwest_latitude`, `northwest_longitude`, `northeast_latitude`, `northeast_longitude`, `southeast_latitude`, `southeast_longitude`, `southwest_latitude`, `southwest_longitude` (decimal degrees as strings; the dataset text says "GPS location coordinates" without naming the datum), `body_of_water`, `description_of_location` (e.g. "3.0 Nautical Miles South of Atlantic Beach"), `size` ("413 acres"), `boundaries` ("2,000 by 1,000 yards"), `minimum_depth` and `maximum_depth` ("55 feet"; text with unit, no datum stated), `comments` (optional), plus platform-generated `location1`, `georeference` (point), `multipoint_set`, `polygon_shape` (GeoJSON polygon of the four corners) and `:@computed_region_*` columns.
- Sites (all 16): Atlantic Beach, Matinecock, Rockaway, Twelve Mile, Fire Island, Mattituck, Port Jefferson/Mount Sinai, Smithtown, Hempstead, Sixteen Fathom, McAllister Grounds, Huntington/Oyster Bay, Shinnecock, Moriches, Kismet and Yellowbar reefs; bodies of water Atlantic Ocean, Long Island Sound, Great South Bay.
- Dates: rows last updated 2025-01-22 (`Last-Modified` header; `rowsUpdatedAt` 1737568038), view modified 2026-04-03. The dataset states it "is primarily static and is revised only if site boundaries are changed or new sites added".
- Licence and credit: the view has no licence field; `attribution` is "New York State Department of Environmental Conservation" with link `http://www.dec.ny.gov/outdoor/7896.html`. Secondary sources say the State's open-data terms do not require attribution; the OPEN-NY terms page could not be read, so the terms are **unverified**. Credit NYSDEC as the source. The coordinates describe the general area where material was placed, not a point; link to the DEC pages for rules rather than restating them.
- Fixture: `ny-artificial-reefs-socrata-2026-11-17.json` (all 16 rows, unmodified).
- Other: the MARCO Mid-Atlantic portal compiles reef polygons for five states (reported by search); not examined.

## Not verified

- Terms for the Indiana GIS layer and for the Minnesota contour dataset's own record; the OPEN-NY terms of use.
- The depth unit of the Minnesota contour layer from the layer's own metadata (secondary documents say feet).
- Reef lists for Indiana and Minnesota (none found), and for other coastal states.
- Rate limits: none published by any of the three hosts; none returned rate-limit headers.
- Maintenance windows or deprecations: none announced in anything read; the Metropolitan Council copy of the Minnesota service was down on the check date.

## Licence, terms and attribution

Re-checked 2026-11-18: Minnesota unchanged: the DNR General Geographic Data License Agreement (`https://www.dnr.state.mn.us/wmas/kml/license.html`) still grants a "non-exclusive, non-sublicensable, license", requires that MNDNR "must be acknowledged as having contributed data to the development of the product", provides the data "as is", and says that although use is not restricted, the data "may not be sold commercially or privately" without written permission; the dataset-specific record stays **unverified**. Indiana unchanged: the service root has `licenseInfo` and `accessInformation` "None", and layer 0 has `copyrightText` "Indiana Department of Natural Resources, Division of Fish and Wildlife"; terms for the GIS layer stay **unverified**. New York artificial reefs: the OPEN-NY Terms of Use were read in full on this date (see `stocking-ny.md`): no attribution is required, the State grants a non-exclusive, revocable licence, and use must be lawful. The earlier statement that the terms page could not be read is answered; crediting NYSDEC remains optional.
