# Raw fixtures: DFO IWLS (synthetic)

Every file here is hand-made (`*.synthetic.json`) and is not a recording. The service's licence does
not allow its data to be given to third parties, so these files only copy the JSON shape and field
names described in `docs/providers/dfo-iwls.md`. Station codes, ids, names, coordinates, heights and
times are invented; the ids are arbitrary 24-character hex strings. Request URLs in the lines below
show the shape of the call the file stands for, with the invented station.

Invented stations: Pinegate Harbour 90101 = `5865a794e8e13973715cf951` (tidal, Pacific),
Narrows Point 90102 (tidal currents), Lakeshore Gauge 90201 = `a66cb57182b4d901acc06645` (lake,
`IGLD85` offset), Gull Bay 90301, and four non-operating stations.

- `stations-all.synthetic.json`: shape of `/api/v1/stations` (eight stations: four operating, four not).
- `stations-single.synthetic.json`: shape of `/api/v1/stations?code=90101` (one station, three series).
- `metadata-90101.synthetic.json`, `metadata-90201-lake.synthetic.json`, `metadata-90102-narrows.synthetic.json`: shape of `/api/v1/stations/{id}/metadata` (a tidal port with `CGVD` datums, a lake gauge with an `IGLD85` offset, a current station with `ebbDirection` and `floodDirection`).
- `wlp-90101-15min.synthetic.json`: `time-series-code=wlp&resolution=FIFTEEN_MINUTES`, one UTC day (97 rows, quality `"2"`).
- `wlp-90101-1min.synthetic.json`: `time-series-code=wlp`, one hour at the default one-minute resolution (61 rows).
- `wlo-90101-15min.synthetic.json`: `time-series-code=wlo&resolution=FIFTEEN_MINUTES`, one hour (5 rows, quality `"1"`).
- `wlo-90101-all.synthetic.json`: `time-series-code=wlo&resolution=ALL`, ten minutes of one-minute rows.
- `wlo-90201-60min.synthetic.json`: `wlo` at `SIXTY_MINUTES` for the lake gauge (2 rows).
- `wlp-hilo-90101-2026-11-08.synthetic.json`: `time-series-code=wlp-hilo`, five extrema from a low at 06:30 UTC on 2026-11-08.
- `wlp-hilo-90101-2026-11-18-to-20.synthetic.json`: `wlp-hilo`, six extrema over 2026-11-18 to 20.
- `wcsp1-90102-15min.synthetic.json`, `wcs1-90102-15min.synthetic.json`, `wcp1-events-90102-2026-11-18.synthetic.json`: predicted speed, observed speed and slack/extrema events (`qualifier`) for the current station.
- `time-series-definitions.synthetic.json`, `phenomena.synthetic.json`, `height-types.synthetic.json`: the lookup lists, reduced to a few entries.
- `error-400-range-7-days.synthetic.json`, `error-400-range-31-days.synthetic.json`, `error-400-bad-date.synthetic.json`: the 400 body shape (`timestamp`, `status`, `message`, `path`, `errors`).

The 429 body is not covered (see `docs/providers/dfo-iwls.md`).
