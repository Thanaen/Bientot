//! Time zones named by either platform. The ledger keeps the zone each row was written in as the
//! system named it: a Windows key such as `Romance Standard Time` on Windows, an IANA name such as
//! `Europe/Paris` on Linux. An export carries those names to the other platform, so each reads
//! both.

use bientot_domain::time::UnknownTimeZone;
use jiff::tz::TimeZone;

mod windows;

/// The zone `id` names, a Windows key or an IANA name.
pub fn resolve(id: &str) -> Result<TimeZone, UnknownTimeZone> {
    let name = windows::WINDOWS_ZONES
        .iter()
        .find(|(key, _)| *key == id)
        .map_or(id, |(_, iana)| iana);
    jiff::tz::db()
        .get(name)
        .map_err(|_| UnknownTimeZone(id.to_owned()))
}

#[cfg(test)]
mod tests;
