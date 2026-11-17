# Stocked waterbody catalogue (New York)

`ny.json` lists every `(county, waterbody)` pair that appears in the New York State DEC stocking lists
(Socrata datasets `e52k-ymww`, actual, and `d9y2-n436`, planned spring trout), each either placed on the
map or listed as unmatched. `src/stocking/catalogue.rs` loads it (`Catalogue::bundled`) and holds the
name matcher.

## Format

- `placed`: `county`, `waterbody` (as the state writes it), `name` (the map feature), `latitude`,
  `longitude`, `kind` (`Lake`, `River` or `Pond`), `source` (a 3DHP feature id, or `manual`), and a `note`
  when several features in the county carry the name. A river is a line: its point is a vertex on the line
  (the middle of its longest path), not where fish were released; a lake or pond point is inside the polygon.
- `unmatched`: `county`, `waterbody`, and a `note` giving the reason.
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
- `3dhp-layer<60|50>-county-<county>-names-2026-11-17.json` (112 files) and
  `...-spelling-variants-2026-11-17.json` (21): the 3DHP features (layer 60 waterbodies as polygons, layer 50
  flowlines as polylines, generalised by 0.003 degree, 4 decimals) whose name is one the county lists.
- `tigerweb-ny-counties-generalised-2026-11-17.json`: the 62 county polygons (Census TIGERweb, generalised by
  0.001 degree).
- `stocking-3dhp-name-match-sample50-2026-11-17.synthetic.json`: a 50-pair sample of name matches, used
  only for the matching rules below.

Matching rules (`normalise`, `name_matches`): lower case; drop parenthetical text, `#n` numbering and
anything after ` - `; `&` as `and`, `.` as a space; drop a trailing `AS` and trailing digits; spell out
`Res`, `Cr`, `Ck`, `Br`, `Pd`, `Mt`; compare words in any order. A name core with a different type word
(`Moose Pond`, `Moose Creek`) is rejected. Lakes, ponds and reservoirs belong to 3DHP layer 60, creeks,
brooks, rivers, streams and kills to layer 50.

Placement (`build` in `catalogue.rs`), per pair:

1. Candidates are the recorded features of the pair's county, deduplicated by layer and id, whose name
   satisfies `name_matches`. A name with a type word looks in its layer only (lake, pond, reservoir: 60;
   creek, brook, river, stream, kill: 50); a name without one tries layer 60, then layer 50.
2. A candidate's point: for a polygon, the midpoint of the widest interior run over 19 horizontal scan lines
   (rounded to 4 decimals, then checked to be inside the polygon); for a river, the vertex halfway along the
   feature's longest path. A point outside the county polygon (even-odd point in polygon on the generalised
   Census ring) rejects the candidate; for a polygon the next scan-line run is tried first, a point is
   never moved to make it fit.
3. Several accepted candidates: the largest waterbody, or the river feature with the greatest total recorded
   length, is placed (ties: the smaller id). The `note` says how many features share the name and names the
   next two. This is a rule of thumb: the stocking lists cannot say which feature is meant, and a river is
   placed at one vertex of one of its many flowline segments.
4. Otherwise the pair is unmatched with a reason.

## Result

1,103 of 1,586 pairs are placed (309 lakes, 260 ponds, 534 rivers; 610 carry an ambiguity note) and 483
are unmatched: 470 with no same-name feature in the recorded responses (private, club and numbered waters,
misspellings, names that are not waterbodies, features without a GNIS name) and 13 whose same-name features
all lie outside the stocked county (a neighbouring county or state, or across a generalised boundary). Every
placed point is inside its county polygon and, for a waterbody, inside its polygon; a test checks both.
Counties and features are generalised, so a point is good to a few hundred metres at best.

## Rebuilding

The build is a test helper in `catalogue.rs`; it rewrites `ny.json` from the fixtures, and a normal test
fails if the file differs from what the build produces:

```text
cargo test -p underbite-providers regenerate_ny_catalogue -- --ignored
```
