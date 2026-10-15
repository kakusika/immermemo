//! Formatting and stringification utilities for dates, values, and presentation.

pub mod time;

pub use time::{days_to_ymd, format_time_only, format_timestamp};
