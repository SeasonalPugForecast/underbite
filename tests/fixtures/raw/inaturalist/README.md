# Raw fixtures: iNaturalist API

Recorded on 2026-11-17. No key or account is involved. See docs/providers/inaturalist.md.
The field selection used for the v2 requests asks for no user fields, so no observer names or logins are present.

- `inaturalist-v2-observations-fields-dotted-2026-11-17.json`: `https://api.inaturalist.org/v2/observations?taxon_id=51392&quality_grade=research&nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&per_page=2&fields=id,observed_on,taxon.name`, 2026-11-17.
- `inaturalist-v2-observations-fields-striped-bass-2026-11-17.json`: `https://api.inaturalist.org/v2/observations?taxon_id=51392&quality_grade=research&license=cc-by,cc-by-nc,cc0&nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&per_page=3&order_by=observed_on&fields=(id:!t,uuid:!t,uri:!t,observed_on:!t,time_observed_at:!t,observed_time_zone:!t,geojson:(coordinates:!t),obscured:!t,positional_accuracy:!t,public_positional_accuracy:!t,license_code:!t,quality_grade:!t,taxon:(id:!t,name:!t,rank:!t,preferred_common_name:!t),photos:(id:!t,url:!t,license_code:!t))` (the `fields` value URL-encoded), 2026-11-17.
- `inaturalist-histogram-month-of-year-striped-bass-2026-11-17.json`: `https://api.inaturalist.org/v1/observations/histogram?taxon_id=51392&quality_grade=research&nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&interval=month_of_year&date_field=observed`, 2026-11-17.
