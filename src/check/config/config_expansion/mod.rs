mod expansion;
mod generator;
mod include;
mod presets;
mod source;

pub(crate) use expansion::expand_raw_check_config;
pub(crate) use source::CheckConfigSource;

#[cfg(test)]
mod tests;
