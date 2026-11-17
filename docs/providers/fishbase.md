# FishBase species traits, and agency fallbacks

Verified on **2026-11-17**. FishBase has no public web API and its pages are not a data release, so the
source is the parquet snapshot that the rOpenSci package `rfishbase` (5.0.4) reads. Recorded extracts
are in `tests/fixtures/raw/fishbase/` (indexed in its README). Everything below was read from the
release itself or from the cited pages; items that could not be checked are marked **unverified**.

## The release

| Item | Value |
|---|---|
| Host | Source Cooperative bucket `us-west-2.opendata.source.coop`, prefix `cboettig/fishbase/` (public AWS S3, anonymous) |
| Layout | `{fb,slb}/v<version>/parquet/<table>.parquet`; `fb` is FishBase, `slb` is SeaLifeBase |
| Base URL used | `https://s3.us-west-2.amazonaws.com/us-west-2.opendata.source.coop/cboettig/fishbase/fb/v26.06/parquet/` |
| Listing request | `GET https://s3.us-west-2.amazonaws.com/us-west-2.opendata.source.coop?list-type=2&prefix=cboettig/fishbase/fb/&delimiter=/` (S3 ListObjectsV2, XML) |
| FishBase versions present | `v19.04`, `v21.06`, `v23.01`, `v23.05`, `v24.07`, `v25.04`, `v26.06` (latest). Tags are the snapshot month of the database dump. |
| Size of `fb/v26.06` | 222 table files, 139,929,828 bytes in total (sum of the listing). Files used: `species` 3.5 MB, `stocks` 2.5 MB, `estimate` 2.4 MB, `spawning` 0.35 MB, `ecology` 0.9 MB, `reproduc` 0.38 MB, `refrens` (references) not sized here. |
| Format | one zstd-compressed parquet file per table; raw export of FishBase's MySQL backend ("highly unintuitive", tables "not developed to support direct access by end users", per the dataset card) |
| Object date | `species.parquet` `Last-Modified: Fri, 04 Sep 2026 22:55:23 GMT` |
| Authentication, limits | none. No rate limit is published; the extracts were made with HTTP range requests (DuckDB) so only the needed column chunks were read, not the 140 MB. |
| Earlier host | Hugging Face dataset `cboettig/fishbase` (rfishbase 5.0.0 to 5.0.1). rfishbase 5.0.2 (2025) moved to Source Cooperative; the Hugging Face copy was last modified 2025-12-16 and is older than `v26.06`. |
| Code that reads it | rfishbase 5.0.4 (`R/fb_tbl.R`, `data-raw/README.md`; the package code is CC0). `version = "latest"` picks the highest `v*` prefix in the bucket. |
| Release cadence | roughly yearly, when the FishBase team sends a database dump to the maintainer; the publishing steps are documented in rfishbase `data-raw/README.md`. |
| Discrepancy | The dataset card says the current releases are `fb/v26.07` and `slb/v26.04`. On 2026-11-17 the bucket lists `fb/v26.06` and `slb/v26.06`, and `fb/v26.07/parquet/species.parquet` answers 404. The card is out of date; read the listing, never the card, for the version. |

**Recommendation.** Take species traits from `fb/v26.06` by pinned version, converted once at build time
(by a developer tool, not by the app) into the bundled JSON. The app never contacts the bucket. The
FishBase team itself offers no bulk download on its site that was found (the site mentions a CD-ROM);
whether the team supplies dumps to others directly is **unverified**.

## Licence and citation

| Item | Statement | Where |
|---|---|---|
| Licence | "This work is licensed under Attribution-NonCommercial 4.0 International" (Creative Commons **CC BY-NC 4.0**) | FishBase "How to cite FishBase" page, `https://www.fishbase.se/summary/citation.php`, read 2026-11-17 |
| Dataset card | front matter `license: cc-by-nc-4.0`; "License: CC-BY-NC, FishBase.org Team"; "Reuse of this data is not licensed for commerical applications. Developers should contact the FishBase team directly." | `.../cboettig/fishbase/README.md` (kept in the fixtures); the Hugging Face tag is also `license:cc-by-nc-4.0` |
| Reuse | "You are welcome to include text, numbers and maps from FishBase in your own web sites for non-commercial use, given that such inserts are clearly identified as coming from FishBase, with a backward link to the respective source page." | citation page |
| Cite FishBase | "Froese, R. and D. Pauly. Editors. 2026. FishBase. World Wide Web electronic publication. www.fishbase.org, (06/2026)" (the page prints the current month; use the snapshot month, here 06/2026) | citation page |
| Cite records | "To give due credit to the original authors, please cite data taken from FishBase by Main Ref. and/or Data Ref. of the respective record." Each of the reference numbers (`*Ref`, `SourceRef`, `SpawningRefNo`) resolves in the `refrens` table to author, year and title (163 cited ones are in the extract). | citation page |
| Older citation | "Froese, R. and D. Pauly, Editors. 2000. FishBase 2000: concepts, design and data sources", ICLARM, Los Baños, 344 pages, which FishBase also asks users to cite for concepts and design | `https://fishbase.se/manual/FishbaseHow_to_cite_FishBase.htm` |
| Photos and drawings | belong to the named persons or organisations with their own licences; **not used** (the app takes no images from FishBase) | citation page |
| Disclaimer | "we can not exclude errors, and neither we nor our partners can be held responsible for any damage" | citation page |
| Licence version | 4.0 only. The licence-link target (`creativecommons.org/licenses/by-nc/4.0/`) was not fetched; the page names "Attribution-NonCommercial 4.0 International" and the card says `cc-by-nc-4.0`. |

Consequences for the app (the first three restate ADR-006):

- Bundled FishBase values need the CC BY-NC 4.0 notice, the FishBase citation above and, per value, the
  reference (author and year) behind it. The "backward link to the respective source page" implies a
  link per species to its FishBase page; the exact URL pattern of a species summary page was not
  checked (**unverified**).
- The app must stay free and non-commercial. Whether a free app distributed through the App Store (with
  a paid developer account) counts as "non-commercial" is a licence interpretation that was not
  checked with FishBase; the dataset card tells developers to contact the FishBase team (**unverified**,
  worth one email before release).
- The CC BY-NC 4.0 files and the fixture extracts are not MIT-licensed. If the repository is public,
  the data files (`crates/underbite-core/data/species/fishbase/*.json` and `tests/fixtures/raw/fishbase/`)
  need their own licence notice in the directory so the MIT licence of the code is not read as covering
  them.
- A CC BY-NC dataset cannot be redistributed under an open licence; if the project ever publishes the
  bundled catalogue as data, the FishBase part has to be excluded or relicensed by the FishBase team.

## Tables and fields

All tables are keyed by `SpecCode` (an integer species id); several also carry `StockCode`. Missing
values are SQL nulls (no sentinel numbers were seen). Boolean flags are `0` / `-1` in `ecology` and
`0` / `1` in `species` and `stocks` (observed in the extract; the meaning of `-1` as true is inferred
from the values and is **unverified** against FishBase's field documentation). Temperatures are in
degrees Celsius and depths in metres (**unverified** against the FishBase field documentation, which
was not fetched; plausible values and the references support it).

| Need | Table.field | Level | Notes from the extract (33 species) |
|---|---|---|---|
| Scientific and common name | `species.Genus`, `Species`, `FBname`, `Author` | species | `FBname` is FishBase's single English name; it differs from anglers' names (`Cynoscion nebulosus` is "Spotted weakfish", `Salmo trutta` is "Sea trout", `Sciaenops ocellatus` is "Red drum", `Coryphaena hippurus` is "Common dolphinfish"). More names are in `comnames` (10 MB; not extracted). |
| Water type | `species.Fresh`, `Brack`, `Saltwater` | species | all 33 filled. Anadromous species show several flags (`Morone saxatilis` 1/1/1, `Salvelinus fontinalis` 1/1/1). `AnaCat` gives the migration class. |
| Habitat class | `species.DemersPelag` (`demersal`, `pelagic`, `benthopelagic`, ...) | species | filled for the species checked |
| Habitat features | `ecology.Stream`, `Lakes`, `Estuaries`, `Mangroves`, `Neritic`, `Epipelagic`, `Benthic`, `Demersal`, `Pelagic`, `Rocky`, `Sand`, `Mud`, `Gravel`, `SeaGrassBeds`, `CoralReefs`, `Vegetation`, `Pilings`, ... | stock (`StockCode`) | one row per species except `Salmo trutta` (4). Coarse: for example `Centropristis striata` and `Thunnus atlanticus` have every flag 0. |
| Light preference | `ecology.Circadian1` to `Circadian3`, `RemarksCircadian` | stock | **empty for all 33 species** |
| Depth range | `species.DepthRangeShallow`, `DepthRangeDeep` (and `DepthRangeRef`) | species | filled for 29 of 33; one range per species, no seasonal split; often only one end (`Morone saxatilis` 30 and no deep end) |
| Common depth range | `species.DepthRangeComShallow`, `DepthRangeComDeep` | species | filled for 11 of 33 |
| Modelled depth | `estimate.DepthMin`, `DepthMax`, `ComDepthMin`, `ComDepthMax`, with `DepthMinEstimate`, `DepthMaxEstimate` flags | species | filled for all 33; computed, not observed; method not checked (**unverified**) |
| Tolerated temperature | `stocks.TempMin`, `TempMax` (`TempRef`) | stock | filled for 28 of 33 species; for example `Micropterus dolomieu` 8.5 to 29.5, `Salvelinus fontinalis` 0.0 to 25.3. `TempRef` points to literature (`refrens`): Scott and Crossman 1973, Beitinger and Bennett 2000, Eaton et al. 1995, among others. Semantics (lethal limit, tolerance or occurrence range) differ by reference and are not stated in the table. |
| Preferred temperature | `stocks.TempPreferred`, `TempPref25`, `TempPref50`, `TempPref75` (`TempPrefRef`) | stock | only 19 of 33 species, **only marine and diadromous ones**; every `TempPrefRef` seen is Cheung, Watson and Pauly 2013 ("Signature of ocean warming in global fisheries catch"), a model output. No freshwater-only species has it (bass, walleye, pike, muskie, panfish, catfish, lake trout). |
| Modelled preferred temperature | `estimate.TempPrefMin`, `TempPrefMean`, `TempPrefMax` (`nCells`) | species | 20 of 33 (the 19 above plus brook trout); from a species distribution model (the AquaMaps method is the likely origin, **unverified**) |
| Spawning temperature | `spawning.TempLow`, `TempHigh` | per study and place (`SpawningRefNo`, `Spawningarea`, `C_Code`) | 18 of 33 species have at least one row with a value; none for most tunas, bluefish, summer flounder, black sea bass, redfish, snook, tarpon, dolphinfish, bluegill, flathead catfish, white crappie |
| Spawning months | `spawning.Jan` to `Dec` | per study and place | the month columns hold the value `111` in the months where spawning is reported (the meaning of `111` as a presence flag is inferred, **unverified**); 32 of 33 species have rows with months; `Pomoxis annularis` has none |
| Spawning place | `spawning.SpawningGround` (`riverine`, `lacustrine`, `estuarine`), `Spawningarea` (free text, e.g. "Delaware Bay"), `C_Code` (ISO 3166 numeric: `840` is the United States, `124` Canada; suffixed codes such as `840A` and `840B` also occur and their meaning is **unverified**), `AddInfos` (free text, e.g. "Spawning activity commences during spring at water temperature of 16 °C") | per study and place | gives regional spawning windows for some species (striped bass: Chesapeake Bay May, Delaware Bay April and May, Gulf of Maine June) |
| Reproductive mode | `reproduc.ReproMode`, `Spawning` (pattern), `SpawnAgg`, `RepGuild1`, `ParentalCare` | stock | filled for most; useful for notes such as "male guards the nest" |
| Game fish flag | `species.GameFish`, `GameRef` | species | |
| Distribution | `stocks.Northernmost`, `Southermost`, `Westernmost`, `Easternmost`, `StockDefs` (free text), `EnvTemp` (`tropical`, `subtropical`, `temperate`, ...) | stock | `StockDefs` describes the range in words |
| Tide, current preference | none | — | no FishBase field |
| Lures | none | — | no FishBase field (`UsedasBait` exists but is about the species as bait) |
| Typical depth by season | none | — | FishBase has one depth range per species, not by season |

### Per stock, per study or per species

- `species` and `estimate` are per species: one row each.
- `stocks` is per stock. `Level` says what a row describes: in the extract's 71 rows, every one of the
  33 species has a "species in general" row, and there are further rows for 22 "cultured strain", 8
  "wild stock/population", 5 "subspecies" and 3 "hybrid" (`Cyprinus carpio` has 29 rows, `Salmo trutta`
  7, `Salmo salar` and `Thunnus thynnus` 3 each). Temperature range and preferred temperature live
  here. The "species in general" row is the one to use for a species profile; a regional population row
  could refine it, but regions were not mapped. For `Salmo trutta` two rows carry different ranges
  (18.0 to 24.0 and 2.0 to 16.0), and the species table below lists both.
- `ecology`, `reproduc`: per stock (one row per species except `Salmo trutta`, 4 rows).
- `spawning`: per study and place, with its own reference; several rows per species (203 rows for 33
  species; `Cyprinus carpio` 36, `Salmo trutta` 24). Ranges from different studies must be combined by
  the importer (for example the widest and the median), and the rule recorded with the value.
- `Oncorhynchus mykiss` is one species; FishBase does not separate the steelhead (sea-run) form from
  the resident rainbow trout. The app's "steelhead" profile cannot come from a distinct FishBase row.

### Quality warnings seen in the extract

- Preferred temperatures disagree between FishBase's own tables: `Morone saxatilis` `stocks.TempPreferred`
  20.0 (p25 to p75 16 to 25) against `estimate.TempPrefMean` 9.3 (1.0 to 23.5); `Thunnus atlanticus`
  26.0 against 14.4. The modelled values are not suitable for "preferred water temperature" in a
  fishing card without review.
- The salmon `TempPreferred` values (3 to 4 °C) describe the ocean phase, not the spawning river.
- Freshwater species have no preferred temperature at all, so the temperature range card would depend on
  `TempMin`/`TempMax` (tolerance) and on the spawning temperatures.
- Freshwater depth ranges are thin (`Micropterus salmoides` 0 to 6 m) and come from one reference.

## Species and coverage

The ARCH list resolved to FishBase species as follows (names the app shows come from the catalogue, not
from `FBname`). Extra species beyond the list: Pomoxis annularis (white crappie), Pylodictis olivaris
(flathead catfish), Thunnus atlanticus (blackfin tuna), Katsuwonus pelamis (skipjack), Oncorhynchus
nerka (sockeye), O. kisutch (coho), O. tshawytscha (Chinook), Salmo salar (Atlantic salmon). Not
extracted: blue catfish, bullhead, pink and chum salmon, rock bass, other tunas (bigeye, longfin) and
Spanish or king mackerel.

Table of `fb/v26.06` values, one row per species. Columns: `DepthRange` is
`species.DepthRangeShallow`-`Deep` (m); `Com` is the common depth range; `TempMin-Max` is
`stocks.TempMin`-`TempMax` over the species' stocks; `TempPref` is `stocks.TempPreferred` with p25 to p75;
`Estimate` is `estimate.TempPrefMin/Mean/Max`; `spawn temp` is the lowest `TempLow` to the highest
`TempHigh` over `spawning`; `-` is null. All of these are **CC BY-NC 4.0** (FishBase). Species-by-source
columns for the agencies follow after the table.

| SpecCode | Species | FBname | F/B/S | DepthRange | Com | stocks | TempMin-Max | TempPref (p25-p75) | Estimate | spawn rows | spawn temp | rows with months | ecology rows |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 345 | Centropomus undecimalis | Common snook | 111 | - to 22 | - | 1 | 25.0-31.0 | 27.0 (27-27) | 23.5/27.3/28.1 | 2 | - | 2 | 1 |
| 361 | Centropristis striata | Black seabass | 001 | 1 to - | - | 1 | - | 19.0 (14-23) | 10.8/23.6/24.9 | 1 | - | 1 | 1 |
| 6 | Coryphaena hippurus | Common dolphinfish | 011 | 0-85 | 5-10 | 1 | 21.0-30.0 | 28.0 (26-28) | 18.1/27.4/29.1 | 15 | - | 14 | 1 |
| 405 | Cynoscion nebulosus | Spotted weakfish | 011 | 10 to - | - | 1 | - | 23.0 (20-26) | 12.0/24.3/26.7 | 3 | 20.0-32.0 | 3 | 1 |
| 1450 | Cyprinus carpio | Common carp | 110 | 0-29 | - | 29 | 3.0-35.0 | - | - | 36 | 7.7-28.0 | 36 | 1 |
| 258 | Esox lucius | Northern pike | 110 | 0-30 | 1-5 | 1 | 10.0-28.0 | - | - | 11 | 4.4-15.3 | 11 | 1 |
| 2711 | Esox masquinongy | Muskellunge | 100 | - | - | 1 | - | - | - | 1 | 9.0-15.0 | 1 | 1 |
| 290 | Ictalurus punctatus | Channel catfish | 100 | 0-15 | - | 1 | 10.0-31.6 | - | - | 3 | 18.0-30.0 | 3 | 1 |
| 107 | Katsuwonus pelamis | Skipjack tuna | 001 | 0-260 | 0 to - | 1 | 14.7-30.0 | 27.0 (24-27) | 13.3/26.2/29.0 | 5 | - | 5 | 1 |
| 3375 | Lepomis macrochirus | Bluegill | 100 | - | - | 1 | 1.0-36.0 | - | - | 2 | - | 2 | 1 |
| 1079 | Megalops atlanticus | Tarpon | 111 | 0-40 | 0-15 | 1 | 10.0-40.0 | 26.0 (25-27) | 19.6/27.0/28.0 | 4 | - | 4 | 1 |
| 3382 | Micropterus dolomieu | Smallmouth bass | 100 | 1-7 | - | 1 | 8.5-29.5 | - | - | 4 | 12.8-22.0 | 4 | 1 |
| 3385 | Micropterus salmoides | Largemouth black bass | 100 | 0-6 | - | 1 | 10.0-31.7 | - | - | 5 | 15.0-25.0 | 5 | 1 |
| 353 | Morone saxatilis | Striped bass | 111 | 30 to - | - | 1 | 8.0-25.0 | 20.0 (16-25) | 1.0/9.3/23.5 | 6 | 14.0-25.0 | 5 | 1 |
| 245 | Oncorhynchus kisutch | Coho salmon | 111 | 0-250 | - | 1 | 0.0-24.8 | 3.0 (3-6) | 1.0/3.9/8.9 | 2 | 1.0-8.0 | 1 | 1 |
| 239 | Oncorhynchus mykiss | Rainbow trout | 111 | 0-200 | - | 1 | 10.0-24.0 | 8.0 (6-12) | 1.3/5.4/10.0 | 7 | 4.0-19.0 | 6 | 1 |
| 243 | Oncorhynchus nerka | Sockeye salmon | 111 | 0-250 | - | 1 | 0.0-24.9 | 3.0 (3-6) | 1.0/4.2/8.4 | 4 | - | 3 | 1 |
| 244 | Oncorhynchus tshawytscha | Chinook salmon | 111 | 0-375 | - | 1 | 0.0-24.9 | 4.0 (4-8) | 0.7/3.5/8.0 | 5 | 4.0-14.5 | 5 | 1 |
| 1338 | Paralichthys dentatus | Summer flounder | 011 | 10-183 | - to 37 | 1 | - | 13.0 (12-20) | 9.1/19.1/24.4 | 3 | - | 3 | 1 |
| 359 | Perca flavescens | American yellow perch | 110 | 0-56 | - to 9 | 1 | 0.0-29.7 | - | - | 2 | 6.7-12.2 | 2 | 1 |
| 364 | Pomatomus saltatrix | Bluefish | 011 | 0-200 | - | 1 | - | 26.0 (25-28) | 8.4/21.1/27.5 | 8 | - | 8 | 1 |
| 3387 | Pomoxis annularis | White crappie | 100 | - | - | 1 | - to 31.3 | - | - | 0 | - | 0 | 1 |
| 3388 | Pomoxis nigromaculatus | Black crappie | 100 | 0 to - | - | 1 | 14.0-30.6 | - | - | 2 | 14.0-22.0 | 2 | 1 |
| 3051 | Pylodictis olivaris | Flathead catfish | 100 | - | - | 1 | - to 32.5 | - | - | 1 | - | 1 | 1 |
| 236 | Salmo salar | Atlantic salmon | 111 | 0-210 | 10-23 | 3 | 2.0-9.0 | 10.0 (6-12) | 1.5/9.3/15.3 | 15 | - | 15 | 1 |
| 238 | Salmo trutta | Sea trout | 111 | 0-28 | 1-2 | 7 | 18.0-24.0; 2.0-16.0 | 9.0 (9-10) | 6.5/10.1/15.8 | 24 | 6.0-12.4 | 24 | 4 |
| 246 | Salvelinus fontinalis | Brook trout | 111 | 15-27 | - | 1 | 0.0-25.3 | - | 1.5/4.5/10.7 | 4 | 3.0-10.0 | 4 | 1 |
| 248 | Salvelinus namaycush | Lake trout | 100 | 3-61 | 18-53 | 1 | 4.0-13.0 | - | - | 8 | 8.9-14.0 | 8 | 1 |
| 3516 | Sander vitreus | Walleye | 110 | 0-27 | - | 1 | 1.1-29.0 | - | - | 1 | 5.6-11.1 | 1 | 1 |
| 1191 | Sciaenops ocellatus | Red drum | 011 | 10 to - | - | 1 | 15.0-26.0 | 25.0 (24-28) | 15.5/24.2/27.3 | 2 | - | 2 | 1 |
| 143 | Thunnus albacares | Yellowfin tuna | 011 | 1-1602 | 1-250 | 1 | 15.0-31.0 | 28.0 (24-28) | 16.5/26.7/28.9 | 13 | 25.0-26.0 | 13 | 1 |
| 144 | Thunnus atlanticus | Blackfin tuna | 001 | 0-50 | - | 1 | 20.0 to - | 26.0 (26-28) | 10.0/14.4/20.5 | 2 | - | 2 | 1 |
| 147 | Thunnus thynnus | Atlantic bluefin tuna | 011 | 0-985 | 0-100 | 3 | 3.0-30.0 | 26.0 (19-27) | 7.5/12.6/24.7 | 2 | - | 2 | 1 |

`F/B/S` is `Fresh`, `Brack`, `Saltwater`. "x to -" means one end is null.

## Species x field x source x licence

Sources: **FB** is FishBase `fb/v26.06` (CC BY-NC 4.0). **NOAA** is a NOAA Fisheries species page.
**USGS NAS** is a USGS Nonindigenous Aquatic Species fact sheet. **EPA** is the Eaton et al. (1995)
literature value. **State** is a state agency page. Cells say whether the source has the field for the
species; "text" is a narrative sentence, not a number. For FishBase the table above gives the numbers.

| Field | FishBase (CC BY-NC 4.0) | NOAA Fisheries pages (public domain text) | USGS NAS fact sheets (public domain) | EPA / literature and state pages |
|---|---|---|---|---|
| Water type | all 33 | text, per species page | text (non-native only) | state pages not checked (**unverified**) |
| Preferred temperature | 18 marine and diadromous species (modelled) | not given (striped bass page checked) | not structured | Eaton et al. 1995 covers about 30 US freshwater fishes as maximum weekly mean temperatures (journal article, AFS; reuse terms **unverified**; values not read) |
| Tolerated temperature | 28 of 33 | not given | not structured | as above |
| Spawn temperature | 18 of 33 | not given (season only) | text only for some species (common carp: spring spawning, no number read) | state pages: Michigan DNR "Smallmouth bass" (2019) is cited by FishBase for 16 °C; state terms **unverified** |
| Spawn season | 32 of 33, per place | text ("In the spring...", striped bass) | text | state pages **unverified** |
| Depth range | 29 of 33 (one range) | not given | not given | **unverified** |
| Habitat | flags for all 33 (coarse) | text | text | **unverified** |
| Tide, current, light | none | none | none | none |
| Lures | none | none | none | state pages **unverified** |

### Agency fallbacks

- **NOAA Fisheries species pages** (`https://www.fisheries.noaa.gov/species/<slug>`, checked for
  `atlantic-striped-bass` on 2026-11-17). Narrative text on habitat, range, reproduction and
  management; photographs credited to individuals. No temperature, depth or numeric spawn field was
  found, no API or download is offered, and each page is an HTML page: there is no structured source to
  parse. Terms (`https://www.fisheries.noaa.gov/national/about-us/website-policies-and-disclaimers`,
  read 2026-11-17): "Information created by the U.S. government... is not subject to copyright in the
  United States", "Most of the text found on our webpages or in publications for download is not
  protected by copyright in the United States. We identify copyrighted text when used", NOAA-created
  photos may be used without permission with credit to "NOAA", and externally produced text, graphics and
  photos used with permission "may not be used in whole or in part without the express written
  permission of the copyright holder". Seals and logos need written permission. So a short factual
  statement may be taken with the credit "NOAA Fisheries" and the page URL; images are not usable.
- **USGS Nonindigenous Aquatic Species** (`https://nas.er.usgs.gov/`). The species search API
  (`https://nas.er.usgs.gov/api/v2/species/search?genus=Cyprinus&species=carpio`) answered with a JSON
  record (`speciesID` 4, `itis_tsn` 163344, family, native or exotic, `Fresh/Marine/Brackish`). The
  fact sheet page (`FactSheet.aspx?speciesID=4`) has biology text (spawning in spring in temperate
  regions, no temperature figure in the lines read). Only species that are non-native somewhere in the
  US have a sheet, so it covers carp, brown trout, rainbow trout, Atlantic salmon, some of the bass and
  panfish where introduced, and not striped bass in its native range (**unverified** per species).
  Terms (`https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits`): "USGS-authored
  or produced data and information are considered to be in the U.S. Public Domain", with the request
  "we ask that proper credit be given" ("Credit: U.S. Geological Survey"); some photographs and
  illustrations are copyrighted and marked so.
- **EPA and journal literature.** Eaton et al. 1995 (*Fisheries* 20(4): 10-18) gives maximum weekly mean
  temperatures for about 30 US species from a field database; it is the `TempRef` of several FishBase
  rows (`TempRef` 12741). The paper is an American Fisheries Society journal article, so its tables are
  not automatically public domain even though EPA scientists wrote it (**unverified**). The tables were
  not read.
- **State agencies.** One New York page (`dec.ny.gov/things-to-do/fishing/fish-species-information`)
  answered 404 and was not read; state pages (NYSDEC, Michigan DNR and others) were not checked for
  fields or terms. **Unverified.** Terms differ per state; the catalogue should link to them rather than
  copy their text unless the specific terms allow it (see `docs/providers/regulations.md` for the link
  catalogue).

Conclusion on ADR-006's wording "agency values win": the agency pages offer prose, not numbers, so there
is almost nothing for an agency value to override. A bundled agency layer would be hand-curated from
cited agency or published literature values per species, with the reference attached, rather than parsed
from a feed. Each such value carries `PublicDomain` only if its source page says so; journal values keep
their own terms.

## Unverified items

- FishBase field definitions (units, `-1` flags, the `111` month marker, whether `TempMin`/`TempMax` are
  tolerance limits or occurrence ranges) were not read from FishBase's own documentation.
- The modelling method behind `estimate.TempPref*` and `estimate.Depth*`.
- The meaning of suffixed `C_Code` values such as `840A` and `840B`.
- Whether the App Store distribution of a free app is "non-commercial" for FishBase; a contact to the
  FishBase team (the dataset card says developers should contact them; rfishbase NEWS lists
  `fishbase@q-quatics.org` for mistakes and taxonomic reports, not for licensing).
- The URL pattern of a FishBase species page for the required backward link.
- The Creative Commons deed page for CC BY-NC 4.0 was not fetched.
- Whether FishBase or its team offers a download other than this parquet snapshot.
- NOAA species pages other than `atlantic-striped-bass`; USGS NAS fact sheets other than common carp;
  every state agency page.
- Eaton et al. 1995 species values and their reuse terms.
- FishBase web pages were read only for the licence and citation text, not for data. `fishbase.se`
  answered 200 to the citation page; `fishbase.ca` and `fishbase.mnhn.fr` answered 403 to scripted
  requests.
