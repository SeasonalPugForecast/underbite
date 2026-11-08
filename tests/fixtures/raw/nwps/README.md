# NWPS fixtures

Recorded 2026-11-08 with `curl`; the service needs no key. Response bodies only (the service also sets
load-balancer cookies, which are not recorded). Base URL: `https://api.water.noaa.gov/nwps/v1`.
See `docs/providers/nwps.md` for field tables.

- `nwps-gauges-bbox-lower-hudson.json` — `https://api.water.noaa.gov/nwps/v1/gauges?bbox.xmin=-74.4&bbox.ymin=40.9&bbox.xmax=-73.6&bbox.ymax=41.6&srid=EPSG_4326` (2026-11-08; 32 gauges)
- `nwps-gauge-AUON6.json` — `https://api.water.noaa.gov/nwps/v1/gauges/AUON6` (2026-11-08; Owasco Outlet at Auburn NY, USGS 04235440, stage and flow forecast)
- `nwps-gauge-AUON6-stageflow-forecast.json` — `https://api.water.noaa.gov/nwps/v1/gauges/AUON6/stageflow/forecast` (2026-11-08)
- `nwps-gauge-AUON6-stageflow-observed-trimmed.json` — `https://api.water.noaa.gov/nwps/v1/gauges/AUON6/stageflow/observed` (2026-11-08; the only edit is that `data` was cut from 2823 entries to the last 672, 2026-11-01T14:15Z to 2026-11-08T14:00Z, to keep the file small; every other field is as served)
- `nwps-gauge-COHN6.json` — `https://api.water.noaa.gov/nwps/v1/gauges/COHN6` (2026-11-08; Mohawk River at Cohoes NY, USGS 01357500, flood categories and 59 recent crests)
- `nwps-gauge-COHN6-stageflow-forecast.json` — `https://api.water.noaa.gov/nwps/v1/gauges/COHN6/stageflow/forecast` (2026-11-08; a forecast that changes over time)
- `nwps-gauge-ALBN6.json` — `https://api.water.noaa.gov/nwps/v1/gauges/ALBN6` (2026-11-08; Hudson River at Albany NY, USGS 01359139, stage forecast, flow `-999`/`-9999` sentinels, tidal and ice notes)
- `nwps-gauge-BATN6.json` — `https://api.water.noaa.gov/nwps/v1/gauges/BATN6` (2026-11-08; New York Harbor at The Battery, stage in feet MLLW, no USGS id)
- `nwps-gauge-KTNN6.json` — `https://api.water.noaa.gov/nwps/v1/gauges/KTNN6` (2026-11-08; Cross River at Katonah NY, USGS 01374901, observed only, no forecast, undefined flood categories)
- `nwps-reach-6228280-streamflow-analysis.json` — `https://api.water.noaa.gov/nwps/v1/reaches/6228280/streamflow` (2026-11-08; National Water Model reach for KTNN6, default series `analysis_assimilation`)
- `nwps-reach-6228280-streamflow-short-range.json` — `https://api.water.noaa.gov/nwps/v1/reaches/6228280/streamflow?series=short_range` (2026-11-08)
