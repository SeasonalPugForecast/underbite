# Raw fixtures: state lake contours and artificial reefs

All recorded on 2026-11-17 with `User-Agent: underbite-scout/0.1 (provider docs check)`. No key or account is involved. See docs/providers/state-maps.md.

- `in-lake-bathymetry-crooked-creek-lake-2026-11-17.geojson`: `https://gisdata.in.gov/server/rest/services/Hosted/Lake_Bathymetry/FeatureServer/0/query?where=lake_name%3D'Crooked%20Creek%20Lake'&outFields=*&outSR=4326&geometryPrecision=6&maxAllowableOffset=0.00002&f=geojson` (Indiana DNR; 18 contour lines, depth in feet below the lake surface; generalised by the two geometry parameters, because the unreduced response was 760 KB)
- `mn-lake-bathymetry-pine-01000100-2026-11-17.geojson`: `https://enterprise.gisdata.mn.gov/aghost/rest/services/us_mn_state_dnr/water_lake_bathymetry/MapServer/0/query?where=dowlknum%3D'01000100'&outFields=*&outSR=4326&f=geojson` (Minnesota DNR; Pine Lake, Aitkin County, 8 contour lines; `depth` is negative)
- `ny-artificial-reefs-socrata-2026-11-17.json`: `https://data.ny.gov/resource/jsnj-ehv9.json?$limit=50` (NYSDEC; all 16 reef sites, Socrata SODA JSON, unmodified)
