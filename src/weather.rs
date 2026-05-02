pub mod airports;
pub mod location;

pub use airports::lookup as lookup_airport;
#[allow(unused_imports)]
pub use airports::Airport;
#[allow(unused_imports)]
pub use location::WeatherLocation;
pub use location::{resolution_date, weather_location};

/// Format the *current* local civil time at an airport in `h:mm am/pm` form.
/// Returns `None` if the airport's tz string isn't a valid IANA name (which
/// would indicate a stale generated table).
pub fn local_time_now(airport: &Airport) -> Option<String> {
    let tz: chrono_tz::Tz = airport.tz.parse().ok()?;
    let now = chrono::Utc::now().with_timezone(&tz);
    Some(format_h_mm_ampm(now))
}

fn format_h_mm_ampm<Tz: chrono::TimeZone>(dt: chrono::DateTime<Tz>) -> String {
    use chrono::Timelike;
    let h24 = dt.hour();
    let h12 = match h24 % 12 {
        0 => 12,
        n => n,
    };
    let suffix = if h24 < 12 { "am" } else { "pm" };
    format!("{}:{:02} {}", h12, dt.minute(), suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn format_handles_noon_midnight_and_pm() {
        let utc = chrono::Utc;
        assert_eq!(
            format_h_mm_ampm(utc.with_ymd_and_hms(2026, 5, 1, 0, 5, 0).unwrap()),
            "12:05 am"
        );
        assert_eq!(
            format_h_mm_ampm(utc.with_ymd_and_hms(2026, 5, 1, 11, 0, 0).unwrap()),
            "11:00 am"
        );
        assert_eq!(
            format_h_mm_ampm(utc.with_ymd_and_hms(2026, 5, 1, 12, 30, 0).unwrap()),
            "12:30 pm"
        );
        assert_eq!(
            format_h_mm_ampm(utc.with_ymd_and_hms(2026, 5, 1, 23, 59, 0).unwrap()),
            "11:59 pm"
        );
    }

    #[test]
    fn local_time_uses_airport_tz() {
        let lhr = lookup_airport("EGLC").expect("EGLC in table");
        // Just verify the function returns Some; exact time depends on wall clock.
        assert!(local_time_now(lhr).is_some());
    }
}
