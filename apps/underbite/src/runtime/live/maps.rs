//! The map sources: ERDDAP grids (MUR SST, HF radar, VIIRS chlorophyll and Kd490, GLSEA), the
//! tile services (GIBS, NOAA charts, GEBCO, radar), the NCEI elevation model and Copernicus
//! Marine's public capabilities.

use underbite_core::map::catalogue::{
    self, GridDraw, GridSpec, LAYERS, Layer, LayerKind, TileSource,
};
use underbite_core::map::tiles::{
    TileOutcome, classify, gibs_default_date, lonlat_to_tile, previous_day,
};
use underbite_core::map::{Bbox, GeoPoint, GridSource};
use underbite_core::model::Measurement as M;
use underbite_core::time::{UnixTime, UtcOffset};
use underbite_fetch::Call;
use underbite_providers::copernicus::{capabilities_url, layer_capabilities};
use underbite_providers::erddap::{
    self, ErddapRequest, GridTime, grid_request, parse_currents, parse_grids, parse_time_axis,
    time_axis_request,
};
use underbite_providers::glerl::Glsea;
use underbite_providers::ncei_dem::{
    self, DemCrs, DemRequest, check_response as dem_check, finest, identify_request, parse_export,
    parse_identify, parse_query, query_request, tile_request,
};
use underbite_providers::request::{Coordinates, Response};

use super::{
    BATTERY, ctx, expect, fail, get, get_call, now, parsed, point_target, requirements, run,
    sleep_s, transport,
};

/// Seconds between two requests to one ERDDAP node (it answers 429 beyond ten a minute).
const ERDDAP_PAUSE_S: u64 = 7;

/// The New York Bight around the harbour, on a 0.1 degree grid.
const BIGHT: Bbox = Bbox {
    west: -74.5,
    south: 40.0,
    east: -73.5,
    north: 40.8,
};

/// The stride that puts about 30 cells across a degree.
fn stride_for(source: &GridSource) -> u32 {
    (1.0 / (source.native_step_deg * 30.0)).ceil().max(1.0) as u32
}

/// Reads the times of `source` since ten days ago, then the newest grid over `area`, and checks
/// both parse. Returns the number of cells with a value.
fn erddap_dataset(source: &GridSource, area: Bbox, currents: bool) -> usize {
    let transport = transport();
    let since = UnixTime(now().0 - 10 * 86_400).local_date(UtcOffset(0));
    let axis: ErddapRequest = parsed(
        &source.dataset,
        "the time axis request",
        time_axis_request(source, since),
    );
    let fetched = get_call(&transport, &Call::get(axis.url.clone()));
    let body = parsed(
        &axis.url,
        "the ERDDAP answer",
        erddap::check_response(&fetched.as_response()),
    );
    let times = parsed(&axis.url, "the ERDDAP time axis", parse_time_axis(body));
    let Some(latest) = times.latest() else {
        fail(
            &axis.url,
            format!(
                "{} lists no time in the last ten days (an ended or stalled dataset)",
                source.dataset
            ),
        );
    };

    sleep_s(ERDDAP_PAUSE_S);
    let request = parsed(
        &source.dataset,
        "the grid request",
        grid_request(source, GridTime::At(latest), area, stride_for(source)),
    );
    let fetched = get_call(&transport, &Call::get(request.url.clone()));
    let body = parsed(
        &request.url,
        "the ERDDAP answer",
        erddap::check_response(&fetched.as_response()),
    );
    if currents {
        let grid = parsed(
            &request.url,
            "the ERDDAP currents",
            parse_currents(source, body),
        );
        grid.both_count()
    } else {
        let grids = parsed(&request.url, "the ERDDAP grid", parse_grids(source, body));
        let Some(first) = grids.first() else {
            fail(&request.url, "the ERDDAP table holds no variable");
        };
        expect(
            &request.url,
            !first.lats.is_empty() && !first.lons.is_empty(),
            "the grid has no cells",
        );
        first.valid_count()
    }
}

/// The grid specs of the catalogue's layer `id` and whether they draw currents.
fn grid_specs(id: &str) -> (Vec<GridSource>, bool) {
    let layer = catalogue::find(id).unwrap_or_else(|| panic!("the catalogue has no layer {id}"));
    let grid = layer.grid().unwrap_or_else(|| panic!("{id} is not a grid"));
    (
        grid.specs().map(GridSpec::source).collect(),
        grid.draw == GridDraw::Arrows,
    )
}

/// ERDDAP at the West Coast node: MUR sea surface temperature and HF radar currents over the
/// New York Bight.
#[test]
#[ignore = "live: needs the network"]
fn live_erddap_mur_sst_and_hf_radar() {
    let (mur, _) = grid_specs("sst");
    let valid = erddap_dataset(&mur[0], BIGHT, false);
    expect(
        &mur[0].dataset,
        valid > 0,
        "MUR holds no value over the Bight",
    );

    sleep_s(ERDDAP_PAUSE_S);
    let (hf, currents) = grid_specs("hf-radar-currents");
    expect(&hf[0].dataset, currents, "the HF radar layer draws arrows");
    let valid = erddap_dataset(&hf[0], BIGHT, true);
    expect(
        &hf[0].dataset,
        valid > 0,
        "HF radar holds no current over the Bight",
    );
}

/// ERDDAP at the national node: VIIRS chlorophyll and the Kd490 clarity datasets (the primary
/// and its fallback), over the New York Bight. A cloudy day may leave no value; the shape is
/// what is checked.
#[test]
#[ignore = "live: needs the network"]
fn live_erddap_viirs_chlorophyll_and_kd490() {
    let (chl, _) = grid_specs("chlorophyll");
    let valid = erddap_dataset(&chl[0], BIGHT, false);
    eprintln!("{}: {valid} cells with a value", chl[0].dataset);

    let (kd, _) = grid_specs("clarity");
    for source in &kd {
        sleep_s(ERDDAP_PAUSE_S);
        let valid = erddap_dataset(source, BIGHT, false);
        eprintln!("{}: {valid} cells with a value", source.dataset);
    }
}

/// GLERL's ERDDAP: the Great Lakes surface temperature analysis near the western end of Lake
/// Erie, through its source.
#[test]
#[ignore = "live: needs the network"]
fn live_glerl_glsea() {
    let wanted = [M::WaterTemperature];
    let target = point_target((41.7, -83.1), &wanted);
    let found = run(&Glsea, &target, &requirements(&wanted), 1);
    found.expect_all("GLSEA near Lake Erie's western end", &wanted);

    sleep_s(ERDDAP_PAUSE_S);
    let (glsea, _) = grid_specs("great-lakes-sst");
    let area = Bbox {
        west: -83.6,
        south: 41.3,
        east: -82.6,
        north: 42.1,
    };
    let valid = erddap_dataset(&glsea[0], area, false);
    eprintln!("{}: {valid} cells with a value", glsea[0].dataset);
}

/// Fetches one tile of `layer` and fails unless it is an image, trying the day before once
/// for a dated layer that has none. Returns the number of requests sent.
fn tile_of(layer: &Layer, z: u8, dated: bool) -> usize {
    let transport = transport();
    let service = layer
        .tile_service()
        .unwrap_or_else(|| panic!("{} is not a tiled layer", layer.id));
    let key = lonlat_to_tile(
        GeoPoint {
            lat: BATTERY.0,
            lon: BATTERY.1,
        },
        z,
    );
    let mut date = dated.then(|| gibs_default_date(now()));
    let mut sent = 0;
    loop {
        let Some(url) = layer.tile_url(key, date, 0) else {
            fail(layer.id, format!("the layer has no tile at zoom {z}"));
        };
        sent += 1;
        let fetched = super::send(&transport, &Call::get(url.clone()));
        match classify(
            service,
            fetched.status,
            fetched.content_type.as_deref(),
            fetched.body.len(),
            1,
        ) {
            TileOutcome::Image => return sent,
            TileOutcome::NoData if sent == 1 && date.is_some() => {
                date = date.map(previous_day);
                std::thread::sleep(super::GAP);
            }
            outcome => fail(
                &url,
                format!(
                    "{}: HTTP {} {:?}, {} bytes -> {outcome:?}",
                    layer.id,
                    fetched.status,
                    fetched.content_type,
                    fetched.body.len()
                ),
            ),
        }
    }
}

/// NASA GIBS: one tile of every GIBS imagery layer in the catalogue (the ids and matrix sets
/// are the app's).
#[test]
#[ignore = "live: needs the network"]
fn live_gibs_tiles() {
    let mut checked = 0;
    for layer in catalogue::BASES.iter().chain(LAYERS.iter()) {
        if let LayerKind::Tiles(TileSource::Gibs(spec)) = layer.kind {
            if checked > 0 {
                std::thread::sleep(super::GAP);
            }
            tile_of(layer, 5.min(spec.max_zoom), true);
            checked += 1;
        }
    }
    assert!(checked > 0, "the catalogue has no GIBS layer");
}

/// NOAA Chart Display: the pre-rendered chart tile and every WMS display-group layer answer
/// with an image, not the service's HTML or JSON error under HTTP 200.
#[test]
#[ignore = "live: needs the network"]
fn live_ncds_tiles() {
    let mut checked = 0;
    for layer in catalogue::BASES.iter().chain(LAYERS.iter()) {
        if let LayerKind::Tiles(TileSource::NcdsWmts | TileSource::NcdsWms { .. }) = layer.kind {
            if checked > 0 {
                std::thread::sleep(super::GAP);
            }
            let z = layer.min_zoom.unwrap_or(0).max(12);
            tile_of(layer, z, false);
            checked += 1;
        }
    }
    assert!(checked > 0, "the catalogue has no NOAA chart layer");
}

/// The GEBCO shaded-relief WMS and the NEXRAD radar tiles (Iowa Environmental Mesonet).
#[test]
#[ignore = "live: needs the network"]
fn live_gebco_and_radar_tiles() {
    let gebco = catalogue::find("gebco-relief").expect("the GEBCO layer");
    tile_of(gebco, 7, false);
    std::thread::sleep(super::GAP);
    let radar = catalogue::find("radar").expect("the radar layer");
    tile_of(radar, 7, false);
}

fn dem_get(request: &DemRequest) -> underbite_fetch::Fetched {
    get_call(&transport(), &Call::get(request.url.clone()))
}

/// The NCEI elevation model: the value at the Battery, the source models there, a degree
/// export of the harbour and a Web Mercator tile.
#[test]
#[ignore = "live: needs the network"]
fn live_ncei_dem() {
    let app = ctx();
    let point = app.round(Coordinates {
        latitude: BATTERY.0,
        longitude: BATTERY.1,
    });

    let request = identify_request(point);
    let fetched = dem_get(&request);
    let response: Response<'_> = fetched.as_response();
    let body = parsed(&request.url, "the DEM answer", dem_check(&response));
    let value = parsed(&request.url, "the DEM identify", parse_identify(body));
    expect(
        &request.url,
        value.is_some_and(f32::is_finite),
        "the mosaic has no value at the Battery",
    );

    std::thread::sleep(super::GAP);
    let request = query_request(point);
    let fetched = dem_get(&request);
    let body = parsed(
        &request.url,
        "the DEM answer",
        dem_check(&fetched.as_response()),
    );
    let sources = parsed(&request.url, "the DEM source list", parse_query(body));
    expect(
        &request.url,
        finest(&sources).is_some(),
        "no source model covers the Battery",
    );

    std::thread::sleep(super::GAP);
    let area = Bbox {
        west: -74.1,
        south: 40.6,
        east: -73.9,
        north: 40.8,
    };
    let request = parsed(
        "https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer",
        "the area request",
        ncei_dem::area_request(area, 0.002),
    );
    let fetched = dem_get(&request);
    let body = parsed(
        &request.url,
        "the DEM answer",
        dem_check(&fetched.as_response()),
    );
    let image = parsed(
        &request.url,
        "the DEM export",
        parse_export(body, DemCrs::Wgs84),
    );
    expect(
        &request.url,
        image.range().is_some(),
        "the harbour export holds no elevation",
    );

    std::thread::sleep(super::GAP);
    let key = lonlat_to_tile(
        GeoPoint {
            lat: BATTERY.0,
            lon: BATTERY.1,
        },
        10,
    );
    let request = parsed(
        "https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer",
        "the tile request",
        tile_request(key, 256),
    );
    let fetched = dem_get(&request);
    let body = parsed(
        &request.url,
        "the DEM answer",
        dem_check(&fetched.as_response()),
    );
    let image = parsed(
        &request.url,
        "the DEM tile",
        parse_export(body, DemCrs::WebMercator),
    );
    expect(
        &request.url,
        image.range().is_some(),
        "the tile export holds no elevation",
    );
}

/// Copernicus Marine's WMTS: the public capabilities of each dataset the catalogue draws
/// (no credentials) still list the layer, with a time range up to a recent day.
#[test]
#[ignore = "live: needs the network"]
fn live_copernicus_capabilities() {
    let transport = transport();
    let mut checked = 0;
    for layer in LAYERS.iter().filter(|l| l.needs_copernicus()) {
        let spec = layer.copernicus().expect("a Copernicus layer has its spec");
        let url = capabilities_url(spec);
        if checked > 0 {
            std::thread::sleep(super::GAP);
        }
        let fetched = get(&transport, &url);
        let text = String::from_utf8_lossy(&fetched.body).into_owned();
        let found = parsed(
            &url,
            &format!("the capabilities of {}", spec.layer()),
            layer_capabilities(&text, spec),
        );
        eprintln!("{}: {found:?}", layer.id);
        checked += 1;
    }
    assert!(checked > 0, "the catalogue has no Copernicus layer");
}
