# Raw fixtures: NASA GIBS

All recorded on 2026-11-08 with `User-Agent: underbite-scout/0.1 (provider docs check)`. The three tiles are Web Mercator, 256 x 256, zoom 7, column 37, row 48 (longitude -75.94 to -73.13, latitude 38.82 to 40.98: Delaware Bay to the Hudson mouth and the New Jersey shore).

- `viirs-noaa20-truecolor-2026-11-07-z7-37-48.jpg`: `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_NOAA20_CorrectedReflectance_TrueColor/default/2026-11-07/GoogleMapsCompatible_Level9/7/48/37.jpg`
- `ghrsst-mur-sst-2026-11-07-z7-37-48.png`: `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/GHRSST_L4_MUR_Sea_Surface_Temperature/default/2026-11-07/GoogleMapsCompatible_Level7/7/48/37.png`
- `viirs-noaa20-chlorophyll-2026-11-07-z7-37-48.png`: `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_NOAA20_Chlorophyll_a/default/2026-11-07/GoogleMapsCompatible_Level7/7/48/37.png`
- `ghrsst-sea-surface-temperature-colormap-v1.3.xml`: `https://gibs.earthdata.nasa.gov/colormaps/v1.3/GHRSST_Sea_Surface_Temperature.xml`
- `wmts-epsg3857-best-capabilities.excerpt.xml`: an excerpt of `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/1.0.0/WMTSCapabilities.xml` (the full document is 5.8 MB). Kept: the service header, five `Layer` elements (VIIRS_NOAA20_CorrectedReflectance_TrueColor, MODIS_Terra_CorrectedReflectance_TrueColor, GHRSST_L4_MUR_Sea_Surface_Temperature, VIIRS_NOAA20_Chlorophyll_a, OCI_PACE_Chlorophyll_a) and the TileMatrixSets `GoogleMapsCompatible_Level7` and `_Level9`. Cut with a text script; every kept element is unmodified. The `<Dimension>` time ranges are those of 2026-11-08.

## Recorded on 2026-11-16

- `wmts-epsg3857-best-capabilities.excerpt-2026-11-16.xml`: an excerpt of `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/1.0.0/WMTSCapabilities.xml` (5.8 MB, 1,330 layers that day). Kept: the service header and eleven unmodified `Layer` elements (VIIRS NOAA-20, NOAA-21 and S-NPP true colour, MODIS Aqua and Terra true colour, GHRSST MUR SST, VIIRS NOAA-20 and NOAA-21 chlorophyll, PACE OCI chlorophyll, Sentinel-3A OLCI chlorophyll, VIIRS S-NPP L2 SST day). No TileMatrixSet elements. The `<Dimension>` ranges are those of 2026-11-16.
- `viirs-noaa21-chlorophyll-2026-11-15-z7-37-48.png`: `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_NOAA21_Chlorophyll_a/default/2026-11-15/GoogleMapsCompatible_Level7/7/48/37.png` (256 x 256; the response said `layer-identifier-actual: VIIRS_NOAA21_Chlorophyll_a_v2022.0_NRT`, `layer-time-actual: 2026-11-15T00:00:00Z`).
