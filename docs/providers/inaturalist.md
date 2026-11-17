# iNaturalist API

Verified on 2026-11-17 (UTC) with `curl` against `api.inaturalist.org` (versions 1 and 2). An earlier survey is in `docs/providers/tier2.md`. The iNaturalist documentation pages (`/pages/api+recommended+practices`, `/v1/docs/`) answer a scripted client with 403 or an empty shell, so the limits below are those printed in the API's own description (`/v1/swagger.json`, recorded 2026-11-08) plus measurements. Fixtures: `tests/fixtures/raw/inaturalist/` (index in its `README.md`).

## Endpoints

| Item | Value |
|---|---|
| Base v1 | `https://api.inaturalist.org/v1` |
| Base v2 | `https://api.inaturalist.org/v2` |
| Authentication | none for reads |
| Headers sent | `User-Agent: underbite-scout/0.1` |
| Response | JSON; `cache-control: public, max-age=300`, `access-control-allow-origin: *`, `etag` |
| Rate limit | "a max of 100 requests per minute, though we ask that you try to keep it to 60 requests per minute or lower, and to keep under 10,000 requests per day" (swagger description); no rate-limit headers are sent (none in the observed responses) |
| Terms | the API is for application development and not for data scraping; subject to the iNaturalist Terms of Service |

Requests in this check: about 12 over 5 minutes, no throttling.

## The `fields` parameter: v2 only

`fields` is ignored by v1: `GET /v1/observations?...&per_page=2&fields=id,observed_on,geojson` returned 140,988 bytes for two observations, the same size as the same request with the object syntax `fields=(id:!t,...)`. A v1 page of 200 observations measured 15,223,681 bytes (about 75 KB each); do not use v1 for lists.

v2 honours `fields` and returns only what is asked:

```text
GET /v2/observations?taxon_id=51392&quality_grade=research
    &nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&per_page=2&fields=id,observed_on,taxon.name
```

Fixture `inaturalist-v2-observations-fields-dotted.synthetic.json` (same shape, invented records). The dotted list `a,b,taxon.name` and the nested object (Rison-like) syntax both work. The object syntax is `(key:!t,nested:(key:!t))`, where `!t` means true:

```text
fields=(id:!t,uuid:!t,uri:!t,observed_on:!t,time_observed_at:!t,observed_time_zone:!t,
        geojson:(coordinates:!t),obscured:!t,positional_accuracy:!t,public_positional_accuracy:!t,
        license_code:!t,quality_grade:!t,
        taxon:(id:!t,name:!t,rank:!t,preferred_common_name:!t),
        photos:(id:!t,url:!t,license_code:!t))
```

URL-encode it (`curl --data-urlencode`). Three observations with this field set and `license=cc-by,cc-by-nc,cc0` were 2,197 bytes (fixture `inaturalist-v2-observations-fields-striped-bass.synthetic.json` has the same shape), about 700 bytes each, against about 70 KB in v1. Because the fields chosen name no user, the fixture holds no observer names or logins. Asking for `user` or `identifications` would return them; do not.

## Month-of-year histogram

```text
GET /v1/observations/histogram?taxon_id=51392&quality_grade=research
    &nelat=41.0&nelng=-73.6&swlat=40.4&swlng=-74.4&interval=month_of_year&date_field=observed
```

The same path works under `/v2`. Response (157 bytes, fixture `inaturalist-histogram-month-of-year-striped-bass-2026-11-17.json`):

```json
{"total_results":12,"page":1,"per_page":12,"results":{"month_of_year":{"1":4,"2":3,"3":8,"4":54,"5":49,"6":52,"7":42,"8":46,"9":52,"10":47,"11":44,"12":17}}}
```

Keys are month numbers as strings, 1 to 12, always all twelve in the sample. `date_field=observed` uses the observation date (the alternative is `created`). Other values for `interval` (`year`, `month`, `week`, `day`, `hour`, `week_of_year`) were not tried (**unverified**).

## Paging

- `per_page` of 201 returned 200 results (`per_page` 200 in the response): the maximum is 200.
- `page` x `per_page` must stay at or below 10,000: `page=100000&per_page=1` returned HTTP 500 (JSON `errors[0].message`: "Result window is too large, page x size must be less than or equal to [10000]. Please narrow your search, or use a sliding window approach with id_above or id_below params"). Note the status is 500, not 4xx.
- `total_results` is the full match count. For the striped-bass box it was 418 for all research-grade records on 2026-11-17 and 272 with `license=cc-by,cc-by-nc,cc0`.

## Fields

| Field | Meaning |
|---|---|
| `id`, `uuid`, `uri` | identifiers and observation page |
| `observed_on` (date), `time_observed_at` (ISO 8601 with offset, local), `observed_time_zone` | when |
| `geojson.coordinates` | `[lon, lat]`; possibly obscured |
| `positional_accuracy`, `public_positional_accuracy` (m), `obscured` | location quality; obscured points are spread over a cell (about 27 km seen on 2026-11-08) |
| `quality_grade` | `research`, `needs_id`, `casual` |
| `license_code` | `cc0`, `cc-by`, `cc-by-nc` and others, or null (all rights reserved) |
| `taxon.id`, `name`, `rank`, `preferred_common_name` | identification; Morone saxatilis is taxon 51392 |
| `photos[].url`, `license_code` | `.../square.jpg`; sizes by replacing the file name: `square` 75 px, `small` 240, `medium` 500, `large` 1024, `original` 2048 |

## License and attribution

Records and photos are licensed individually by the observer. Filter with `license=` and `photo_license=` (comma-separated codes) to retrieve only reusable ones; photos hosted on `inaturalist-open-data.s3.amazonaws.com` carry open licences, those on `static.inaturalist.org` do not. When showing a record, credit the observer as the licence requires, link the observation (`uri`), and credit iNaturalist. Observer names then come from `user.login`, a field the app does not request by default; decide when a credit line is built.

## Unverified

- The "API recommended practices" page (HTTP 403 to scripts, 2026-11-17).
- Whether v1 is deprecated: v2 is the documented current version; no deprecation date was found.
- Other `interval` values; behaviour of `fields` on `/v2/observations/histogram`.
