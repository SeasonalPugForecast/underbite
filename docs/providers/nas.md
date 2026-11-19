# USGS Nonindigenous Aquatic Species (NAS) occurrence API

Verified on 2026-11-17 (UTC) with `curl` against `nas.er.usgs.gov`. An earlier survey is in `docs/providers/tier2.md`. The documentation page `https://nas.er.usgs.gov/api/documentation.aspx` answered 403 to a fetch tool and 200 to `curl` with a plain user agent; the parameter names below come from it. Fixtures: `tests/fixtures/raw/nas/` (index in its `README.md`).

## Endpoint

| Item | Value |
|---|---|
| Base | `https://nas.er.usgs.gov/api/v2/occurrence/search` (the documentation prints `http://`; HTTPS works) |
| Single record | `/api/v2/occurrence/{key}` |
| Authentication | none |
| Headers sent | `User-Agent: underbite-scout/0.1`; responses come through Cloudflare (`server: cloudflare`, `cache-control: private, max-age=0`, `cf-cache-status: BYPASS`) |
| Rate limit | none documented; requests were spaced 3 s apart, no throttling in six requests |
| Format | JSON (`format` parameter exists; the default was JSON) |

## Search parameters

From the documentation: `species_ID`, `group`, `family`, `genus`, `species`, `state`, `county`, `huc8`, `huc10`, `huc12`, `year`, `month`, `day`, `recordType`, `spatialAcc`, `yrAcc`, `refID`, `status`, `callback`, `format`, `limit`, `offset`. `group` is a vernacular class such as `Fishes`, `Crustaceans-Crayfish`, `Mollusks-Bivalves`. `county` is the full county name. `year` accepts a range written with a comma (`year=2020,2026`).

**No bounding box and no radius.** Spatial filtering is by `state`, `county` or HUC (8, 10 or 12 digits), then by coordinate in the adapter.

Verified requests:

| Request | Result |
|---|---|
| `huc8=02030101&limit=5` | HTTP 200, 5,079 bytes; "Lower Hudson" (HUC 02030101, which includes parts of Connecticut and New Jersey). Fixture `nas-occurrence-huc8-02030101-limit5-2026-11-17.json` |
| `state=NY&county=Westchester&limit=5` | HTTP 200, 4,218 bytes. Fixture `nas-occurrence-ny-westchester-limit5-2026-11-17.json` |
| `state=NY&year=2020,2026&limit=1` | HTTP 200; first row a zebra mussel in Oneida County |
| `state=NY&genus=Channa&limit=1000` | HTTP 200, 123,967 bytes, 111 records, `endOfRecords":"true"` |
| `huc8=02030101&offset=5&limit=2` | HTTP 200: records 6 and 7 (the offset is zero-based and skips the first five) |

`state` took the abbreviation `NY`; the records print `"state":"New York"`. The county records in the HUC 02030101 sample include Fairfield (CT), Hudson and Bergen (NJ) and Westchester (NY): the HUC and state filters are independent.

## Response and paging

```json
{"endOfRecords":"false","count":5,"offset":0,"limit":5,"results":[ ... ]}
```

- `endOfRecords` is a **string** (`"true"` or `"false"`). It read `"false"` on a page that returned exactly `limit` rows and `"true"` when 111 records came back for `limit=1000`, so a short page ends the listing.
- `count` is the **number of rows in this page**, not the total (5 with `limit=5`, 1 with `limit=1`, 111 with `limit=1000`). There is no total count in the response.
- The largest `limit` accepted was 1000 (the documented maximum was not found: **unverified**). Page with `offset` until `endOfRecords` is `"true"`.
- Numeric fields can be `null` (`month`, `day`); `date` is text and can be a year only (`"1996"`) or `"1936-7-10"` (no zero padding).

## Fields

| Field | Meaning |
|---|---|
| `key` | record id |
| `speciesID` | NAS species id (usable as `species_ID`) |
| `group`, `family`, `genus`, `species`, `scientificName`, `commonName` | taxonomy |
| `state`, `county`, `locality` | place names (`county` may be empty) |
| `decimalLatitude`, `decimalLongitude` | WGS84 degrees |
| `latLongSource`, `latLongAccuracy` | how the position was derived ("Map derived", "Approximate", "Accurate") |
| `Centroid Type` | a key with a space in its name |
| `huc8`, `huc8Name`, `huc10`, `huc10Name`, `huc12`, `huc12Name` | watershed codes; empty strings when unknown |
| `date`, `year`, `month`, `day` | when collected |
| `status` | `established`, `collected`, `failed`, `extirpated`, ... |
| `recordType` | `Literature`, `Agency`, ... |
| `freshMarineIntro` | `Freshwater`, `Marine`, ... |
| `comments`, `disposal`, `museumCatNumber`, `UUID` | free text, often empty |
| `references[]` | `key`, `refType`, `year`, `author`, `title`, `publisher`, `publisherLocation` |

`references[].author` holds publication authors, not collectors. The fixtures contain no other personal names.

The database holds records of non-native occurrences (the `status` and `freshMarineIntro` fields describe the introduction); it is not a species-presence atlas.

## License and attribution

Public US government data. The site's data disclaimer: the data "vary in accuracy, scale, completeness, extent of coverage and origin. It is the user's responsibility to use these data consistent with their intended purpose and within stated limitations." Publications are asked to contact NAS. Suggested citation: "U.S. Geological Survey. [year]. Nonindigenous Aquatic Species Database. Gainesville, Florida. Accessed [date]."

Re-checked 2026-11-18: unchanged. `https://nas.er.usgs.gov/api/documentation.aspx` carries the same data disclaimer and the citation form "U.S. Geological Survey. [2026]. Nonindigenous Aquatic Species Database. Gainesville, Florida. Accessed [11/18/2026].", and asks users who publish to contact NAS to make sure the data are used appropriately.

## Unverified

- The documented maximum for `limit`.
- Whether the API sets any rate limit (none seen).
- How often the data are updated.
