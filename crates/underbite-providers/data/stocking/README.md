# Stocked waterbody catalogue (New York)

`ny.json` lists every `(county, waterbody)` pair that appears in the New York State DEC stocking lists
(Socrata datasets `e52k-ymww`, actual, and `d9y2-n436`, planned spring trout), each either placed on the
map or listed as unmatched. `src/stocking/catalogue.rs` loads it (`Catalogue::bundled`) and holds the
name matcher.

## Format

- `placed`: `county`, `waterbody` (as the state writes it), `name` (the map feature), `latitude`,
  `longitude`, `kind` (`Lake`, `River` or `Pond`), `source` (a 3DHP feature id, or `manual`). A river is a
  line: its point is a representative point on the line, not where fish were released.
- `unmatched`: `county`, `waterbody`, and a `note` where a recorded response identified a feature but
  gave no point.
- Both lists are sorted by county and lower-cased waterbody, one entry per line. A name in two counties
  is two entries; no entry is ever placed outside its county.
- Counties are title case with `St. Lawrence`; the waterbody is the most frequent published spelling with
  white space collapsed. Lookup ignores case and white space.

## Method

Built from recorded responses only, no live requests (fixtures in `tests/fixtures/raw/stocking-ny/`,
all recorded 2026-11-17):

- `socrata-ny-e52k-ymww-county-waterbody-pairs-2026-11-17.json` and
  `socrata-ny-d9y2-n436-county-waterbody-pairs-2026-11-17.json`: the distinct pairs. Their union after
  normalising the county and the waterbody's case and white space is 1,586 pairs.
- `3dhp-waterbody-layer60-name-query-kensico-2026-11-17.json`: one 3DHP waterbody (`JLOQT`, Kensico
  Reservoir) found by name in an envelope over Westchester County.
- `stocking-3dhp-name-match-sample50-2026-11-17.synthetic.json`: a 50-pair sample of name matches, used
  only for the matching rules below.

Matching rules (`normalise`, `name_matches`): lower case; drop parenthetical text, `#n` numbering and
anything after ` - `; `&` as `and`, `.` as a space; drop a trailing `AS` and trailing digits; spell out
`Res`, `Cr`, `Ck`, `Br`, `Pd`, `Mt`; compare words in any order. A name core with a different type word
(`Moose Pond`, `Moose Creek`) is rejected. Lakes, ponds and reservoirs belong to 3DHP layer 60, creeks,
brooks, rivers, streams and kills to layer 50.

## Result

None of the recorded responses carries a coordinate (they were fetched without geometry), so no pair can
be placed from them: 0 placed, 1,586 unmatched. Kensico Reservoir is the one pair a response identifies
(its note carries the 3DHP id). Placing the rest needs a recorded 3DHP response with geometry per county,
then a rerun of the build.

## Rebuilding

The build is a test helper in `catalogue.rs`; it rewrites `ny.json` from the fixtures, and a normal test
fails if the file differs from what the build produces:

```text
cargo test -p underbite-providers regenerate_ny_catalogue -- --ignored
```
