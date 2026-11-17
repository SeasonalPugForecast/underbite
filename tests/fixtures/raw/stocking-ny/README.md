# Raw fixtures: NY stocking datasets and the 3DHP name match

Recorded on 2026-11-17. No key or account is involved. See docs/providers/stocking-ny.md.

- `socrata-ny-e52k-ymww-county-waterbody-pairs-2026-11-17.json`: `https://data.ny.gov/resource/e52k-ymww.json?$select=county,waterbody,count(*) as n&$group=county,waterbody&$limit=50000`, 2026-11-17.
- `socrata-ny-d9y2-n436-county-waterbody-pairs-2026-11-17.json`: `https://data.ny.gov/resource/d9y2-n436.json?$select=county,waterbody,count(*) as n&$group=county,waterbody&$limit=50000`, 2026-11-17.
- `3dhp-waterbody-layer60-name-query-kensico-2026-11-17.json`: `https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/MapServer/60/query?where=UPPER(gnisidlabel) LIKE '%KENSICO%'&geometry=-73.78,41.05,-73.7,41.14&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=id3dhp,gnisid,gnisidlabel,featuretypelabel,areasqkm&returnGeometry=false&f=json`, 2026-11-17.
- `stocking-3dhp-name-match-sample50-2026-11-17.synthetic.json`: hand-reduced from the results of 100 queries (layers 60 and 50 of `3DHP_all/MapServer`, county polygon from TIGERweb) for 50 random pairs; not a source response. Method in the note.
