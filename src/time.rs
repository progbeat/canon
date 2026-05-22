pub(crate) fn unix_timestamp() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|err| format!("system time is before UNIX_EPOCH: {}", err))
}

pub(crate) fn format_record_timestamp(seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts_from_unix_seconds(seconds);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

pub(crate) fn parse_record_timestamp(value: &str) -> Option<u64> {
    if value.len() != 20 || !value.ends_with('Z') {
        return None;
    }
    if &value[4..5] != "-"
        || &value[7..8] != "-"
        || &value[10..11] != "T"
        || &value[13..14] != ":"
        || &value[16..17] != ":"
    {
        return None;
    }
    let year = value[0..4].parse::<i64>().ok()?;
    let month = value[5..7].parse::<u32>().ok()?;
    let day = value[8..10].parse::<u32>().ok()?;
    let hour = value[11..13].parse::<u64>().ok()?;
    let minute = value[14..16].parse::<u64>().ok()?;
    let second = value[17..19].parse::<u64>().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    if civil_from_days(days) != (year, month, day) {
        return None;
    }
    let seconds = days
        .checked_mul(86_400)?
        .checked_add((hour * 3_600 + minute * 60 + second) as i64)?;
    u64::try_from(seconds).ok()
}

pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    let year = year - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day = day as i64;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    if !(0..366).contains(&day_of_year) {
        return None;
    }
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

pub(crate) fn utc_parts_from_unix_seconds(seconds: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    (year, month, day, hour, minute, second)
}

pub(crate) fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let days = days_since_unix_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month as u32, day as u32)
}
