# State boundaries (bundled)

Used by `underbite-core::discover::states` to tell which state a map point is in, so the point card can link that state's fishing regulations and licence without the user saving a region. Bundled; the app and the tests never fetch it.

- **Source:** U.S. Census Bureau, cartographic boundary file `cb_2025_us_state_5m` (states, DC and the territories; scale 1:5,000,000; clipped to the shoreline).
- **URL:** `https://www2.census.gov/geo/tiger/GENZ2025/shp/cb_2025_us_state_5m.zip` (shapefile, 1.1 MB zipped).
- **Fetched:** 2026-11-20.
- **Licence:** the file's own metadata (`cb_2025_us_state_5m.shp.iso.xml`) says: "These products are free to use in a product or publication, however acknowledgement must be given to the U.S. Census Bureau as the source." It adds that the files are for visual display at small scales and not for geographic analysis or geocoding. The app uses them to choose which official regulation links to show, with a border tolerance, and states the source here and in the app's notices.
- **Conversion:** `tools/state_boundaries.py` (Python standard library only) reads the shapefile, rounds coordinates to 1/1000 degree, simplifies each ring with Douglas-Peucker at 1.5 units (about 165 m), drops rings under 5 units in both directions, and writes delta-encoded varints. The format is described in the script. Run it again when the file is renewed.
- **Result:** `crates/underbite-core/data/states/us-states.bin`, 56 jurisdictions (50 states, DC, PR, AS, GU, MP, VI), 43,821 points, 92,839 bytes (budget 300,000).
- **Behaviour:** the file follows the shore, so open water belongs to no state. Rivers and lakes that are borders run along the boundary, so the lookup returns every state within 3 km of the point, and a point at sea within 25 km of a coast gets that coast's state. Longitudes of the western Aleutians (east of 172 degrees E) are not handled.
- **Terms page:** `https://www.census.gov/geographies/mapping-files/time-series/geo/cartographic-boundary.html` (the files' page; the licence wording above is the file's own metadata). The app credits it as "State lines: U.S. Census Bureau, cartographic boundary files" in the sources list and on the Water here card whenever the lookup supplied a region.
