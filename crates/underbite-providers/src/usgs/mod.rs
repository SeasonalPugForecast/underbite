//! USGS Water Data APIs.

use underbite_core::model::{Measurement, Requirements};
use underbite_core::provenance::{Attribution, SourceId};

use crate::request::{Request, Response, Target};
use crate::source::{Budget, FetchContext, Fragments, Period, Source, SourceError};

pub mod ogc;
pub mod sites;
pub mod statistics;

/// The USGS Water Data source: the newest values of a site's series, their recent history and
/// daily values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usgs {
    /// How old the newest value of a series may be before the series counts as dead, seconds.
    pub max_age_s: i64,
    /// Metered requests allowed per hour (a key raises USGS's own limit).
    pub hourly_budget: u32,
}

impl Default for Usgs {
    fn default() -> Self {
        Self {
            max_age_s: 3 * 3_600,
            hourly_budget: 1_000,
        }
    }
}

impl Usgs {
    /// A source with the default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Series whose newest value is older than `seconds` are dropped.
    #[must_use]
    pub fn with_max_age(mut self, seconds: i64) -> Self {
        self.max_age_s = seconds;
        self
    }

    /// The most metered requests per hour.
    #[must_use]
    pub fn with_hourly_budget(mut self, limit: u32) -> Self {
        self.hourly_budget = limit;
        self
    }
}

impl Source for Usgs {
    fn id(&self) -> SourceId {
        SourceId::USGS
    }

    fn attribution(&self) -> Attribution {
        ogc::attribution()
    }

    fn measurements(&self) -> &[Measurement] {
        ogc::MEASUREMENTS
    }

    fn budget(&self) -> Option<Budget> {
        Some(Budget {
            period: Period::Hour,
            limit: self.hourly_budget,
        })
    }

    fn plan(
        &self,
        target: &Target,
        _requirements: &Requirements,
        ctx: &FetchContext,
    ) -> Result<Vec<Request>, SourceError> {
        ogc::plan(target, ctx)
    }

    fn parse(
        &self,
        request: &Request,
        response: &Response<'_>,
        ctx: &FetchContext,
        out: &mut Fragments,
    ) -> Result<(), SourceError> {
        ogc::parse(self, request, response, ctx, out)
    }
}
