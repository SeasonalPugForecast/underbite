# New York State fish stocking (Socrata) and matching to 3DHP names

Verified on 2026-11-17 (UTC) with `curl` and a small script against `data.ny.gov` (Socrata SODA 2), the Census TIGERweb county layer and the USGS 3DHP map service. An earlier survey of the two datasets is in `docs/providers/tier2.md`; the 3DHP service is described in `docs/providers/hydro.md`. Fixtures: `tests/fixtures/raw/stocking-ny/` (index in its `README.md`).

## Datasets

| Dataset | Title | Rows last modified (`X-SODA2-Truth-Last-Modified`) | Fields |
|---|---|---|---|
| `e52k-ymww` | Fish Stocking Lists (Actual): Beginning 2011 | 2026-06-24 | `year`, `county`, `waterbody`, `town`, `month`, `number`, `species`, `size_inches` |
| `d9y2-n436` | Current Season Spring Trout Stocking | 2026-03-19 | `year`, `dec_region`, `county`, `town`, `waterbody`, `date`, `number`, `species_name`, `size_inches` |

Base `https://data.ny.gov/resource/<id>.json`, SoQL parameters `$select`, `$where`, `$group`, `$order`, `$limit`. Every value is a JSON string, including aggregates (`n`). Neither dataset has coordinates, a waterbody id or a license field. Licence and limitations text: see `tier2.md` (OPEN-NY Terms of Use: no attribution required, non-exclusive revocable licence; crediting NYS DEC is advisable; stocking numbers are estimates, published about mid-May of the following year; the spring list is planned stocking).

## Throttling without an app token

Socrata's documentation (`https://dev.socrata.com/docs/app-tokens.html`, read 2026-11-17) says requests without an application token "come from a shared pool via IP address", that requests with a token are not throttled unless judged abusive, that the limits may change, and that a throttled request gets HTTP 429. No numeric limit is given. Authentication is an `X-App-Token` header (or the `$$app_token` parameter in SODA 2.1 and earlier). In this check, two aggregate requests (97 KB and 30 KB, 3 s apart) and a few smaller ones drew no 429 and no rate-limit headers (`X-Socrata-RequestId` and `X-Socrata-Region` only). The exact threshold is **unverified**. The data change at most twice a year, so cache the whole pair list for the life of a release and fetch rows for one waterbody only when a spot needs them.

## Distinct `(county, waterbody)` pairs

```text
GET https://data.ny.gov/resource/e52k-ymww.json?$select=county,waterbody,count(*) as n
    &$group=county,waterbody&$limit=50000
GET https://data.ny.gov/resource/d9y2-n436.json?$select=county,waterbody,count(*) as n
    &$group=county,waterbody&$limit=50000
```

Fixtures `socrata-ny-e52k-ymww-county-waterbody-pairs-2026-11-17.json` (97,406 bytes) and `socrata-ny-d9y2-n436-county-waterbody-pairs-2026-11-17.json` (30,543 bytes).

| | `e52k-ymww` | `d9y2-n436` | both |
|---|---|---|---|
| distinct pairs as published | 1,603 | 508 | 1,632 in the union, 479 in both |
| distinct counties as published | 61 | 57 | |
| distinct pairs after normalisation (county in title case, whitespace collapsed, waterbody lower-cased) | 1,557 | 508 | 1,586 in the union |

Data-quality points found: county spelled in capitals or lower case (`SULLIVAN`, `clinton`, `franklin`, `warren`) alongside the title-case form, `St Lawrence` without the full stop (the Census name is `St. Lawrence`), one waterbody with leading or trailing white space, and agency qualifiers inside the waterbody text: `Patterson Pond (#1)`, `Clear Pond (P594- Upper Hudson)`, `Palmer Pond 2 AS`, abbreviations (`Bog Brook Res`), and names that are not waterbodies at all (`County Fair`). About 56 of the 1,632 names contain parentheses. Essex (220 pairs), Hamilton (173), Warren (115) and Franklin (104) hold the most.

## Name match against 3DHP

Method. 50 pairs were drawn at random (`random.seed(20261117)`) from the 1,632 published pairs. For each, the county polygon was fetched from TIGERweb (`.../TIGERweb/State_County/MapServer/1/query`, `where=STATE='36' AND BASENAME='<county>'`, `outSR=4326`, `maxAllowableOffset=0.003`), and 3DHP `3DHP_all/MapServer` layer 60 (waterbody) and layer 50 (flowline) were queried by POST with the polygon as geometry (`spatialRel=esriSpatialRelIntersects`) and a `where` of `UPPER(gnisidlabel) LIKE '%<token>%'` on the two longest tokens that are not type words. 100 requests, paced 0.5 s apart; no failures. A hit means the 3DHP `gnisidlabel` equals the normalised waterbody name inside the county.

Normalisation that was applied before comparing: lower-case; drop parenthetical text and `#n` numbering; drop any text after ` - `; replace `&` with `and` and `.` with a space; drop trailing digits and the qualifier `AS`; expand abbreviations `Res` to `Reservoir`, `Cr` and `Ck` to `Creek`, `Br` to `Brook`, `Pd` to `Pond`, `Mt` to `Mount`. A second class, "word-set", ignores word order (`Fish Creek East Branch` equals `East Branch Fish Creek`).

Result (fixture `stocking-3dhp-name-match-sample50-2026-11-17.synthetic.json`, hand-reduced from the script output):

| Outcome | Pairs |
|---|---|
| exact name match, in the layer that fits the type (lake, pond, reservoir in layer 60; creek, brook, river, stream in layer 50) | 39 |
| word-order match (layer 50) | 1 |
| **hit** | **40 of 50 (80 %)** |
| only the name core matched with a different type word (`Patterson Pond` to `Patterson Creek`) | 1, counted as a miss |
| no match | 9 |

Details. Layer 60 alone matched 19 of the 50 (all lakes, ponds and reservoirs); layer 50 alone matched 20, plus the word-order match; no lake or pond name was found only in layer 50, and no stream name only in layer 60, so the type word picks the layer. Of the 7 pairs where a feature of a different type shared the name core in the same county (`Moose Pond` and `Moose Creek`, `Rock Pond` and `Rock Pond Brook`, `Oneida Lake` and `Oneida River`, `Canisteo River` and `Canisteo Reservoir`), none may be accepted as a match. The 10 unmatched names are small or private waters and non-waterbody labels: `Upper Saranac Lake`, `Patterson Pond (#1)`, `Upper Wallface Pond`, `Mallet Pond`, `Holliday Creek`, `County Fair`, `Peru Fish & Game Club Pond`, `Lakewood Rod and Gun Channel`, `Oppenheim Park Pond` and `Como Park Lake`. Why `Upper Saranac Lake` has no GNIS-named waterbody in Essex County was not investigated (**unverified**).

Caveats. The sample is 50 pairs spread over 27 counties, so the 80 % figure has a wide margin (about 11 points at 95 % confidence); pairs in the Adirondack counties dominate the dataset. A county polygon that is generalised by 0.003 degrees can let through a same-named feature just across the boundary; the sample did not show one. Several names returned more than one 3DHP waterbody in a county (`Moose Pond` 2 candidates, `Rock Pond` 4, `Mud Pond` 3, counting names that merely contain the token), so a name match gives a candidate, not a unique feature: pick by distance to the spot. Matching is name-based, so two waterbodies with the same name in one county (three candidates were returned for `Mud Pond` in Lewis County) cannot be separated by the stocking data.

Rules for the implementation, from what matched: match by normalised name in the county; try layer 60 for lake, pond and reservoir and layer 50 for creek, brook, river, stream and kill; never accept a core-only match; treat everything that fails as "stocked waterbody, location unknown", still listed by county. A reference table of pairs to 3DHP ids built offline would avoid per-spot queries.

## Recordings with geometry for the catalogue

Recorded on 2026-11-17 so that each matchable `(county, waterbody)` pair can be given a point and checked against its county. Index: `tests/fixtures/raw/stocking-ny/README.md`.

Service facts checked (2026-11-17). Layers 60 and 50 of `3DHP_all/MapServer` have `maxRecordCount` 2500, expose no state or county field (the name is `gnisidlabel`, the id `id3dhp`) and ignore `returnCentroid`. With `returnGeometry=true`, `outSR=4326`, `maxAllowableOffset` and `geometryPrecision` are honoured (Kensico Reservoir at offset 0.003 is 434 bytes). Request time depends on the filter: `gnisidlabel IN ('Moose Pond', ...)` is fast (3 s nationwide, a few seconds per county); `UPPER(gnisidlabel) IN (...)` is about five times slower and drew HTTP 504 for Essex County waterbodies; an envelope over all of New York with `UPPER()` took more than 45 s. A name-only query over a county envelope (`gnisidlabel IS NOT NULL`, `returnDistinctValues=true`, `returnGeometry=false`) is cheap (about 46 KB and 2 s for 729 names).

Request design.

1. Counties: TIGERweb `State_County/MapServer/1/query?where=STATE='36'&outFields=BASENAME,NAME,GEOID&outSR=4326&maxAllowableOffset=0.001&geometryPrecision=4&f=json`, one request, 62 polygons, 115,581 bytes (U.S. Census Bureau). Fixture `tigerweb-ny-counties-generalised-2026-11-17.json`.
2. Names, per county and layer: each pair's name normalised as in the rules above and title-cased word by word, in batches of 20 (the first 29 files, alphabetically up to Essex, were sent in batches of 40 with `UPPER()` on both sides), `where=gnisidlabel IN ('Name', ...)`, `geometry=<county bounding box + 0.01 degree>`, `geometryType=esriGeometryEnvelope`, `inSR=4326`, `outSR=4326`, `outFields=id3dhp,gnisidlabel,featuretypelabel`, `returnGeometry=true`, `maxAllowableOffset=0.003`, `geometryPrecision=4`. A name with a type word goes to its layer only (lake, pond, reservoir to 60; creek, brook, river, stream, kill to 50); any other name goes to both. One file per county and layer (`3dhp-layer<60|50>-county-<county>-names-2026-11-17.json`, 112 files), requests paced at least 1 s apart, no 429, no key.
3. Spelling variants: for the pairs still without a feature, the distinct `gnisidlabel` values in the county envelope were listed per layer (124 small requests, not recorded) and compared with the state name under the catalogue's `name_matches` rule (case, `.`, word order). The exact spellings that match (`McKenzie Pond`, `DeBraine Lake`, `Lake of the Woods`, `East Branch Fish Creek`, `West Branch Croton River`) were fetched with the request of step 2 into 21 files `3dhp-layer<60|50>-county-<county>-spelling-variants-2026-11-17.json`.

The files hold polylines (layer 50) and polygons (layer 60) in the service's JSON: `paths` or `rings` of `[longitude, latitude]` at 4 decimals, generalised by 0.003 degree (about 300 m). A river has many flowline features per name (Mohawk River 479, Catskill Creek 131 in the whole service), so its point is a vertex of one of them, for example the middle vertex of the longest path; a lake or pond point should be a point inside the generalised ring. A feature belongs to the pair's county when that point lies inside the TIGERweb polygon: the envelope query also returns same-named features in neighbouring counties and states, so the check is required.

Sizes (bytes): layer 60 files 594,170; layer 50 files 3,895,048; counties 115,581. With the earlier fixtures the directory is 4.84 MB in 139 files. Layer 50 is 87 % of the volume because of flowline segments.

Coverage of the 1,586 pairs: a normalised name equal to a 3DHP `gnisidlabel` (case-insensitive, any word order) in the county's envelope, then a vertex of one such feature inside the county polygon; counted by script on 2026-11-17.

| | pairs with a same-name feature in the county envelope | with a vertex inside the county polygon |
|---|---|---|
| layer 60 (waterbody) | 580 | 572 |
| layer 50 (flowline) | 536 | 535 |
| either layer | 1,116 | 1,107 |

The layers add up to more than the union because a name without a type word was sent to both. 470 pairs have no same-name feature in either layer. Examples: `Six Mile Waterworks`, `Allen Lake`, `Harwood Lake`, `Lake Erie`, `Lake Ontario`, `Cayuga`, `Genesse River` (a misspelling of Genesee), `Chautauqua Lake` listed under Allegany, and many private, club, hatchery or numbered waters (`County Pond A`, `Springville Field and Stream C`, `Dodge Creek T17`). Why a clearly named lake is absent (`Allen Lake`, `Case Lake`, `Quaker Lake`) was not investigated (**unverified**): it may carry no GNIS name in the service, or be a feature of another type; layers other than 60 and 50 were not searched.

Limits of the check. A vertex test does not prove an interior point lies in the county for a feature that straddles a boundary; pick a point inside both the feature and the county, or reject the feature. The Census generalisation of 0.001 degree (about 100 m) can put a shoreline lake just outside its county. Several same-name features in a county (a river's segments, several `Mud Pond`s) cannot be told apart by the stocking data.

## Request examples

3DHP waterbody by name in an envelope (fixture `3dhp-waterbody-layer60-name-query-kensico-2026-11-17.json`, Kensico Reservoir, Westchester County, `featuretypelabel` "Lake", 5.41 km2):

```text
GET https://hydro.nationalmap.gov/arcgis/rest/services/3DHP_all/MapServer/60/query
    ?where=UPPER(gnisidlabel) LIKE '%KENSICO%'&geometry=-73.78,41.05,-73.7,41.14
    &geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects
    &outFields=id3dhp,gnisid,gnisidlabel,featuretypelabel,areasqkm&returnGeometry=false&f=json
```

The service has no county field, so a county restriction needs a county polygon or envelope, as above.

## Unverified

- The numeric threshold for requests without a token.
- Whether the 80 % figure holds for the 2026 additions to the spring list.
- Behaviour of the pair-list query once the datasets exceed the `$limit=50000` row cap (the pair lists are about 1,600 and 500 rows).
