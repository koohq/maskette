//! Random ids and timestamps in the old backend's formats.

use std::time::{SystemTime, UNIX_EPOCH};

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    // The OS random source does not fail on the desktops Maskette runs on.
    getrandom::fill(&mut bytes).expect("OS random source");
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A random UUID (version 4) in the usual 8-4-4-4-12 form, like Python's
/// `str(uuid.uuid4())`.
pub fn uuid4() -> String {
    let mut bytes: [u8; 16] = random_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// `chars` random lowercase hex digits (Python's `uuid4().hex[:chars]`).
pub fn short_hex(chars: usize) -> String {
    let bytes: [u8; 16] = random_bytes();
    let mut text = hex(&bytes);
    text.truncate(chars.min(32));
    text
}

/// The current time like Python's `datetime.now(UTC).isoformat()`:
/// `2026-11-08T03:04:05.123456+00:00`.
pub fn now_iso() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    iso_utc(elapsed.as_secs(), elapsed.subsec_micros())
}

fn iso_utc(seconds: u64, micros: u32) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{micros:06}+00:00",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_look_like_python_isoformat() {
        assert_eq!(iso_utc(0, 0), "1970-01-01T00:00:00.000000+00:00");
        // 2026-11-08 03:04:05 UTC
        assert_eq!(
            iso_utc(1_794_107_045, 42),
            "2026-11-08T03:04:05.000042+00:00"
        );
        assert_eq!(iso_utc(951_782_400, 0), "2000-02-29T00:00:00.000000+00:00");
    }

    #[test]
    fn uuids_have_the_version_4_shape() {
        let id = uuid4();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert_eq!(short_hex(6).len(), 6);
    }
}
