# Raw fixtures: PAD-US public access layer

Recorded on 2026-11-17. No key or account is involved. See docs/providers/padus.md.
Base: `https://services.arcgis.com/v01gqwM5QqNysAAi/arcgis/rest/services/PADUS_Public_Access/FeatureServer/0/query`

- `padus-croton-attrs-2026-11-17.json`: `?geometry=-73.90,41.17,-73.86,41.23&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outSR=4326&outFields=Unit_Nm,Pub_Access,GAP_Sts,Category,DesTp_Desc,MngNm_Desc,GIS_Acres&orderByFields=OBJECTID&returnGeometry=false&resultRecordCount=40&f=json` (35 features), 2026-11-17.
- `padus-croton-geojson-offset0.0005-5rows-2026-11-17.json`: the same envelope and fields with `returnGeometry=true&resultRecordCount=5&f=geojson&maxAllowableOffset=0.0005&geometryPrecision=5`, 2026-11-17.
