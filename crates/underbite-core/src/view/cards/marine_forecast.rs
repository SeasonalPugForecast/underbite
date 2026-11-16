//! The marine zone forecast text.

use schemars::JsonSchema;
use serde::Serialize;

use crate::model::{Measurement, ZoneProduct};
use crate::provenance::SourceId;
use crate::view::format::Formatter;
use crate::view::{
    CardChrome, CardId, CardState, Compact, CreditVm, NoticeKind, Quantity, ViewInput,
};

/// A forecast older than this is stale unless the user set another age limit: the NWS reissues
/// marine zone forecasts every few hours.
const DEFAULT_MAX_AGE_S: i64 = 12 * 3_600;

/// The NWS marine forecast page of a zone.
fn zone_page(zone_id: &str) -> String {
    format!("https://forecast.weather.gov/shmrn.php?mz={zone_id}")
}

const fn product_label(product: ZoneProduct) -> &'static str {
    match product {
        ZoneProduct::ZoneApi => "Zone forecast",
        ZoneProduct::Cwf => "Coastal waters forecast",
        ZoneProduct::Nsh => "Nearshore marine forecast",
        ZoneProduct::Glf => "Open lake forecast",
        ZoneProduct::Off => "Offshore waters forecast",
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MarineForecastCard {
    pub chrome: CardChrome,
    pub compact: Compact,
    pub full: MarineForecastFull,
}

/// One period of the forecast, text exactly as issued (never truncated).
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ForecastPeriodVm {
    pub name: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct MarineForecastFull {
    pub zone_id: String,
    pub zone_name: String,
    /// "Issued 4:09 AM"
    pub issued: String,
    /// "Coastal waters forecast"
    pub product: String,
    /// The first period, for the compact form.
    pub headline: Option<String>,
    pub periods: Vec<ForecastPeriodVm>,
}

/// The zone forecast with its periods exactly as issued; never parsed into numbers or shortened.
pub fn build(input: &ViewInput) -> Option<MarineForecastCard> {
    let forecast = input.snapshot.marine.marine_forecast.as_ref()?;
    if forecast.periods.is_empty() {
        return None;
    }
    let f = Formatter::for_input(input);
    let now = input.now;
    let same_day = f.date_of(forecast.issued) == f.date_of(now);
    let issued = if same_day {
        format!("Issued {}", f.clock(forecast.issued))
    } else {
        format!(
            "Issued {}, {}",
            f.date_of(forecast.issued),
            f.clock(forecast.issued)
        )
    };
    let product = product_label(forecast.product).to_owned();
    let periods: Vec<ForecastPeriodVm> = forecast
        .periods
        .iter()
        .map(|p| ForecastPeriodVm {
            name: p.name.clone(),
            text: p.text.clone(),
        })
        .collect();
    let first = periods.first()?;
    let headline = Some(first.text.clone());

    let age = (now.0 - forecast.issued.0).max(0);
    let limit = input
        .limit(Measurement::MarineForecast)
        .and_then(|l| l.max_age_s)
        .unwrap_or(DEFAULT_MAX_AGE_S);
    let state = if age > limit {
        CardState::Stale {
            reason: f.stale_reason(age, limit),
        }
    } else {
        CardState::Fresh
    };

    let credit = {
        let found = input
            .snapshot
            .attribution
            .iter()
            .find(|a| a.source == SourceId::NWS);
        CreditVm {
            source: "National Weather Service".to_owned(),
            text: found.map_or_else(
                || "Marine forecast: NOAA National Weather Service".to_owned(),
                |a| a.credit.clone(),
            ),
            link: Some(zone_page(&forecast.id)),
            licence: None,
        }
    };
    let title = f.strings.card_title(CardId::MarineForecast).to_owned();
    let accessibility = format!(
        "{title} for {}. {product}, {}. {}: {}",
        forecast.name,
        issued.to_lowercase(),
        first.name,
        first.text
    );
    Some(MarineForecastCard {
        chrome: CardChrome {
            id: CardId::MarineForecast,
            title,
            footnote: Some(format!("{} · {} · {issued}", forecast.name, product)),
            credits: vec![credit],
            state,
            notices: vec![f.notice(NoticeKind::TextOnly)],
            accessibility,
        },
        compact: Compact {
            headline: Quantity {
                value: None,
                text: first.text.clone(),
                unit: String::new(),
            },
            lines: vec![first.name.clone(), format!("{} · {issued}", forecast.name)],
            spark: Vec::new(),
        },
        full: MarineForecastFull {
            zone_id: forecast.id.clone(),
            zone_name: forecast.name.clone(),
            issued,
            product,
            headline,
            periods,
        },
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{ForecastPeriod, ZoneForecast, ZoneKind};
    use crate::time::{UnixTime, parse_rfc3339};
    use crate::view::Voice;
    use crate::view::tests::sample_input;

    /// The recorded ANZ338 zone forecast, as the provider would parse it.
    pub(crate) fn anz338() -> ZoneForecast {
        let path = format!(
            "{}/../../tests/fixtures/raw/nws-marine/zone-forecast-ANZ338.ld.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        ZoneForecast {
            id: "ANZ338".into(),
            name: "New York Harbor".into(),
            kind: ZoneKind::Coastal,
            issued: parse_rfc3339(json["updated"].as_str().unwrap()).unwrap(),
            product: ZoneProduct::ZoneApi,
            periods: json["periods"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| ForecastPeriod {
                    name: p["name"].as_str().unwrap().into(),
                    text: p["detailedForecast"].as_str().unwrap().into(),
                })
                .collect(),
        }
    }

    fn input(hours_after_issue: i64, voice: Voice) -> ViewInput {
        let mut input = sample_input();
        let forecast = anz338();
        input.now = UnixTime(forecast.issued.0 + hours_after_issue * 3_600);
        input.voice = voice;
        input.snapshot.marine.marine_forecast = Some(forecast);
        input
    }

    #[test]
    fn the_periods_are_shown_exactly_as_issued() {
        let input = input(2, Voice::Luna);
        let source = input.snapshot.marine.marine_forecast.clone().unwrap();
        let card = build(&input).unwrap();
        assert_eq!(card.full.periods.len(), 10);
        for (shown, given) in card.full.periods.iter().zip(&source.periods) {
            assert_eq!((&shown.name, &shown.text), (&given.name, &given.text));
        }
        assert_eq!(card.full.zone_id, "ANZ338");
        assert_eq!(card.full.issued, "Issued 1:57 AM");
        assert_eq!(card.full.product, "Zone forecast");
        assert_eq!(
            card.full.headline.as_deref(),
            Some(source.periods[0].text.as_str())
        );
        assert_eq!(card.chrome.notices.len(), 1);
        assert_eq!(card.chrome.notices[0].kind, NoticeKind::TextOnly);
        assert_eq!(card.chrome.state, CardState::Fresh);
        assert_eq!(
            card.chrome.credits[0].link.as_deref(),
            Some("https://forecast.weather.gov/shmrn.php?mz=ANZ338")
        );
        insta::assert_json_snapshot!("marine_forecast_anz338_luna", card);
    }

    #[test]
    fn plain_voice_gives_the_same_forecast_text() {
        let luna = build(&input(2, Voice::Luna)).unwrap();
        let plain = build(&input(2, Voice::Plain)).unwrap();
        assert_eq!(luna.full, plain.full);
        insta::assert_json_snapshot!("marine_forecast_anz338_plain", plain);
    }

    #[test]
    fn an_old_forecast_is_stale_and_names_the_day() {
        let card = build(&input(13, Voice::Plain)).unwrap();
        assert!(matches!(card.chrome.state, CardState::Stale { .. }));
        let mut later = input(30, Voice::Plain);
        later
            .snapshot
            .marine
            .marine_forecast
            .as_mut()
            .unwrap()
            .product = ZoneProduct::Nsh;
        let card = build(&later).unwrap();
        assert_eq!(card.full.issued, "Issued Nov 8, 1:57 AM");
        assert_eq!(card.full.product, "Nearshore marine forecast");
    }

    #[test]
    fn no_forecast_no_card() {
        let mut input = input(2, Voice::Plain);
        input.snapshot.marine.marine_forecast = None;
        assert!(build(&input).is_none());
    }
}
