# Raw fixtures: DFO IWLS

One line per file: file name, exact request URL (no keys; the service needs none), recording date.
Base is `https://api-iwls.dfo-mpo.gc.ca`. Every file was recorded 2026-11-17 and keeps the response
bytes. Station ids: Vancouver 07735 = `5cebf1de3d0f4a073c4bb943`, Toronto 13320 =
`5cebf1e43d0f4a073c4bc3e5`, Second Narrows 07745 = `5dd30650e0fdc4b9b4be6c2d`.

- `stations-all-2026-11-17.json`: `/api/v1/stations` (all 1,576 stations, 918,660 bytes), 2026-11-17.
- `metadata-07735.json`: `/api/v1/stations/5cebf1de3d0f4a073c4bb943/metadata`, 2026-11-17.
- `metadata-13320-toronto.json`: `/api/v1/stations/5cebf1e43d0f4a073c4bc3e5/metadata`, 2026-11-17.
- `metadata-07745-second-narrows.json`: `/api/v1/stations/5dd30650e0fdc4b9b4be6c2d/metadata`, 2026-11-17.
- `wlp-07735-15min-2026-11-18.json`: `/api/v1/stations/5cebf1de3d0f4a073c4bb943/data?time-series-code=wlp&from=2026-11-18T00:00:00Z&to=2026-11-19T00:00:00Z&resolution=FIFTEEN_MINUTES`, 2026-11-17.
- `wlp-07735-1min-2026-11-18-0000-0100.json`: same path, `time-series-code=wlp&from=2026-11-18T00:00:00Z&to=2026-11-18T01:00:00Z` (no `resolution`, so the default `ONE_MINUTE`), 2026-11-17.
- `wlo-07735-15min-2026-11-17-0800-0900.json`: same path, `time-series-code=wlo&from=2026-11-17T08:00:00Z&to=2026-11-17T09:00:00Z&resolution=FIFTEEN_MINUTES`, 2026-11-17.
- `wlo-07735-all-2026-11-17-0800-0810.json`: same path, `time-series-code=wlo&from=2026-11-17T08:00:00Z&to=2026-11-17T08:10:00Z&resolution=ALL`, 2026-11-17.
- `wlp-hilo-07735-2026-11-18-to-20.json`: same path, `time-series-code=wlp-hilo&from=2026-11-18T00:00:00Z&to=2026-11-20T00:00:00Z`, 2026-11-17.
- `wlo-13320-toronto-60min-2026-11-17-0800-0900.json`: `/api/v1/stations/5cebf1e43d0f4a073c4bc3e5/data?time-series-code=wlo&from=2026-11-17T08:00:00Z&to=2026-11-17T09:00:00Z&resolution=SIXTY_MINUTES`, 2026-11-17.
- `wcsp1-07745-15min-2026-11-18-0000-0100.json`: `/api/v1/stations/5dd30650e0fdc4b9b4be6c2d/data?time-series-code=wcsp1&from=2026-11-18T00:00:00Z&to=2026-11-18T01:00:00Z&resolution=FIFTEEN_MINUTES`, 2026-11-17.
- `wcp1-events-07745-2026-11-18.json`: same path, `time-series-code=wcp1-events&from=2026-11-18T00:00:00Z&to=2026-11-19T00:00:00Z`, 2026-11-17.
- `wcs1-07745-15min-2026-11-17-0800-0900.json`: same path, `time-series-code=wcs1&from=2026-11-17T08:00:00Z&to=2026-11-17T09:00:00Z&resolution=FIFTEEN_MINUTES`, 2026-11-17.
- `time-series-definitions.json`: `/api/v1/time-series-definitions`, 2026-11-17.
- `phenomena.json`: `/api/v1/phenomena`, 2026-11-17.
- `height-types.json`: `/api/v1/height-types`, 2026-11-17.
- `openapi-v3-api-docs.json`: `https://api-iwls.dfo-mpo.gc.ca/v3/api-docs` (OpenAPI 3.1, API version 1.3.1), 2026-11-17.
- `error-400-range-7-days.json`: HTTP 400 body for `.../data?time-series-code=wlp&from=2026-11-01T00:00:00Z&to=2026-11-30T00:00:00Z&resolution=ONE_MINUTE` on station 07735, 2026-11-17.
- `error-400-range-31-days.json`: HTTP 400 body for `.../data?time-series-code=wlp&from=2026-11-18T00:00:00Z&to=2026-12-20T00:00:00Z&resolution=SIXTY_MINUTES` on station 07735, 2026-11-17.
- `error-400-bad-date.json`: HTTP 400 body for `.../data?time-series-code=wlp&from=garbage&to=2026-12-20T00:00:00Z` on station 07735, 2026-11-17.

The 429 body was not recorded (see `docs/providers/dfo-iwls.md`).
