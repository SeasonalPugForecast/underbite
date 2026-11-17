# Raw fixtures: USBR RISE

One line per file: name, exact request URL (no keys were used), recording date. Requests carried `User-Agent: underbite-scout/0.1`. All recorded 2026-11-17. Files keep the source's format (JSON:API). Older fixture: `../tier2/rise-fresno-reservoir-elevation-2026-11.json`.

- `rise-location-1535-flaming-gorge-2026-11-17.json`: `https://data.usbr.gov/rise/api/location/1535`
- `rise-locations-lake-reservoir-page1-5-2026-11-17.json`: `https://data.usbr.gov/rise/api/location?locationTypeId=3&itemsPerPage=5` (first five of 286 reservoir locations)
- `rise-catalog-items-flaming-gorge-2026-11-17.json`: `https://data.usbr.gov/rise/api/catalog-item?id%5B%5D=341&id%5B%5D=4314&id%5B%5D=339&id%5B%5D=337&id%5B%5D=4306&itemsPerPage=100`
- `rise-result-341-flaming-gorge-elevation-2026-11-17.json`: `https://data.usbr.gov/rise/api/result?itemId=341&dateTime%5Bafter%5D=2026-11-10&itemsPerPage=10&order%5BdateTime%5D=DESC`
- `rise-result-4314-flaming-gorge-release-total-cfs-2026-11-17.json`: `https://data.usbr.gov/rise/api/result?itemId=4314&dateTime%5Bafter%5D=2026-11-10&itemsPerPage=10&order%5BdateTime%5D=DESC`
- `rise-result-10863-shasta-elevation-2026-11-17.json`: `https://data.usbr.gov/rise/api/result?itemId=10863&dateTime%5Bafter%5D=2026-11-10&itemsPerPage=10&order%5BdateTime%5D=DESC`
- `rise-result-no-filter-error-2026-11-17.json`: `https://data.usbr.gov/rise/api/result?itemsPerPage=1` (HTTP 400, `application/problem+json`)
