//! The storm-ahead check and the hurricane watch for a spot.

use underbite_core::model::{Spot, StormWatch};
use underbite_providers::source::FetchContext;
use underbite_providers::storms::{StormOptions, StormOutcome, StormWatchPlan};

use crate::host::{Call, Driver, FetchHost};

/// Runs the storm-ahead check and the hurricane watch for `spot`, given its previous watch.
pub(crate) async fn fetch_storms<H: FetchHost>(
    host: &Driver<'_, H>,
    ctx: &FetchContext,
    spot: &Spot,
    previous: Option<&StormWatch>,
    hurricane_distance_km: f64,
) -> StormOutcome {
    let options = StormOptions {
        hurricane_distance_km,
        allow_metered: host.allow_metered,
    };
    let mut plan = StormWatchPlan::new(spot, previous, &options, ctx, Some(host.usage.clone()));
    while let Some(request) = plan.next_request() {
        let call = Call::from(&request.http).with_cache(request.cache);
        let metered = request.http.metered;
        if metered && let Err(e) = host.save_usage().await {
            tracing::warn!(error = %e, "storm request not sent");
            plan.on_transport_error(&e);
            continue;
        }
        match host.send(&call).await {
            Ok(fetched) => plan.on_response(&fetched.as_response()),
            Err(e) => plan.on_transport_error(&e.to_string()),
        }
    }
    plan.finish()
}
