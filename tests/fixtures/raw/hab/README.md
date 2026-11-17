# Raw fixtures: NOAA HAB forecasts (Gulf of Mexico respiratory irritation forecast)

One line per file: name, exact request URL (no keys were used), recording date. Requests carried `User-Agent: underbite-scout/0.1`. All recorded 2026-11-17. Older fixture: `../tier2/noaa-hab-gulf-rif-currentmodelrun.zip` (2026-11-08).

- `noaa-hab-gulf-rif-currentmodelrun-2026-11-17.zip`: `https://nccospublicstor.blob.core.windows.net/hab-data/no_Explorer/rif_model/csv/RIFv2_currentmodelrun.zip` (16.9 KB; `Last-Modified: Tue, 17 Nov 2026 08:09:14 GMT`; 13 files)
- `RIFv2_data_dictionary.csv`: extracted unchanged from the zip above
- `RIFv2_README.txt`: extracted unchanged from the zip above
- `RIFv2_modelrun_202611170300-EST_forecast_valid_01hrs.csv`: extracted unchanged from the zip above (22 rows; the first of 11 forecast hours)

## Licence

The forecast is a NOAA National Centers for Coastal Ocean Science product; NOAA content is public domain. The
rows also carry cell counts collected by third parties (Florida Fish and Wildlife Research Institute,
HABscope, Pinellas County and the Sanibel-Captiva Conservation Foundation, see docs/providers/hab.md) and
wind from the National Digital Forecast Database. The product page states no terms for the third-party
counts, so treat them as credited sources whose terms are unstated.
