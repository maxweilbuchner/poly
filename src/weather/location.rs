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

use chrono::NaiveDate;

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
        .and_then(country_icao_from_desc)
        .unwrap_or_default();
    Some(WeatherLocation {
        display,
        country,
        icao,
    })
}

fn country_icao_from_desc(desc: &str) -> Option<(String, String)> {
    country_icao_from_url(desc).or_else(|| country_icao_from_prose(desc))
}

fn country_icao_from_url(desc: &str) -> Option<(String, String)> {
    const NEEDLE: &str = "wunderground.com/history/daily/";
    let start = desc.find(NEEDLE)? + NEEDLE.len();
    let tail = &desc[start..];
    // Accept `-`, `/`, alphanumerics, and `%` (URL-encoded city slugs like
    // `tr/%C3%A7ubuk/LTAC` for Ankara).
    let end = tail
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '/' || c == '%'))
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

/// Some weather markets resolve via NOAA / Hong Kong Observatory rather than
/// Wunderground, so the description names the airport in prose without a URL.
/// Match the small known set explicitly.
fn country_icao_from_prose(desc: &str) -> Option<(String, String)> {
    const PROSE: &[(&str, &str, &str)] = &[
        ("Ben Gurion", "il", "LLBG"),
        ("Vnukovo", "ru", "UUWW"),
        ("Istanbul Airport", "tr", "LTFM"),
        // HKO sits in Tsim Sha Tsui; VHHH (HK Intl) is in the same Open-Meteo
        // grid cell and gives a usable forecast for the temperature reading.
        ("Hong Kong Observatory", "hk", "VHHH"),
    ];
    PROSE.iter().find_map(|(needle, country, icao)| {
        desc.contains(needle)
            .then(|| ((*country).to_string(), (*icao).to_string()))
    })
}

/// Resolution date for a weather market.
///
/// Slugs encode the date deterministically:
/// `highest-temperature-in-milan-on-may-1-2026-16c` → `2026-05-01`.
/// Falls back to the date portion of `end_date` (UTC) when the slug doesn't
/// match — accurate to within a day, which is fine for the D+0..D+2 window
/// check and avoids parsing failures on novel slug formats.
pub fn resolution_date(market: &Market) -> Option<NaiveDate> {
    if let Some(d) = date_from_slug(&market.slug) {
        return Some(d);
    }
    market
        .end_date
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.date_naive())
}

fn date_from_slug(slug: &str) -> Option<NaiveDate> {
    let key = "-on-";
    let idx = slug.find(key)?;
    let tail = &slug[idx + key.len()..];
    // tail = "may-1-2026-16c"
    let mut parts = tail.split('-');
    let month = parts.next()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let year: i32 = parts.next()?.parse().ok()?;
    let month_n = month_from_name(month)?;
    NaiveDate::from_ymd_opt(year, month_n, day)
}

fn month_from_name(name: &str) -> Option<u32> {
    match name {
        "january" | "jan" => Some(1),
        "february" | "feb" => Some(2),
        "march" | "mar" => Some(3),
        "april" | "apr" => Some(4),
        "may" => Some(5),
        "june" | "jun" => Some(6),
        "july" | "jul" => Some(7),
        "august" | "aug" => Some(8),
        "september" | "sep" | "sept" => Some(9),
        "october" | "oct" => Some(10),
        "november" | "nov" => Some(11),
        "december" | "dec" => Some(12),
        _ => None,
    }
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

    #[test]
    fn extracts_ankara_with_url_encoded_city() {
        let m = mk(
            "Will the highest temperature in Ankara be 12°C on May 1?",
            Some("https://www.wunderground.com/history/daily/tr/%C3%A7ubuk/LTAC."),
        );
        assert_eq!(
            weather_location(&m),
            Some(WeatherLocation {
                display: "Ankara".into(),
                country: "tr".into(),
                icao: "LTAC".into(),
            })
        );
    }

    #[test]
    fn extracts_tel_aviv_via_noaa_prose() {
        let m = mk(
            "Will the highest temperature in Tel Aviv be 18°C or below on May 1?",
            Some("...recorded by NOAA at the Ben Gurion International Airport in degrees..."),
        );
        assert_eq!(
            weather_location(&m),
            Some(WeatherLocation {
                display: "Tel Aviv".into(),
                country: "il".into(),
                icao: "LLBG".into(),
            })
        );
    }

    #[test]
    fn extracts_moscow_via_noaa_prose() {
        let m = mk(
            "Will the highest temperature in Moscow be 4°C or below on May 1?",
            Some("...recorded by NOAA at the Vnukovo International Airport..."),
        );
        let loc = weather_location(&m).unwrap();
        assert_eq!(loc.country, "ru");
        assert_eq!(loc.icao, "UUWW");
    }

    #[test]
    fn extracts_istanbul_via_noaa_prose() {
        let m = mk(
            "Will the highest temperature in Istanbul be 7°C or below on May 1?",
            Some("...recorded by NOAA at the Istanbul Airport in degrees..."),
        );
        let loc = weather_location(&m).unwrap();
        assert_eq!(loc.icao, "LTFM");
    }

    #[test]
    fn resolution_date_from_slug() {
        let m = Market {
            slug: "highest-temperature-in-milan-on-may-1-2026-16c".into(),
            ..mk(
                "Will the highest temperature in Milan be 16°C on May 1?",
                None,
            )
        };
        assert_eq!(
            resolution_date(&m),
            Some(NaiveDate::from_ymd_opt(2026, 5, 1).unwrap())
        );
    }

    #[test]
    fn resolution_date_falls_back_to_end_date() {
        let m = Market {
            slug: "totally-unparseable".into(),
            end_date: Some("2026-04-30T23:59:00Z".into()),
            ..mk(
                "Will the highest temperature in X be 1°C on April 30?",
                None,
            )
        };
        assert_eq!(
            resolution_date(&m),
            Some(NaiveDate::from_ymd_opt(2026, 4, 30).unwrap())
        );
    }

    #[test]
    fn extracts_hong_kong_via_observatory_prose() {
        let m = mk(
            "Will the lowest temperature in Hong Kong be 13°C or below on April 30?",
            Some("...recorded by the Hong Kong Observatory in degrees Celsius..."),
        );
        let loc = weather_location(&m).unwrap();
        assert_eq!(loc.country, "hk");
        assert_eq!(loc.icao, "VHHH");
    }
}
