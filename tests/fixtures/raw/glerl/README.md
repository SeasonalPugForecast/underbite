# Raw fixtures: NOAA GLERL ERDDAP (GLSEA)

All recorded on 2026-11-16 with `User-Agent: underbite-scout/0.1 (provider docs check)`. Responses are byte-for-byte what the server returned.

- `glsea-acspo-gcs.das`: `https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.das`
- `glsea-acspo-gcs-sst-lake-erie-west-2026-11-15.json`: `https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.json?sst%5B(last)%5D%5B(41.3):2:(42.1)%5D%5B(-83.6):2:(-82.6)%5D` (`(last)` resolved to 2026-11-15T12:00:00Z; stride 2 = 0.028 degrees; 1,073 rows, 428 with a value; latitude ascending).
- `glsea-acspo-gcs-time-2026-11-05-to-last.json`: `https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.json?time%5B(2026-11-05):1:(last)%5D` (eleven consecutive days at 12:00:00Z, the last 2026-11-15).
