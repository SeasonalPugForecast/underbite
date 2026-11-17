# Raw fixtures: OpenStreetMap Overpass

Recorded on 2026-11-17. No key or account is involved. See docs/providers/overpass.md.
Endpoint: `POST https://overpass-api.de/api/interpreter` with form field `data` (the query text, all with `[out:json][timeout:25][maxsize:2000000]`). No `meta`, so no user names.

- `overpass-access-battery-2026-11-17.json`: `nwr` of `leisure=slipway`, `leisure=fishing` and `man_made=pier` in `(40.68,-74.05,40.74,-73.99)`, `out center tags 60`, 2026-11-17.
- `overpass-fishing-slipway-croton-2026-11-17.json`: `nwr` of `leisure=fishing` and `leisure=slipway` in `(41.17,-73.95,41.26,-73.82)`, `out center tags 20`, 2026-11-17.
- `overpass-parking-croton-2026-11-17.json`: `nwr["amenity"="parking"](41.22,-73.94,41.24,-73.92)`, `out center tags 10`, 2026-11-17.
