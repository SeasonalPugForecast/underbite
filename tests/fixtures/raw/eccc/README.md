# Raw fixtures: ECCC hydrometric (api.weather.gc.ca)

One line per file: file name, exact request URL (no keys; the service needs none), recording date.
All recorded 2026-11-17 with `--compressed` (the stored bytes are the decoded body). Base is
`https://api.weather.gc.ca/collections`.

- `queryables-hydrometric-realtime.json`: `/hydrometric-realtime/queryables?f=json`.
- `queryables-hydrometric-stations.json`: `/hydrometric-stations/queryables?f=json`.
- `queryables-hydrometric-daily-mean.json`: `/hydrometric-daily-mean/queryables?f=json`.
- `schema-hydrometric-realtime.json`: `/hydrometric-realtime/schema?f=json`.
- `stations-05BH004.json`: `/hydrometric-stations/items?STATION_NUMBER=05BH004&f=json`.
- `stations-bbox-calgary-limit10.json`: `/hydrometric-stations/items?bbox=-114.2,50.9,-113.9,51.2&f=json&limit=10&sortby=STATION_NUMBER` (10 of 12 matched).
- `stations-bbox-calgary-limit3-offset3.json`: `/hydrometric-stations/items?bbox=-114.2,50.9,-113.9,51.2&f=json&limit=3&offset=3&sortby=STATION_NUMBER`.
- `stations-hits-all.json`: `/hydrometric-stations/items?f=json&limit=1&resulttype=hits` (count only: 8,057).
- `realtime-05BH004-limit12.json`: `/hydrometric-realtime/items?STATION_NUMBER=05BH004&f=json&limit=12&sortby=-DATETIME`.
- `realtime-05BH004-hits.json`: `/hydrometric-realtime/items?STATION_NUMBER=05BH004&f=json&limit=1&resulttype=hits` (count only: 8,666).
- `daily-mean-05BH004-latest5.json`: `/hydrometric-daily-mean/items?STATION_NUMBER=05BH004&f=json&limit=5&sortby=-DATE`.
- `daily-mean-05BH004-2025-11-10-to-16.json`: `/hydrometric-daily-mean/items?STATION_NUMBER=05BH004&f=json&datetime=2025-11-10/2025-11-16&sortby=DATE`.
- `daily-mean-05BH004-hits.json`: `/hydrometric-daily-mean/items?STATION_NUMBER=05BH004&f=json&limit=1&resulttype=hits` (count only: 41,305).
- `daily-mean-05BH004-2023-2025-compact.json`: `/hydrometric-daily-mean/items?STATION_NUMBER=05BH004&f=json&datetime=2023-01-01/2025-12-31&sortby=DATE&limit=1100&skipGeometry=true&properties=DATE,LEVEL,DISCHARGE,DISCHARGE_SYMBOL_EN,LEVEL_SYMBOL_EN` (1,096 days).
- `error-400-limit.json`: HTTP 400 body for `/hydrometric-daily-mean/items?STATION_NUMBER=05BH004&f=json&limit=abc`.

## Licence

Contains information licenced under the Data Server End-use Licence of Environment and Climate Change Canada (<https://eccc-msc.github.io/open-data/licence/readme_en/>).
