# Raw fixtures: state lake contours and artificial reefs

The two lake files are hand-made (`*.synthetic.geojson`) and are not recordings: the agencies' lake
bathymetry is not ours to redistribute, so each file describes an invented lake with the same
properties and geometry types as the service (rounded positions, made-up contour rings). See
docs/providers/state-maps.md. The reef table was recorded on 2026-11-17 with `User-Agent:
underbite-scout/0.1 (provider docs check)`; no key or account is involved.

- `in-lake-bathymetry-mill-pond-lake.synthetic.geojson`: the shape of the Indiana DNR `Lake_Bathymetry` FeatureServer query `where=lake_name='...'&outFields=*&outSR=4326&f=geojson`: 18 contour features (`LineString` and `MultiLineString`), `contour` in feet below the lake surface, `line_type` `INDEX` or `INTERMEDIATE`, `survey_date` in milliseconds.
- `mn-lake-bathymetry-alder-99000100.synthetic.geojson`: the shape of the Minnesota DNR `water_lake_bathymetry` MapServer query `where=dowlknum='...'&outFields=*&outSR=4326&f=geojson`: 8 `LineString` contours; `depth` is negative feet.
- `ny-artificial-reefs-socrata-2026-11-17.json`: `https://data.ny.gov/resource/jsnj-ehv9.json?$limit=50` (NYSDEC; all 16 reef sites, Socrata SODA JSON, unmodified), 2026-11-17.
