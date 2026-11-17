# Copernicus Marine WMTS fixtures

Recorded without an account, 2026-11-17 unless stated. Base `https://wmts.marine.copernicus.eu/teroWmts`. Tiles are the New York bight at zoom 5 (column 9, row 12). Details in `docs/providers/copernicus.md`.

- `wmts-capabilities-glo-phy-cur-anfc-0.083deg-P1D-m.xml`: `https://wmts.marine.copernicus.eu/teroWmts/GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy-cur_anfc_0.083deg_P1D-m_202406?SERVICE=WMTS&REQUEST=GetCapabilities` (2026-11-17)
- `wmts-capabilities-glo-phy-thetao-anfc-0.083deg-P1D-m.xml`: `https://wmts.marine.copernicus.eu/teroWmts/GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy-thetao_anfc_0.083deg_P1D-m_202406?SERVICE=WMTS&REQUEST=GetCapabilities` (2026-11-17)
- `wmts-capabilities-oc-plankton-gapfree-4km-P1D.xml`: `https://wmts.marine.copernicus.eu/teroWmts/OCEANCOLOUR_GLO_BGC_L4_NRT_009_102/cmems_obs-oc_glo_bgc-plankton_nrt_l4-gapfree-multi-4km_P1D_202311?SERVICE=WMTS&REQUEST=GetCapabilities` (2026-11-17)
- `wmts-capabilities-sst-l4-nrt-metoffice-v2.xml`: `https://wmts.marine.copernicus.eu/teroWmts/SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001/METOFFICE-GLO-SST-L4-NRT-OBS-SST-V2?SERVICE=WMTS&REQUEST=GetCapabilities` (2026-11-17)
- `wmts-uo-z5-9-12-2026-11-16.png`: `https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy-cur_anfc_0.083deg_P1D-m_202406/uo&STYLE=cmap:balance&FORMAT=image/png&TILEMATRIXSET=EPSG:3857&TILEMATRIX=5&TILEROW=12&TILECOL=9&TIME=2026-11-16T00:00:00.000Z` (2026-11-17)
- `wmts-sea-water-velocity-solidAndVector-z5-9-12-2026-11-16.png`: same with `LAYER=.../cmems_mod_glo_phy-cur_anfc_0.083deg_P1D-m_202406/sea_water_velocity&STYLE=cmap:speed,vectorStyle:solidAndVector` (2026-11-17)
- `wmts-thetao-z5-9-12-2026-11-16.png`: same with `LAYER=GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy-thetao_anfc_0.083deg_P1D-m_202406/thetao&STYLE=cmap:thermal` (2026-11-17)
- `wmts-chl-oc-z5-9-12-2026-11-15.png`: same with `LAYER=OCEANCOLOUR_GLO_BGC_L4_NRT_009_102/cmems_obs-oc_glo_bgc-plankton_nrt_l4-gapfree-multi-4km_P1D_202311/CHL&STYLE=cmap:algae&TIME=2026-11-15T00:00:00.000Z` (2026-11-17)
- `wmts-analysed-sst-z5-9-12-2026-11-16.png`: same with `LAYER=SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001/METOFFICE-GLO-SST-L4-NRT-OBS-SST-V2/analysed_sst&STYLE=cmap:thermal` (2026-11-17)
- `wmts-legend-thetao.json`: `https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetLegend&LAYER=GLOBAL_ANALYSISFORECAST_PHY_001_024%2Fcmems_mod_glo_phy-thetao_anfc_0.083deg_P1D-m_202406%2Fthetao&STYLE=cmap%3Athermal&FORMAT=application%2Fjson` (2026-11-17)
- `wmts-error-time-out-of-range.xml`: the OC chlorophyll tile request above with `TIME=2026-11-16T00:00:00.000Z`, one day past the dataset's last time; HTTP 400 (2026-11-17)

The existing Copernicus fixtures from the earlier survey are in `../tier2/README.md`.
