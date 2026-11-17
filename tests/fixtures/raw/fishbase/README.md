# Raw fixtures: FishBase data release (parquet extract)

Recorded on 2026-11-17. No key or account is involved. See docs/providers/fishbase.md.

Source: the `cboettig/fishbase` dataset on Source Cooperative (the release rfishbase 5.0.4 reads),
FishBase snapshot `fb/v26.06`, one zstd parquet file per database table. Base URL:
`https://s3.us-west-2.amazonaws.com/us-west-2.opendata.source.coop/cboettig/fishbase/fb/v26.06/parquet/`

Licence: Creative Commons Attribution-NonCommercial 4.0 International (CC BY-NC 4.0), FishBase.org
team. Not licensed for commercial use. Cite as: Froese, R. and D. Pauly. Editors. 2026. FishBase.
World Wide Web electronic publication. www.fishbase.org, (06/2026). Also cite the Main Ref. or Data
Ref. of each record used. These files are CC BY-NC data: keep them out of anything distributed
commercially.

The extracts keep every column of the source table and keep only the rows whose `SpecCode` belongs to
the 33 species of the first catalogue (Morone saxatilis, Pomatomus saltatrix, Paralichthys dentatus,
Centropristis striata, Sciaenops ocellatus, Cynoscion nebulosus, Centropomus undecimalis, Megalops
atlanticus, Coryphaena hippurus, Thunnus thynnus, T. albacares, T. atlanticus, Katsuwonus pelamis,
Micropterus salmoides, M. dolomieu, Sander vitreus, Esox lucius, E. masquinongy, Pomoxis
nigromaculatus, P. annularis, Lepomis macrochirus, Ictalurus punctatus, Pylodictis olivaris,
Salvelinus fontinalis, Salmo trutta, Oncorhynchus mykiss, Salvelinus namaycush, Oncorhynchus
tshawytscha, O. kisutch, Salmo salar, Oncorhynchus nerka, Perca flavescens, Cyprinus carpio). They were
made by reading the remote parquet files with DuckDB (HTTP range requests; the full release is about
140 MB in 222 files and was not downloaded) and writing the filtered rows back as parquet, so the bytes
are not the source's own files; the schema and values are.

- `fishbase-v26.06-species-extract.parquet`: `.../species.parquet`, 33 rows (names, salinity flags, `DemersPelag`, `AnaCat`, depth ranges, `GameFish`)
- `fishbase-v26.06-stocks-extract.parquet`: `.../stocks.parquet`, 71 rows (one per stock; `TempMin`, `TempMax`, `TempPreferred`, `EnvTemp`, bounding box)
- `fishbase-v26.06-estimate-extract.parquet`: `.../estimate.parquet`, 33 rows (modelled `TempPrefMin/Mean/Max`, `DepthMin/Max`, trophic level)
- `fishbase-v26.06-spawning-extract.parquet`: `.../spawning.parquet`, 203 rows (per study and place: month flags, `TempLow`, `TempHigh`)
- `fishbase-v26.06-ecology-extract.parquet`: `.../ecology.parquet`, 36 rows (habitat flags, feeding type)
- `fishbase-v26.06-reproduc-extract.parquet`: `.../reproduc.parquet`, 33 rows (reproductive mode, spawning pattern, guild)
- `fishbase-v26.06-refrens-cited.parquet`: `.../refrens.parquet`, 163 rows (the references cited by the depth, temperature and spawning rows above)
- `source-coop-dataset-README-2026-11-17.md`: `https://s3.us-west-2.amazonaws.com/us-west-2.opendata.source.coop/cboettig/fishbase/README.md` (the dataset card with the licence statement, unmodified)
- `release-listing-fb-2026-11-17.xml`: `https://s3.us-west-2.amazonaws.com/us-west-2.opendata.source.coop?list-type=2&prefix=cboettig/fishbase/fb/&delimiter=/` (S3 listing of the FishBase releases, unmodified)
- `listing-fb-v26.06-parquet-2026-11-17.xml`: `https://s3.us-west-2.amazonaws.com/us-west-2.opendata.source.coop?list-type=2&prefix=cboettig/fishbase/fb/v26.06/parquet/` (S3 listing with the size of each table file, unmodified)
