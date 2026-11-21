# Raw fixtures: storm outlook and NHC

Recorded on 2026-11-15 from the live services unless the line says otherwise. No keys or
personal data are involved. Files named `whether-*` were recorded by the whether project and
copied from its repository at revision 34bb5c0; the date given is the date of the recording
or of the whether commit that added the file.

- `open-meteo-meta-ecmwf_ifs025-2026-11-15.json`: `https://api.open-meteo.com/data/ecmwf_ifs025/static/meta.json`, 2026-11-15.
- `open-meteo-meta-ncep_gfs025-2026-11-15.json`: `https://api.open-meteo.com/data/ncep_gfs025/static/meta.json`, 2026-11-15.
- `open-meteo-meta-cmc_gem_gdps_15km-2026-11-15.json`: `https://api.open-meteo.com/data/cmc_gem_gdps_15km/static/meta.json`, 2026-11-15.
- `open-meteo-meta-dwd_icon-2026-11-15.json`: `https://api.open-meteo.com/data/dwd_icon/static/meta.json`, 2026-11-15.
- `nhc-current-storms-2026-11-15.json`: `https://www.nhc.noaa.gov/CurrentStorms.json`, 2026-11-15; `{"activeStorms": []}` (no active storm, so no a-deck or b-deck was recorded).
- `whether-open-meteo-storm-point-10001.json`: the four-model point request recorded by whether for New York ZIP 10001 (grid point 40.75,-74.0), about 22:50 UTC on 2026-11-14. Same query shape as the Battery URL in `docs/providers/storms.md`, other coordinates.
- `whether-open-meteo-storm-grid-10001.json`: the pressure grid request recorded by whether with the point request above: `pressure_msl` every 6 hours (40 steps) for the four models at the 49 points of the 7×7, 2.5° grid around 40.75,-74.0 (`https://api.open-meteo.com/v1/forecast?latitude=…&longitude=…&models=ecmwf_ifs025,gfs_seamless,gem_seamless,icon_seamless&hourly=pressure_msl&forecast_days=10&temporal_resolution=hourly_6&timeformat=unixtime`, the point lists elided), about 22:50 UTC on 2026-11-14. ICON's last 7 steps are null.
- `whether-open-meteo-storm-point-missing-model.synthetic.json`: hand-made by whether, the point recording cut to 6 hours without GEM's columns.
- `whether-nhc-current-storms-josephine.synthetic.json`: hand-made by whether, an active-storms summary for the sample storm Josephine (`al102026`), valid 2026-10-22.
- `whether-nhc-bal102026.dat`: b-deck of `al102026` (Josephine) as recorded by whether, added to its repository on 2026-11-14.
- `whether-nhc-aal102026.dat.gz`: a-deck of `al102026` (Josephine), gzip as served, 309 KB, as recorded by whether, added on 2026-11-14.

Not recorded: the Battery point request itself (`latitude=40.70&longitude=-74.02`). Open-Meteo
answered HTTP 429 on 2026-11-15 (daily per-IP limit), see `docs/providers/storms.md`.

Open-Meteo files: weather data by Open-Meteo.com (<https://open-meteo.com/>), licensed CC BY 4.0 (<https://open-meteo.com/en/licence>).
