# Raw fixtures: GBIF occurrence API

No key or account is involved. See docs/providers/gbif.md.
Base: `https://api.gbif.org/v1/occurrence/search`

Occurrence records carry licences set by their publishers (some non-commercial), so the record file
is hand-made (`*.synthetic.json`): the JSON shape and field names of the service, with invented keys,
uuids, names, dates and localities, positions rounded to 0.1 degree, example URLs that do not
resolve, and the licence CC0. It is not a recording.

- `gbif-occurrence-striped-bass-nyharbor-2-records.synthetic.json`: shape of `?scientificName=Morone%20saxatilis&decimalLatitude=40.4,41.0&decimalLongitude=-74.4,-73.6&hasCoordinate=true&limit=2&offset=0` (two records, `facets` empty).
- `gbif-facet-month-striped-bass-nyharbor-2026-11-17.json`: recorded 2026-11-17, `?scientificName=Morone%20saxatilis&decimalLatitude=40.4,41.0&decimalLongitude=-74.4,-73.6&hasCoordinate=true&limit=0&facet=month&facetLimit=12` (monthly counts only, no records).
