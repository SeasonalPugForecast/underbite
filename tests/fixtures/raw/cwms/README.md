# Raw fixtures: USACE CWMS Data API

One line per file: name, exact request URL (no keys were used), recording date. Every request carried `Accept: application/json;version=2` and `User-Agent: underbite-scout/0.1`. All recorded 2026-11-17 unless stated. Files keep the source's format. Older fixture: `../tier2/cwms-nap-beltzville-elev-2026-11-08.json`.

- `cwms-offices-2026-11-17.json`: `https://cwms-data.usace.army.mil/cwms-data/offices`
- `cwms-locations-nab-projects-2026-11-17.json`: `https://cwms-data.usace.army.mil/cwms-data/catalog/LOCATIONS?office=NAB&location-kind-like=PROJECT&page-size=100`
- `cwms-catalog-nab-raystown-2026-11-17.json`: `https://cwms-data.usace.army.mil/cwms-data/catalog/TIMESERIES?office=NAB&like=Raystown%5C..*&page-size=100`
- `cwms-nab-raystown-location-2026-11-17.json`: `https://cwms-data.usace.army.mil/cwms-data/locations/Raystown?office=NAB`
- `cwms-nab-raystown-elev-2026-11-17.json`: `https://cwms-data.usace.army.mil/cwms-data/timeseries?office=NAB&name=Raystown.Elev.Inst.15Minutes.0.Best-NAB&begin=2026-11-17T06:00:00Z&end=2026-11-17T08:00:00Z&unit=SI` (quality 3, datum `OTHER`)
- `cwms-nab-raystown-flow-out-2026-11-17.json`: `https://cwms-data.usace.army.mil/cwms-data/timeseries?office=NAB&name=Raystown.Flow-Out.Inst.15Minutes.0.Best-NAB-Computed-PY3&begin=2026-11-17T06:00:00Z&end=2026-11-17T08:00:00Z&unit=SI`
- `cwms-mvp-baldhill-elev-2026-11-17.json`: `https://cwms-data.usace.army.mil/cwms-data/timeseries?office=MVP&name=Baldhill_Dam.Elev.Inst.15Minutes.0.rev-NGVD29&begin=2026-11-17T06:00:00Z&end=2026-11-17T08:00:00Z&unit=SI` (quality 0, datum NGVD-29)
- `cwms-mvp-baldhill-flow-out-2026-11-17.json`: `https://cwms-data.usace.army.mil/cwms-data/timeseries?office=MVP&name=Baldhill_Dam.Flow-Out.Inst.15Minutes.0.rev&begin=2026-11-17T06:00:00Z&end=2026-11-17T08:00:00Z&unit=SI`
- `cwms-sam-allatoona-elev-2026-11-17.json`: `https://cwms-data.usace.army.mil/cwms-data/timeseries?office=SAM&name=Allatoona.Elev.Inst.1Hour.0.Raw-USGS&begin=2026-11-17T06:00:00Z&end=2026-11-17T08:00:00Z&unit=SI` (time zone `Etc/GMT+6`, quality 0)
- `cwms-nap-beltzville-flow-in-2026-11-08.json`: `https://cwms-data.usace.army.mil/cwms-data/timeseries?office=NAP&name=Beltzville.Flow-In.Inst.1Hour.0.Computed&begin=2026-11-08T12:00:00Z&end=2026-11-08T14:00:00Z&unit=SI` (recorded 2026-11-18 with `User-Agent: underbite/0.1`; inflow in `cms`, quality 0; the same hours as the Beltzville pool fixture)
