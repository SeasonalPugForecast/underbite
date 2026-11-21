# Raw fixtures: gridded wind, pressure and wave candidates

All recorded on 2026-11-17 (UTC) with `User-Agent: Underbite-scout/0.1 (data source check)`. Boxes are New York Bight only. The PacIOOS `_lon180` datasets use longitudes from -180 to 180.

- `pacioos-ncep-global-lon180-wind-pressure-2026-11-17T12.json`: `https://pae-paha.pacioos.hawaii.edu/erddap/griddap/ncep_global_lon180.json?ugrd10m[(2026-11-17T12:00:00Z)][(39.0):(41.5)][(-75.0):(-72.5)],vgrd10m[(2026-11-17T12:00:00Z)][(39.0):(41.5)][(-75.0):(-72.5)],prmslmsl[(2026-11-17T12:00:00Z)][(39.0):(41.5)][(-75.0):(-72.5)]` (brackets sent percent-encoded). GFS 0.5 degree; wind in m s-1, pressure in Pa; latitude rows run north to south.
- `pacioos-ww3-global-lon180-waves-2026-11-17T12.json`: `https://pae-paha.pacioos.hawaii.edu/erddap/griddap/ww3_global_lon180.json?Thgt[(2026-11-17T12:00:00Z)][(0.0)][(39.0):(41.5)][(-75.0):(-72.5)]`, and the same constraint on `Tdir` and `Tper` (brackets percent-encoded). WaveWatch III 0.5 degree; metres, degrees (direction the waves come from), seconds; `null` on land and unresolved cells; latitude rows run south to north.
- `open-meteo-forecast-grid-3x3-2026-11-17.json`: `https://api.open-meteo.com/v1/forecast?latitude=39.75,39.75,39.75,40.25,40.25,40.25,40.75,40.75,40.75&longitude=-73.75,-73.25,-72.75,-73.75,-73.25,-72.75,-73.75,-73.25,-72.75&hourly=wind_speed_10m,wind_direction_10m,pressure_msl&wind_speed_unit=ms&forecast_hours=2&timezone=GMT`
- `open-meteo-marine-grid-3x3-2026-11-17.json`: `https://marine-api.open-meteo.com/v1/marine?` with the same nine `latitude` and `longitude` values, `&hourly=wave_height,wave_direction,wave_period&forecast_hours=2&timezone=GMT`
- `copernicus-wmts-wav-vhm0-z5-9-12-2026-11-17T12.png`: `https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=GLOBAL_ANALYSISFORECAST_WAV_001_027/cmems_mod_glo_wav_anfc_0.083deg_PT3H-i_202411/VHM0&STYLE=cmap:amp&FORMAT=image/png&TILEMATRIXSET=EPSG:3857&TILEMATRIX=5&TILEROW=12&TILECOL=9&TIME=2026-11-17T12:00:00.000Z`
- `copernicus-wmts-wav-vhm0-legend-amp.json`: `https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetLegend&LAYER=GLOBAL_ANALYSISFORECAST_WAV_001_027%2Fcmems_mod_glo_wav_anfc_0.083deg_PT3H-i_202411%2FVHM0&STYLE=cmap%3Aamp&FORMAT=application%2Fjson`
- `copernicus-wmts-wind-solid-z5-9-12-2026-11-15T12.png`: `https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=WIND_GLO_PHY_L4_NRT_012_004/cmems_obs-wind_glo_phy_nrt_l4_0.125deg_PT1H_202207/wind&STYLE=cmap:speed,vectorStyle:solid&FORMAT=image/png&TILEMATRIXSET=EPSG:3857&TILEMATRIX=5&TILEROW=12&TILECOL=9&TIME=2026-11-15T12:00:00.000Z`
- `copernicus-wmts-wind-vector-z5-9-12-2026-11-15T12.png`: the same with `STYLE=cmap:speed,vectorStyle:vector`.

## Licences and credits

- `open-meteo-*.json`: Weather data by Open-Meteo.com, <https://open-meteo.com/>, licensed CC BY 4.0 (<https://open-meteo.com/en/licence>).
- `copernicus-wmts-*`: E.U. Copernicus Marine Service Information; product DOIs and credit text are in `docs/providers/copernicus.md`.
- `pacioos-*.json`: Pacific Islands Ocean Observing System (PacIOOS) datasets, free to use and redistribute under the terms in their `license` attribute, summarised in `docs/providers/gridded-weather.md`.
