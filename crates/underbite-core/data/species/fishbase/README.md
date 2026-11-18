# FishBase-derived species data

The JSON files in this directory (`salt.json`, `fresh.json`) hold values taken from FishBase.

**Licence: Creative Commons Attribution-NonCommercial 4.0 International (CC BY-NC 4.0), FishBase.org
team.** These files are **not covered by this repository's MIT licence**. They may be used only
non-commercially, with the attribution below, and must be left out of anything distributed
commercially or relicensed as open data.

## Citation

Froese, R. and D. Pauly. Editors. 2026. FishBase. World Wide Web electronic publication.
www.fishbase.org, (06/2026).

Source release: the FishBase parquet snapshot `fb/v26.06` (Source Cooperative, `cboettig/fishbase`,
the release the rfishbase package reads). Each value also names the FishBase reference (author, year,
reference number) it came from and links to the species page on `www.fishbase.se`, as FishBase asks.

## How the values were taken

- Tables: `species`, `stocks`, `estimate`, `spawning` and `refrens` (the recorded extracts in
  `tests/fixtures/raw/fishbase/`).
- Stock rows: the "species in general" row of `stocks`, which every species has.
- Tolerated temperature: `TempMin` to `TempMax` of that row; left out when either end is empty.
- Preferred temperature: the 25th to 75th percentile (`TempPref25` to `TempPref75`) of that row.
  FishBase's separately modelled `estimate.TempPref*` is not used; where it differs from the stock row by
  3 C or more, the value's citation says so.
- Spawning temperature: the widest range over all `spawning` rows with a temperature (lowest low to
  highest high; a row with one end counts for both ends); every contributing reference is listed.
- Depth: the species depth range, or the common depth range when the species range has no reference in
  the extract; left out when either end is empty. FishBase has no seasonal split, so the same range is
  given for every season.
- Water type: from the `Fresh`, `Brack` and `Saltwater` flags.
- A value without a resolvable FishBase reference is not included.

Photographs, drawings and text of FishBase are not used. FishBase gives no guarantee of correctness
("we can not exclude errors", from its citation page).
