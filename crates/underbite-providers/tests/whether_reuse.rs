//! whether's providers are reachable at the pinned revision with the parts Underbite reuses.

use whether_providers::nhc::current_storms_request;
use whether_providers::redact_url;

#[test]
fn redact_url_masks_an_api_key() {
    let url = "https://api.waterdata.usgs.gov/ogcapi/v1/collections/continuous/items?monitoring_location_id=USGS-01375000&api_key=abc123";
    assert_eq!(
        redact_url(url),
        "https://api.waterdata.usgs.gov/ogcapi/v1/collections/continuous/items?monitoring_location_id=USGS-01375000&api_key=***"
    );
}

#[test]
fn nhc_active_storms_request_carries_no_coordinates() {
    let request = current_storms_request();
    assert_eq!(request.url, "https://www.nhc.noaa.gov/CurrentStorms.json");
}
