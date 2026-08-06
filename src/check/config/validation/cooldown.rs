use crate::config_types::{Cooldown, CooldownConfig};

pub(crate) fn parse_cooldown_config(value: &CooldownConfig) -> Result<Cooldown, String> {
    // Raw compact-duration validation belongs to the config boundary. The
    // expansion path calls this before constructing canonical expectations,
    // whose cooldown is already represented as seconds.
    Ok(Cooldown {
        seconds: parse_cooldown_duration(&value.0)?,
    })
}

fn parse_cooldown_duration(value: &str) -> Result<u64, String> {
    if value.trim() != value {
        return Err("must use compact duration syntax without surrounding whitespace".to_string());
    }
    let Some((unit_index, unit)) = value.char_indices().next_back() else {
        return Err("must use integer duration with unit s, m, h, d, or w".to_string());
    };
    if unit_index == 0 {
        return Err("must use integer duration with unit s, m, h, d, or w".to_string());
    }
    let digits = &value[..unit_index];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("must start with an integer".to_string());
    }
    let amount = digits
        .parse::<u64>()
        .map_err(|_| "duration integer is too large".to_string())?;
    if amount == 0 {
        return Err("must be greater than zero".to_string());
    }
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        'w' => 7 * 24 * 60 * 60,
        _ => return Err("unit must be one of s, m, h, d, or w".to_string()),
    };
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| "duration is too large".to_string())
}
