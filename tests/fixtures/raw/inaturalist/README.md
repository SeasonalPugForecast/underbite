# Raw fixtures: iNaturalist API

Observation records are licensed by their observers (many under non-commercial terms), so the
observation files here are hand-made (`*.synthetic.json`): the JSON shape and field names of the
service, with invented ids, uuids, positions rounded to 0.1 degree and dates, example URLs that do not
resolve, and the licence `cc0`. They are not recordings. See docs/providers/inaturalist.md.
The field selection used for the v2 requests asks for no user fields, so no observer names or logins appear.

- `inaturalist-v2-observations-fields-dotted.synthetic.json`: shape of `https://api.inaturalist.org/v2/observations?taxon_id=51392&quality_grade=research&nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&per_page=2&fields=id,observed_on,taxon.name`.
- `inaturalist-v2-observations-fields-striped-bass.synthetic.json`: shape of `https://api.inaturalist.org/v2/observations?taxon_id=51392&quality_grade=research&license=cc-by,cc-by-nc,cc0&nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&per_page=3&order_by=observed_on&fields=(id:!t,uuid:!t,uri:!t,observed_on:!t,time_observed_at:!t,observed_time_zone:!t,geojson:(coordinates:!t),obscured:!t,positional_accuracy:!t,public_positional_accuracy:!t,license_code:!t,quality_grade:!t,taxon:(id:!t,name:!t,rank:!t,preferred_common_name:!t),photos:(id:!t,url:!t,license_code:!t))` (the `fields` value URL-encoded). Three observations, one of them obscured.
- `inaturalist-histogram-month-of-year-striped-bass-2026-11-17.json`: recorded 2026-11-17, `https://api.inaturalist.org/v1/observations/histogram?taxon_id=51392&quality_grade=research&nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&interval=month_of_year&date_field=observed` (monthly counts only, no records).
