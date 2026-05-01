//! Extract the location a Polymarket weather market resolves against.
//!
//! Weather markets follow a fixed template: question text names a city, and the
//! description embeds a Wunderground URL like
//! `https://www.wunderground.com/history/daily/it/milan/LIMC` (or
//! `.../us/wa/seatac/KSEA` for US markets, which insert a state segment) whose
//! first segment is the ISO country code and last segment is the ICAO station
//! code the market resolves against.
//!
//! The helper combines both sources: question gives a clean display name
//! ("Seattle"), URL gives the canonical `(country, icao)` pair.

use crate::types::Market;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeatherLocation {
    /// Human-readable city for display, sourced from the question.
    pub display: String,
    /// ISO-3166-1 alpha-2 country code, lowercased. Empty if URL unavailable.
    pub country: String,
    /// ICAO station code, uppercased (e.g. "KSEA"). Empty if URL unavailable.
    pub icao: String,
}

pub fn weather_location(market: &Market) -> Option<WeatherLocation> {
    let display = display_from_question(&market.question)?;
    let (country, icao) = market
        .description
        .as_deref()
        .and_then(country_icao_from_url)
        .unwrap_or_default();
    Some(WeatherLocation {
        display,
        country,
        icao,
    })
}

fn country_icao_from_url(desc: &str) -> Option<(String, String)> {
    const NEEDLE: &str = "wunderground.com/history/daily/";
    let start = desc.find(NEEDLE)? + NEEDLE.len();
    let tail = &desc[start..];
    let end = tail
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '/'))
        .unwrap_or(tail.len());
    let segments: Vec<&str> = tail[..end].split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 3 {
        return None;
    }
    let country = segments[0];
    let icao = *segments.last().unwrap();

    if country.len() != 2 || !country.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    if !(3..=4).contains(&icao.len())
        || !icao
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return None;
    }
    Some((country.to_string(), icao.to_string()))
}

fn display_from_question(question: &str) -> Option<String> {
    let lower = question.to_lowercase();
    let (idx, klen) = ["highest temperature in ", "lowest temperature in "]
        .iter()
        .find_map(|k| lower.find(k).map(|i| (i, k.len())))?;
    let after = &question[idx + klen..];
    // Template: "... in <City> be <range> on <date>?". Stop at " be ".
    let end = after.to_lowercase().find(" be ").unwrap_or(after.len());
    let display = after[..end].trim().to_string();
    if display.is_empty() {
        None
    } else {
        Some(display)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Market, MarketStatus};

    fn mk(question: &str, description: Option<&str>) -> Market {
        Market {
            condition_id: String::new(),
            question: question.to_string(),
            description: description.map(str::to_string),
            slug: String::new(),
            group_slug: String::new(),
            status: MarketStatus::Active,
            end_date: None,
            volume: 0.0,
            liquidity: 0.0,
            outcomes: vec![],
            category: None,
            tags: vec![],
            neg_risk: false,
        }
    }

    #[test]
    fn extracts_milan() {
        let m = mk(
            "Will the highest temperature in Milan be 16°C on May 1?",
            Some("...available here: https://www.wunderground.com/history/daily/it/milan/LIMC.\n\nMore."),
        );
        assert_eq!(
            weather_location(&m),
            Some(WeatherLocation {
                display: "Milan".into(),
                country: "it".into(),
                icao: "LIMC".into(),
            })
        );
    }

    #[test]
    fn extracts_seattle_with_state_segment() {
        let m = mk(
            "Will the highest temperature in Seattle be between 70-71°F on April 29?",
            Some("https://www.wunderground.com/history/daily/us/wa/seatac/KSEA."),
        );
        assert_eq!(
            weather_location(&m),
            Some(WeatherLocation {
                display: "Seattle".into(),
                country: "us".into(),
                icao: "KSEA".into(),
            })
        );
    }

    #[test]
    fn extracts_helsinki() {
        let m = mk(
            "Will the highest temperature in Helsinki be 17°C on April 30?",
            Some("https://www.wunderground.com/history/daily/fi/vantaa/EFHK"),
        );
        assert_eq!(
            weather_location(&m),
            Some(WeatherLocation {
                display: "Helsinki".into(),
                country: "fi".into(),
                icao: "EFHK".into(),
            })
        );
    }

    #[test]
    fn extracts_toronto() {
        let m = mk(
            "Will the highest temperature in Toronto be 15°C on April 30?",
            Some("https://www.wunderground.com/history/daily/ca/mississauga/CYYZ"),
        );
        assert_eq!(
            weather_location(&m),
            Some(WeatherLocation {
                display: "Toronto".into(),
                country: "ca".into(),
                icao: "CYYZ".into(),
            })
        );
    }

    #[test]
    fn extracts_miami_lowest_temperature() {
        let m = mk(
            "Will the lowest temperature in Miami be between 72-73°F on April 28?",
            Some("https://www.wunderground.com/history/daily/us/fl/miami/KMIA"),
        );
        assert_eq!(
            weather_location(&m),
            Some(WeatherLocation {
                display: "Miami".into(),
                country: "us".into(),
                icao: "KMIA".into(),
            })
        );
    }

    #[test]
    fn falls_back_when_url_missing() {
        let m = mk(
            "Will the highest temperature in Tokyo be 22°C on May 2?",
            Some("Description without a wunderground link."),
        );
        assert_eq!(
            weather_location(&m),
            Some(WeatherLocation {
                display: "Tokyo".into(),
                country: String::new(),
                icao: String::new(),
            })
        );
    }

    #[test]
    fn falls_back_when_description_none() {
        let m = mk(
            "Will the highest temperature in Berlin be 18°C on May 3?",
            None,
        );
        assert_eq!(
            weather_location(&m),
            Some(WeatherLocation {
                display: "Berlin".into(),
                country: String::new(),
                icao: String::new(),
            })
        );
    }

    #[test]
    fn returns_none_for_non_weather_market() {
        let m = mk("Will Trump win the 2028 election?", None);
        assert_eq!(weather_location(&m), None);
    }
}
