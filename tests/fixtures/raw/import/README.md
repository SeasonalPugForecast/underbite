# Import fixtures

Hand-made inputs for the GPX, CSV, KML and KMZ readers. Every file here is synthetic
(`*.synthetic.*`) and uses only the test places of the project.

All dates and places are invented except the coordinates of the test places (The Battery,
40.7033,-74.0170; the Croton River near 41.21,-73.86). Nothing here was downloaded.

| File | What it holds |
|---|---|
| `battery.synthetic.gpx` | GPX 1.1 with two waypoints at The Battery: entities, CDATA, `extensions` to ignore, one `time` without an offset |
| `croton.synthetic.gpx` | GPX 1.1 with a two-segment track along the Croton River and a two-point route |
| `battery.synthetic.csv` | CSV with a byte order mark, CRLF line ends, a quoted comma and a quoted line break; header `Name,Latitude,Longitude,Notes` |
| `croton.synthetic.csv` | semicolon separated CSV with the columns in another order and upper-case names |
| `decimal-comma.synthetic.csv` | a decimal comma in a quoted latitude, which must be an error |
| `battery.synthetic.kml` | KML 2.2 with the same two waypoints as `battery.synthetic.gpx` (CDATA, an entity, a `TimeStamp` without an offset, spaces after commas, `ExtendedData` to ignore); reads to the same `Imported` |
| `croton.synthetic.kml` | a `gx:MultiTrack` of two `gx:Track`s and a `LineString`; reads to the same `Imported` as `croton.synthetic.gpx` |
| `folders.synthetic.kml` | nested `Folder`s, a `MultiGeometry` with two points and a line, a polygon to skip, a `TimeSpan` with a bare date |
| `battery.synthetic.kmz` | `battery.synthetic.kml` as `doc.kml` beside another file |
| `croton.synthetic.kmz` | no `doc.kml`: the first `.kml` at the root (upper-case extension) after a `.kml` in a subfolder |
| `bomb.synthetic.kmz` | 25 MB of spaces in `doc.kml`, compressed to a few kilobytes; refused |
| `escape.synthetic.kmz` | an entry named `../evil.kml`; refused |

The `.kmz` files were made with Python's `zipfile` (deflate, fixed timestamps) from the `.kml` files here.
