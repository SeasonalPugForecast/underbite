# OpenStreetMap Overpass API

Verified on 2026-11-17 (UTC) with `curl` against `overpass-api.de` and the OSM wiki page "Overpass API" (policy, instance list). An earlier survey is in `docs/providers/tier2.md`. Eight requests were sent to the main instance in about 15 minutes, one at a time, 15 to 45 s apart, with 30 s or more after each failure. Fixtures: `tests/fixtures/raw/overpass/` (index in its `README.md`).

## Endpoint

| Item | Value |
|---|---|
| Main instance | `https://overpass-api.de/api/interpreter` (operated by FOSSGIS) |
| Method | GET with `data=<query>` or POST with form field `data` (all requests here were POST via `curl --data-urlencode`) |
| Authentication | none |
| Header | `User-Agent: underbite-scout/0.1`; the wiki asks every client to send a `User-Agent` or `Referer` that identifies the app |
| Status | `GET /api/status` (answered in a second): `Connected as: <number>`, `Current time: 2026-11-17T10:17:07Z`, `Announced endpoint: gall.openstreetmap.de/`, `Rate limit: 2`, `2 slots available now.` |
| Data age | `osm3s.timestamp_osm_base` was within minutes of the request time (`2026-11-17T10:16:09Z` at 10:17) |

## Usage policy (OSM wiki, read 2026-11-17)

- Under about 10,000 queries and 1 GB per day does not disturb other users; total processing time within about 10 minutes per day; "regular users should divide these numbers by 100".
- An app counts all its users' requests toward that total. No parallel scripts. Cache and rate-limit.
- After HTTP 429 or 406, "pause for 30 seconds before making a new request". Other error codes can also mean a block.
- Commercial use should go to a self-hosted or paid server. The wiki warns that the main instance is overloaded and not to expect high reliability.
- Default query `[timeout]` is 180 s and `[maxsize]` 512 MiB; set both for each query.

## Observed behaviour

- Per-user slot limit: `Rate limit: 2`.
- Successful queries took **25 to 35 s** of wall time (32.5 s for a 12 KB result over a 6 km x 6 km box with three tag groups, 35.6 s for a 1 KB result over a 10 km x 12 km box, 24.9 s for 1.8 KB): the request queues behind other users even when the query itself is cheap.
- 5 of 8 interpreter requests got HTTP **502** after 0.3 s with a plain-text (markdown) body, not JSON: "This Overpass API server is currently at capacity. All query workers are busy and the request queue is full." with the advice to retry with exponential backoff, and the line "This public endpoint is intended for browser-based and human-operated tools. For automated or high-volume use, run a local Overpass API instance." The same query succeeded on a later attempt. No 429 or 406 occurred. A client must treat 502, 504 and 429 as retryable with backoff (start at 30 s) and check that a 200 body is JSON.
- The query form `node[...]->.r; nwr(around.r:300)[...]` failed with 502 three times, but two plain bounding-box queries also failed with 502 before succeeding on a later attempt, so the failures follow server load and not the query form. Whether `around` is accepted when the server is not at capacity was not established (**unverified**).
- The JSON body is pretty-printed; no rate-limit headers are sent.

## Queries the app needs

All use `[out:json][timeout:25][maxsize:2000000]` and ask for `out center tags N` (the limit `N` after `tags` caps the element count; `center` gives a point for ways and relations; `meta` is not requested, so no user names or ids appear). Bounding boxes are `(south,west,north,east)`.

Boat ramps, fishing places and piers near The Battery (fixture `overpass-access-battery-2026-11-17.json`, 12 KB, 60 elements, truncated by the `out` limit):

```text
[out:json][timeout:25][maxsize:2000000];
(nwr["leisure"="slipway"](40.68,-74.05,40.74,-73.99);
 nwr["leisure"="fishing"](40.68,-74.05,40.74,-73.99);
 nwr["man_made"="pier"](40.68,-74.05,40.74,-73.99););
out center tags 60;
```

Fishing places and ramps near Croton Point (fixture `overpass-fishing-slipway-croton-2026-11-17.json`, 3 elements, all `leisure=slipway`; no `leisure=fishing` there):

```text
[out:json][timeout:25][maxsize:2000000];
(nwr["leisure"="fishing"](41.17,-73.95,41.26,-73.82);
 nwr["leisure"="slipway"](41.17,-73.95,41.26,-73.82););
out center tags 20;
```

Parking in a small box (fixture `overpass-parking-croton-2026-11-17.json`, 10 ways, 1.8 KB):

```text
[out:json][timeout:25][maxsize:2000000];
nwr["amenity"="parking"](41.22,-73.94,41.24,-73.92);
out center tags 10;
```

Proximity to a ramp can be computed in the app from the returned coordinates; the `around` form was not confirmed to work.

## Tags seen

| Tag | Values and meaning |
|---|---|
| `leisure=slipway` | boat ramp; tags `access` (`yes`), `fee`, `surface` (`concrete`, `paving_stones`, `ground`), `canoe`, `trailer`, `parking`, `opening_hours` (`Apr-Sep 08:00-dusk`), `operator`, `website`, `name` |
| `leisure=fishing` | fishing place (nodes and ways); no other tags in the Battery sample |
| `man_made=pier` | ways; `name`, `floating`, `mooring`, `surface`, `operator`; 53 of 60 elements in the Battery sample, most unnamed or ferry and private; there is no tag for "fishing allowed" on a pier (the sample has `fishing` on none), so fishing from a pier is not inferable |
| `amenity=parking` | ways; `parking=surface`, `surface`; most carry no other tags |

Element JSON: `type` (`node`, `way`, `relation`), `id`, `lat` and `lon` for nodes or `center.lat` and `center.lon`, `tags`. OSM tags are free-form: `access` may be missing, `fee` is `yes` or `no`, `opening_hours` uses the OSM syntax.

## Other public instances (wiki, 2026-11-17)

| Instance | Policy stated on the wiki | Checked here |
|---|---|---|
| `https://overpass.private.coffee/api/interpreter` | "No rate limit. Notify the operator in advance for large-scale projects." | `/api/status` and an interpreter query both timed out (30 s and 60 s) on 2026-11-17: **unavailable from this network at the time** |
| `https://maps.mail.ru/osm/tools/overpass/api/interpreter` (VK Maps) | no request limits stated | not tried (operated from Russia; not suitable for a US app) |
| Overpass North America at `dresel.systems` | free, no key; at most 4 parallel queries per IP and up to 16 GB per query; send a User-Agent or Referer | not tried |
| `overpass.ch` (`api.overpass.ch/v1/<key>/interpreter`) | free tier 10,000 requests per month and 30 per minute; key required | not tried |
| NextGIS, Geofabrik, Tracestrack, Overspan, Mapsource, FairwayMapper | keys or payment | not tried |

A shipped app needs its own caching layer or a paid or self-hosted instance for any volume: one phone's use is small, but all users share the single main-instance quota and the 25 to 35 s latency observed here is long for a card on a spot screen.

## License and attribution

ODbL 1.0. Every response carries `osm3s.copyright`: "The data included in this document is from www.openstreetmap.org. The data is made available under ODbL." Credit "(c) OpenStreetMap contributors" with a link to `https://www.openstreetmap.org/copyright`. A database derived from OSM data and redistributed (for example a bundled catalogue of ramps) must be offered under ODbL.

Re-checked 2026-11-18: licence unchanged; one usage statement is new to this note. `https://www.openstreetmap.org/copyright` still requires credit to OpenStreetMap and a clear statement that the data is available under the ODbL (a link to the copyright page is enough). The OSM wiki usage text (`https://wiki.openstreetmap.org/wiki/Overpass_API`) is unchanged from the "Usage policy" section below. The Overpass user manual's Commons page (`https://dev.overpass-api.de/overpass-doc/en/preface/commons.html`), read the same day, lists among problematic behaviour "Setting up an app for more than just OSM mappers and relying on the public instances as backend" and says "only running your own instance sustainably serves your mission" in that case; it states each public server can fulfil about 1 million requests a day for roughly 30,000 daily users and gives a guideline of about 10,000 requests and 1 GB per day per user.

## Unverified

- The dresel.systems and overpass.ch instances (not requested).
- Whether the `around` form works when the server has free capacity.
- Behaviour above the slot limit (HTTP 429), not provoked on purpose.
