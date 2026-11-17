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
