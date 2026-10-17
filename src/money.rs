//! Amounts are integers in minor units; a currency's `exponent` is its decimal
//! places. These helpers never use floating point.

use std::fmt;

/// A decimal string that cannot be converted to minor units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoneyError(String);

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gamebank: {}", self.0)
    }
}

impl std::error::Error for MoneyError {}

/// Converts a decimal string to minor units: `to_minor("100.50", 2)` is `10050`.
/// Commas are ignored. It fails on more decimal places than the exponent
/// allows, and on overflow.
pub fn to_minor(amount: &str, exponent: u32) -> Result<i64, MoneyError> {
    if exponent > 6 {
        return Err(MoneyError("exponent must be 0 to 6".into()));
    }
    let cleaned: String = amount.trim().chars().filter(|c| *c != ',').collect();
    let (neg, s) = match cleaned.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, cleaned.as_str()),
    };
    let (whole, frac, has_dot) = match s.split_once('.') {
        Some((w, f)) => (w, f, true),
        None => (s, "", false),
    };
    let digits = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || (has_dot && frac.is_empty()) || !digits(whole) || !digits(frac) {
        return Err(MoneyError(format!("not a decimal amount: {amount:?}")));
    }
    if frac.len() > exponent as usize {
        return Err(MoneyError(format!(
            "{amount} has more than {exponent} decimal places"
        )));
    }
    let padded = format!(
        "{whole}{frac}{}",
        "0".repeat(exponent as usize - frac.len())
    );
    let n: i64 = padded
        .parse()
        .map_err(|_| MoneyError(format!("{amount} is too large")))?;
    Ok(if neg { -n } else { n })
}

/// Formats minor units as a plain decimal string: `format_minor(10050, 2)` is `"100.50"`.
pub fn format_minor(minor: i64, exponent: u32) -> String {
    let exponent = if exponent > 6 { 0 } else { exponent as usize };
    let sign = if minor < 0 { "-" } else { "" };
    let mut s = minor.unsigned_abs().to_string();
    if exponent == 0 {
        return format!("{sign}{s}");
    }
    if s.len() <= exponent {
        s = format!("{}{s}", "0".repeat(exponent - s.len() + 1));
    }
    let (whole, frac) = s.split_at(s.len() - exponent);
    format!("{sign}{whole}.{frac}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_to_minor() {
        assert_eq!(to_minor("100.50", 2), Ok(10050));
        assert_eq!(to_minor("1,000", 2), Ok(100000));
        assert_eq!(to_minor("-0.5", 2), Ok(-50));
        assert_eq!(to_minor("7", 0), Ok(7));
        assert_eq!(to_minor(" 3.1 ", 3), Ok(3100));
        for bad in [
            "",
            "1.",
            ".5",
            "1.234",
            "abc",
            "1e3",
            "--1",
            "99999999999999999999",
        ] {
            assert!(to_minor(bad, 2).is_err(), "{bad} accepted");
        }
        assert!(to_minor("1", 7).is_err());
    }

    #[test]
    fn formats_minor() {
        assert_eq!(format_minor(10050, 2), "100.50");
        assert_eq!(format_minor(5, 2), "0.05");
        assert_eq!(format_minor(-5, 2), "-0.05");
        assert_eq!(format_minor(42, 0), "42");
        assert_eq!(format_minor(i64::MIN, 2), "-92233720368547758.08");
        assert_eq!(to_minor(&format_minor(123456, 3), 3), Ok(123456));
    }
}
