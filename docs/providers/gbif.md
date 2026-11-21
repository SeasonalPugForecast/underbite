# GBIF occurrence API

Verified on 2026-11-17 (UTC) with `curl` against `api.gbif.org`. An earlier survey is in `docs/providers/tier2.md`. The GBIF documentation site (`techdocs.gbif.org`) returned only navigation to a scripted fetch and `www.gbif.org/terms/data-user` answered HTTP 403, so limits below are measured, not quoted. Fixtures: `tests/fixtures/raw/gbif/` (index in its `README.md`).

## Endpoint

| Item | Value |
|---|---|
| Base | `https://api.gbif.org/v1/occurrence/search` |
| Authentication | none for search |
| Method | GET, query string |
| Response | JSON, `content-type: application/json`, `cache-control: public, max-age=600` |
| Headers sent | `User-Agent: underbite-scout/0.1`; GBIF asks that clients identify themselves |
| Rate limit | not published; responses carry `x-varnish-rli` and `x-varnish-rlg` headers (values 17 and 250 on the month-facet request), whose meaning is not documented (**unverified**) |

No HTTP 429 occurred in about 8 requests spaced 2 s apart.

## Paging limits (measured 2026-11-17)

| Request | Result |
|---|---|
| `limit=301` | HTTP 200; the response says `"limit":300`: values above 300 are clamped to 300 |
| `limit=300` | HTTP 200, 2,126,671 bytes for 300 striped bass records (about 7 KB per record). Request fewer, or use `facet` for summaries |
| `offset=100001&limit=1` | HTTP 400, body `Max offset of 100001 exceeded: 100001 + 1` (41 bytes): `offset + limit` must not exceed 100,000 |
| `limit=-1` | treated as the default, 20 |
| `limit=0` | HTTP 200 with empty `results`; use it with `facet` and for counts |

`endOfRecords` is a JSON boolean here (unlike USGS NAS). `count` is the total number of matches, not the page size.

## Requests the app needs

Records of one species in a box (scientific name, decimal-degree range filters, only georeferenced):

```text
GET /v1/occurrence/search?scientificName=Morone%20saxatilis
    &decimalLatitude=40.4,41.0&decimalLongitude=-74.4,-73.6&hasCoordinate=true&limit=2&offset=0
```

Fixture `gbif-occurrence-striped-bass-nyharbor-2-records.synthetic.json` (two invented records of this shape; the real `count` for the box was about 350 on 2026-11-17).

Month histogram for the same species and box, with no records:

```text
GET /v1/occurrence/search?scientificName=Morone%20saxatilis
    &decimalLatitude=40.4,41.0&decimalLongitude=-74.4,-73.6&hasCoordinate=true
    &limit=0&facet=month&facetLimit=12
```

Fixture `gbif-facet-month-striped-bass-nyharbor-2026-11-17.json` (397 bytes). The response has `facets[0].field = "MONTH"` and `counts[]` of `{name:"5",count:49}` where `name` is the month number as a string, sorted by count and not by month; sort in the adapter. Values on 2026-11-17: May 49, April 46, June 39, August 34, September 32, October 30, July 29, November 27, March 25, December 17, January 6, February 4 (the counts sum to 338 of the 350 matches; the facet leaves out records with no month). The facet works with `limit=0` and with taxon filters other than names (`taxonKey` was tried with a small result). `facetLimit` accepts at least 12.

Filtering by month is a request parameter (`month=10` or a range such as `month=9,11`); it was not exercised in these requests (**unverified**).

## Fields to read

| Field | Meaning |
|---|---|
| `key`, `gbifID` | identifiers (`key` is a number; `gbifID` is a string) |
| `scientificName`, `species`, `speciesKey`, `taxonKey`, `taxonRank` | identification |
| `decimalLatitude`, `decimalLongitude`, `geodeticDatum` | position (WGS84) |
| `coordinateUncertaintyInMeters` | accuracy (119.0 in the first sample); iNaturalist obscured records carry large values |
| `eventDate` | local date-time without offset in the sample (`2026-03-12T16:20:05`) |
| `year`, `month`, `day` | integers; missing when not known |
| `basisOfRecord` | `HUMAN_OBSERVATION` in the sample |
| `occurrenceStatus` | `DETECTED` |
| `issues[]` | quality flags such as `COORDINATE_ROUNDED` |
| `license` | URL of the licence for the record (`.../by-nc/4.0/legalcode`) |
| `datasetKey`, `datasetName`, `publishingOrgKey` | dataset-level attribution |
| `references`, `occurrenceID` | page of the record at the source (iNaturalist observation URL) |
| `media[]` | `identifier` (photo URL), `license`, `creator`, `rightsHolder`, `references` |

Names of people appear in `recordedBy`, `identifiedBy`, `rightsHolder`, `media[].creator` and `extensions`. The app does not need them; do not store them.

## License and attribution

Each record carries its own `license` (CC0, CC BY, CC BY-NC in the samples) and the media have their own. A commercial or sold app must filter out CC BY-NC records (the request parameter is `license=CC0_1_0,CC_BY_4_0`, not exercised here: **unverified**). Credit text: "GBIF.org" plus the dataset name or the dataset's citation, with the licence. The GBIF data-user terms page could not be read (HTTP 403 on 2026-11-17), so the exact citation wording is **unverified**.

Re-checked 2026-11-18: **unverified**. `https://www.gbif.org/terms/data-user` and `https://www.gbif.org/citation-guidelines` answered HTTP 403 to `curl` and to a plain HTTP client on 2026-11-18. The API documentation (`https://techdocs.gbif.org/en/openapi/`) says rapid or numerous queries to search APIs may be rate limited and that no particular rate is guaranteed.

## Unverified

- The month filter parameter and the licence filter value names listed above.
- The meaning of the `x-varnish-rl*` headers and any numeric rate limit.
- The published maximum for `limit` (300 is measured; 100,000 for `offset + limit` is measured).
