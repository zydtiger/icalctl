use crate::config::MAX_TRAVEL_RANGE_DAYS;
use anyhow::{Context, Result, anyhow, bail};
use chrono::{Days, NaiveDate};

#[derive(Debug)]
pub(super) struct DateRange {
    pub(super) start: NaiveDate,
    pub(super) end: NaiveDate,
    pub(super) future_only_start: bool,
}

pub(super) fn parse_range(
    url: &str,
    today: NaiveDate,
    default_range_days: u32,
) -> Result<DateRange> {
    let query = url.split_once('?').map(|(_, query)| query).unwrap_or("");
    let mut start = None;
    let mut end = None;
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        let slot = match key.as_ref() {
            "start" => &mut start,
            "end" => &mut end,
            _ => bail!("unknown query parameter `{key}`"),
        };
        if slot.replace(value.into_owned()).is_some() {
            bail!("query parameter `{key}` may only be specified once");
        }
    }

    match (start, end) {
        (None, None) => {
            let days = default_range_days
                .checked_sub(1)
                .ok_or_else(|| anyhow!("travel.default_range_days must be greater than zero"))?;
            if default_range_days > MAX_TRAVEL_RANGE_DAYS {
                bail!(
                    "travel.default_range_days must not exceed {MAX_TRAVEL_RANGE_DAYS} inclusive days"
                );
            }
            let end = today
                .checked_add_days(Days::new(u64::from(days)))
                .ok_or_else(|| anyhow!("default travel date range is out of bounds"))?;
            Ok(DateRange {
                start: today,
                end,
                future_only_start: true,
            })
        }
        (Some(start), Some(end)) => {
            let start = parse_query_date(&start, "start")?;
            let end = parse_query_date(&end, "end")?;
            if start > end {
                bail!("start must be on or before end");
            }
            let inclusive_days = end.signed_duration_since(start).num_days() + 1;
            if inclusive_days > i64::from(MAX_TRAVEL_RANGE_DAYS) {
                bail!("date range must not exceed {MAX_TRAVEL_RANGE_DAYS} inclusive days");
            }
            Ok(DateRange {
                start,
                end,
                future_only_start: false,
            })
        }
        _ => bail!("start and end must be provided together"),
    }
}

fn parse_query_date(value: &str, name: &str) -> Result<NaiveDate> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        bail!("{name} must use YYYY-MM-DD");
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .with_context(|| format!("{name} must use YYYY-MM-DD"))
}
