//! Dates as signatures, certificates and property lists write them.

use std::time::SystemTime;

use super::der;

/// `time` as its civil date and time of day, in UTC.
fn civil(time: SystemTime) -> (i64, i64, i64, u64, u64, u64) {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    // Howard Hinnant's days-to-civil conversion.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rest / 3600, rest % 3600 / 60, rest % 60)
}

/// `time` as a property list writes a date, `YYYY-MM-DDThh:mm:ssZ`, which
/// sorts as the time does.
pub(crate) fn iso8601(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(time);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

pub(super) fn now_iso8601() -> String {
    iso8601(SystemTime::now())
}

/// `time` as an ASN.1 UTCTime: `YYMMDDhhmmssZ`.
pub(crate) fn utc_time(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(time);
    format!(
        "{:02}{month:02}{day:02}{hour:02}{minute:02}{second:02}Z",
        year % 100
    )
}

/// An ASN.1 UTCTime or GeneralizedTime, as [`iso8601`] writes it. UTCTime
/// years from 50 are 19xx, as X.509 reads them.
pub(super) fn from_asn1(tag: u8, content: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(content).ok()?.strip_suffix('Z')?;
    let (year, rest) = match tag {
        der::UTC_TIME => {
            let short: u32 = text.get(..2)?.parse().ok()?;
            let century = if short >= 50 { 1900 } else { 2000 };
            (century + short, text.get(2..)?)
        }
        der::GENERALIZED_TIME => (text.get(..4)?.parse().ok()?, text.get(4..)?),
        _ => return None,
    };
    if rest.len() < 10 || !rest.bytes().take(10).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let second = rest.get(8..10)?;
    Some(format!(
        "{year:04}-{}-{}T{}:{}:{second}Z",
        &rest[..2],
        &rest[2..4],
        &rest[4..6],
        &rest[6..8]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn times_in_each_form() {
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_793_735_619);
        assert_eq!(utc_time(time), "261103195339Z");
        assert_eq!(utc_time(SystemTime::UNIX_EPOCH), "700101000000Z");
        assert_eq!(iso8601(time), "2026-11-03T19:53:39Z");
        assert_eq!(
            from_asn1(der::UTC_TIME, b"261103195339Z").as_deref(),
            Some("2026-11-03T19:53:39Z")
        );
        assert_eq!(
            from_asn1(der::UTC_TIME, b"991231235959Z").as_deref(),
            Some("1999-12-31T23:59:59Z")
        );
        assert_eq!(
            from_asn1(der::GENERALIZED_TIME, b"21251020120000Z").as_deref(),
            Some("2125-10-20T12:00:00Z")
        );
        assert_eq!(from_asn1(der::UTC_TIME, b"2611Z"), None);
    }
}
