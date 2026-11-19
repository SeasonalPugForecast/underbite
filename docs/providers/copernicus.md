# Copernicus Marine WMTS

Verified on 2026-11-17 (UTC) with `curl` against `https://wmts.marine.copernicus.eu/teroWmts`, with no account, key or cookie. Fixtures: `tests/fixtures/raw/copernicus/` (index in its `README.md`). An earlier survey (2026-11-08) is in `docs/providers/tier2.md`. Subsetting (the Copernicus Marine Toolbox and its data API) needs an account and was not tested: **unverified**.

## Service

- One endpoint for every dataset: `https://wmts.marine.copernicus.eu/teroWmts`, OGC WMTS 1.0.0, **key-value (KVP) encoding only**. Operations: `GetCapabilities`, `GetTile`, `GetFeatureInfo` and (not in the OWS operation list) `GetLegend`.
- Tile matrix sets in the capabilities: `EPSG:3857`, `EPSG:3857@2x`, `EPSG:3857@3x`, `EPSG:4326` and its `@2x`/`@3x` variants. `EPSG:3857` gives 256 x 256 RGBA PNG tiles; `TILEMATRIX` is the plain zoom number, `TILEROW` is y from the north, `TILECOL` is x (the usual XYZ tile, checked at zoom 5 and 7 near New York).
- Tile responses: `content-type: image/png`, `cache-control: max-age=2592000` (30 days), `access-control-allow-origin: *`, served through Varnish. No rate-limit headers were seen, and no limit is published; requests here were paced at one per 1.5 s, 12 tiles in all. A client name in `User-Agent` was sent; none is required.
- Errors are OGC `ExceptionReport` XML with HTTP 400 (bad `TIME`, bad `FORMAT`) or 500 (unknown dataset).
- The data are daily means on a grid; the tile is a rendered picture, not values. Values come from `GetFeatureInfo` (JSON, HTML, XML or GML; not exercised) or from account-based subsetting.

## Capabilities: the version suffix

The service-wide document is about 70 MB: never fetch it. Three smaller levels work, all anonymous:

| Level | URL | Size seen |
|---|---|---|
| Product | `.../teroWmts/<PRODUCT>?SERVICE=WMTS&REQUEST=GetCapabilities` | `GLOBAL_ANALYSISFORECAST_PHY_001_024` 379 KB, `..._BGC_001_028` 219 KB, `OCEANCOLOUR_GLO_BGC_L4_NRT_009_102` 552 KB, `SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001` 35 KB |
| Dataset | `.../teroWmts/<PRODUCT>/<DATASET>?SERVICE=WMTS&REQUEST=GetCapabilities` | 26 to 73 KB |
| Layer id | `<PRODUCT>/<DATASET>/<VARIABLE>` | in `GetTile` `LAYER=` |

The dataset id carries the version suffix that changes when the product is reprocessed (`_202406`, `_202311`, `_202411`, `_202611` appear side by side in one product; some datasets have none). A wrong suffix answers HTTP 500 (`Dataset ... not found`; checked with `..._bgc-pft_anfc_0.25deg_P1D-m_202411`, which does not exist, while `_202311` does). To find the current one, read the product capabilities, take every `<ows:Identifier>` of the form `<PRODUCT>/<DATASET>/<VARIABLE>`, strip the last segment, and choose the dataset by its name stem (`cmems_mod_glo_phy-cur_anfc_0.083deg_P1D-m`) taking the highest suffix. Note that product-level lists include monthly (`P1M`), 6-hourly instantaneous (`PT6H-i`) and hourly datasets beside the daily means. The catalogue of the app should keep the stem and refresh the suffix from a product capabilities file at build time, not at run time.

## Layers

Time values are UTC midnights written `YYYY-MM-DDT00:00:00.000Z` in requests. Each layer has a `time` dimension (ISO8601, `start/end/P1D`, with a default), and the physics and BGC layers an `elevation` dimension (metres, negative down; the first value, -0.494 m, is the default and is the surface layer). The app need not send `ELEVATION`; sending nothing returned the surface.

| Use | Product / dataset id (2026-11-17) | Layer | Styles (default first) | Unit | `time` range at check | Kind |
|---|---|---|---|---|---|---|
| Surface current, east | `GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy-cur_anfc_0.083deg_P1D-m_202406` | `uo` | `cmap:balance` | m s-1 (colour range -1.035 to 1.035) | 2022-06-01 to 2026-11-26 (default 2026-11-17) | analysis and 10-day forecast |
| Surface current, north | same | `vo` | `cmap:balance` | m s-1 | same | same |
| Current speed with arrows | same | `sea_water_velocity` | `cmap:speed,vectorStyle:solid`, `:solidAndVector`, `:vector`, and the same with `logScale` | m s-1 (0 to 1.035) | same | same |
| Water temperature | `GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy-thetao_anfc_0.083deg_P1D-m_202406` | `thetao` | `cmap:thermal` | degrees_C (-1.74 to 30.04) | 2022-06-01 to 2026-11-26 | model, forecast |
| Sea surface height | `GLOBAL_ANALYSISFORECAST_PHY_001_024/cmems_mod_glo_phy_anfc_0.083deg_P1D-m_202406` | `zos` | `cmap:viridis` (see `tier2.md`) | m | 2022-06-01 to about 2026-11-26 | model, forecast |
| Chlorophyll, satellite | `OCEANCOLOUR_GLO_BGC_L4_NRT_009_102/cmems_obs-oc_glo_bgc-plankton_nrt_l4-gapfree-multi-4km_P1D_202311` | `CHL` | `cmap:algae`, `cmap:algae,logScale` | mg m-3 (0.027 to 1.81) | **rolling 12 days**: 2026-11-04 to 2026-11-15 | observation, gap-free, about 1 to 2 days behind |
| Chlorophyll, model | `GLOBAL_ANALYSISFORECAST_BGC_001_028/cmems_mod_glo_bgc-pft_anfc_0.25deg_P1D-m_202311` | `chl` | `cmap:algae`, `cmap:algae,logScale` | mg m-3 | 2021-11-01 to 2026-11-26 | model, forecast (capabilities fetched, tile not) |
| Sea surface temperature, satellite | `SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001/METOFFICE-GLO-SST-L4-NRT-OBS-SST-V2` | `analysed_sst` | `cmap:thermal`, `cmap:thermal,logScale` | **kelvin** (271.35 to 303.26) | 2024-01-17 to 2026-11-16 | observation analysis, OSTIA |

Findings that bear on the catalogue:

- `thetao` is a model temperature at the first level (0.49 m), 0.083 degree; no SST variable exists in the daily PHY datasets. The OSTIA `analysed_sst` layer is a satellite analysis, in kelvin, and its colour legend is in kelvin. Either layer may be used; credit the product that is shown.
- The PHY `time` range ends about nine days after today because the product is a forecast. A `TIME` beyond the analysis (2026-11-20 for `thetao`) returned a different image from the default, so the future days are served; whether they are labelled forecast is only in the product documentation (**unverified**).
- A `TIME` outside the dataset's range answers HTTP 400 `InvalidParameterValue` with the valid range in the message (fixture `wmts-error-time-out-of-range.xml`). The observation layers (chlorophyll) keep only a rolling window of days. A request with no `TIME` is accepted and returns the default day.
- `QUERYABLE` layers are marked `queryable="1"`; `GetFeatureInfo` was not tried.
- The tile of the chlorophyll layer at zoom 7 (column 37, row 48) also returned 200 (4.4 KB); the maximum zoom was not determined (**unverified**).
- Nothing was found near the freshwater test places: the layers are open-ocean fields and the grid is 0.083 degree (about 9 km) for PHY and 4 km for chlorophyll. Over land, tiles are transparent. They do not resolve the inner harbour, rivers or lakes and must not be presented as local values.

### Request template (one per layer)

```text
https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0
  &LAYER={Product}/{Dataset}/{Variable}&STYLE={Style}&FORMAT=image/png
  &TILEMATRIXSET=EPSG:3857&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}&TIME={YYYY-MM-DD}T00:00:00.000Z
```

For `MKTileOverlay` the placeholders are `{z}`, `{y}`, `{x}`; commas and colons in `STYLE` may stay literal (they were sent literally in all requests here). Examples recorded (zoom 5, column 9, row 12, the New York bight):

| Layer | Fixture |
|---|---|
| `uo` | `wmts-uo-z5-9-12-2026-11-16.png` |
| `sea_water_velocity`, `cmap:speed,vectorStyle:solidAndVector` | `wmts-sea-water-velocity-solidAndVector-z5-9-12-2026-11-16.png` |
| `thetao` | `wmts-thetao-z5-9-12-2026-11-16.png` |
| `CHL` (satellite) | `wmts-chl-oc-z5-9-12-2026-11-15.png` |
| `analysed_sst` | `wmts-analysed-sst-z5-9-12-2026-11-16.png` |

`vo` and `zos` tiles also returned 200 and were not stored (`zos` was recorded on 2026-11-08, see `tier2.md`). The current layers: `uo` and `vo` are single-component colour fields; `sea_water_velocity` with `vectorStyle:solidAndVector` draws speed colours with direction arrows (the useful one for a current layer); pure `vector` and `solid` were not fetched.

## Legends

The capabilities list a `LegendURL` per style in `image/svg+xml` and `application/json`. **The hrefs as published do not work**: they carry `FORMAT=application%252Fjson`, which is encoded twice, and the server answers HTTP 400 `Invalid format: 'application%2Fjson'`. With a single encoding (`FORMAT=application%2Fjson`, or `FORMAT=application/json`) the answers are 200:

```text
https://wmts.marine.copernicus.eu/teroWmts?SERVICE=WMTS&REQUEST=GetLegend
  &LAYER=GLOBAL_ANALYSISFORECAST_PHY_001_024%2Fcmems_mod_glo_phy-thetao_anfc_0.083deg_P1D-m_202406%2Fthetao
  &STYLE=cmap%3Athermal&FORMAT=application%2Fjson
```

- JSON (8.5 KB, `wmts-legend-thetao.json`): `{"continuous":{"clamp":true,"logScale":false,"valueMin":-1.74,"valueMax":30.04,"variableId":"thetao","variableName":"Temperature","cmap":{"colorMap":[[r,g,b],...]}}}`, a 256-step colour table with the value range. This is the form to ship: the app draws its own legend and converts the unit itself. `valueMin`/`valueMax` are the colour range, fixed per layer, not the day's range.
- SVG (34 KB, `FORMAT=image%2Fsvg%2Bxml`): a 125 x 300 legend picture with embedded ticks; not recorded. It has a white background and its own text, so the JSON is preferable.
- `sea_water_velocity` with `STYLE=cmap%3Aspeed%2CvectorStyle%3AsolidAndVector` answered 200 JSON (0 to 1.035).

## Authentication, limits, availability

- No authentication for `GetCapabilities`, `GetTile`, `GetLegend` (checked 2026-11-17). Whether this is intended and permanent is **unverified**: no documentation page for the WMTS was found that says it is open to anonymous use.
- No documented rate limit and no rate-limit headers (2026-11-17). Tiles are cached for 30 days by the server and should be cached by the client by (layer, style, time, z, x, y).
- No outage notice or deprecation for the WMTS was found. The service-level text says the products are free until 30 June 2028 (end of the current service phase). Dataset ids with a version suffix are retired when a product is reprocessed: **unverified** for how long an old suffix keeps working.

## Licence and credit

Source: the Copernicus Marine "Service commitments and licence" page, `https://marine.copernicus.eu/user-corner/service-commitments-and-licence` (read 2026-11-17; the page shows no version or date), and the help article `https://help.marine.copernicus.eu/en/articles/4444611-how-to-cite-copernicus-marine-products-and-services`.

- Grant: "a worldwide, non exclusive, royalty free, perpetual licence", covering internal copies, value-added or derivative works and redistribution of the products in original form.
- Credit: value-added or derivative works "Generated using E.U. Copernicus Marine Service Information" followed by the DOI links; redistribution of products (including web services) "E.U. Copernicus Marine Service Information" followed by the DOI links. The help article gives the source line "Source: European Union, Copernicus Marine Service Information or Data (year), (c) Mercator Ocean" for redistribution. The DOI is the product's, not the dataset's.
- Usage records: the licensee must "maintain such records to document and trace use of Copernicus Marine Service Products" and provide them to the service "upon reasonable notice"; the clause passes to descending licences. The text is written for licensees who signed the agreement and does not say whether it binds anonymous WMTS users. The app keeps no usage records beyond its tile cache.
- Free of charge until the end of the service phase, "planned on 30 June 2028".
- **Not answered by the text** (**unverified**): whether anonymous WMTS access falls under the same licence, and whether the licence requires acceptance by an account holder ("Login and password are personal to the signatory of the SLA"). The WMTS is open in practice; the licence text has no WMTS clause.
- Not for navigation: modelled and analysed fields of 4 to 9 km, not charts.

Credits and DOIs (each resolves at `https://doi.org/<DOI>` to the product page, checked 2026-11-17):

| Product | DOI | Originating centre | Credit text for a layer from it |
|---|---|---|---|
| `GLOBAL_ANALYSISFORECAST_PHY_001_024` (currents, `thetao`, `zos`) | 10.48670/moi-00016 | Mercator Ocean International | Generated using E.U. Copernicus Marine Service Information; https://doi.org/10.48670/moi-00016 |
| `GLOBAL_ANALYSISFORECAST_BGC_001_028` (model chlorophyll) | 10.48670/moi-00015 | Mercator Ocean International | Generated using E.U. Copernicus Marine Service Information; https://doi.org/10.48670/moi-00015 |
| `OCEANCOLOUR_GLO_BGC_L4_NRT_009_102` (satellite chlorophyll) | 10.48670/moi-00279 | ACRI (France) | Generated using E.U. Copernicus Marine Service Information; https://doi.org/10.48670/moi-00279 |
| `SST_GLO_SST_L4_NRT_OBSERVATIONS_010_001` (OSTIA SST) | 10.48670/moi-00165 | Met Office (UK) | Generated using E.U. Copernicus Marine Service Information; https://doi.org/10.48670/moi-00165 |

A screen that shows layers from more than one product lists every DOI of the layers visible.

Re-checked 2026-11-18: unchanged. `https://marine.copernicus.eu/user-corner/service-commitments-and-licence` still contains the worldwide, non-exclusive, royalty-free, perpetual grant, the credit forms, the usage-records clause ("maintain such records to document and trace use of Copernicus Marine Service Products", provided "upon reasonable notice", propagated to descending licences), free of charge until the end of the service phase "planned on 30 June 2028", and "Login and password are personal to the signatory of the SLA". The text still has no clause on anonymous WMTS use, so that question stays **unverified**. Detail not recorded above: the credit "shall be clearly visible on the home page of the Licensee's website or at least on the page allowing to access to the products". The help article is now at `https://help.marine.copernicus.eu/en/articles/4444611-citing-copernicus-marine-products-and-services` (the earlier slug redirects) and still gives "Source: European Union, Copernicus Marine Service Information or Data (year), ©Mercator Ocean" for redistribution. The DOI links were not resolved again.

## Unverified

- Anonymous WMTS access under the licence (2026-11-17, no clause either way).
- `GetFeatureInfo` output and the maximum tile zoom.
- Tiles for the model `chl` layer, `vo` and `zos` content on 2026-11-17 beyond the status code, and the `solid` and `vector` styles.
- How long an old version suffix keeps working after a reprocess.
- Subsetting with an account (not attempted).
