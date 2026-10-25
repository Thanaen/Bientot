//! The system time zone, named as the .NET app names it in the ledger: by its Windows
//! identifier on Windows (`TimeZoneInfo.Local.Id`), by its IANA name elsewhere.

use bientot_domain::time::{TimeZoneProvider, UnknownTimeZone};
use jiff::tz::TimeZone;

pub struct SystemZones;

impl TimeZoneProvider for SystemZones {
    fn local(&self) -> (String, TimeZone) {
        let zone = TimeZone::system();
        (local_id(&zone), zone)
    }

    /// The local zone under its own identifier, otherwise an IANA zone. A day recorded under
    /// another Windows identifier, after a change of zone, is not resolved yet.
    fn get(&self, id: &str) -> Result<TimeZone, UnknownTimeZone> {
        let (local_id, zone) = self.local();
        if id == local_id {
            return Ok(zone);
        }
        jiff::tz::db()
            .get(id)
            .map_err(|_| UnknownTimeZone(id.to_owned()))
    }
}

#[cfg(windows)]
fn local_id(zone: &TimeZone) -> String {
    use windows::Win32::System::Time::{
        DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation, TIME_ZONE_ID_INVALID,
    };
    let mut information = DYNAMIC_TIME_ZONE_INFORMATION::default();
    // SAFETY: the out pointer is a local for the call.
    let result = unsafe { GetDynamicTimeZoneInformation(&mut information) };
    let key = &information.TimeZoneKeyName;
    let length = key.iter().position(|unit| *unit == 0).unwrap_or(key.len());
    if result == TIME_ZONE_ID_INVALID || length == 0 {
        return iana_id(zone);
    }
    String::from_utf16_lossy(&key[..length])
}

#[cfg(not(windows))]
fn local_id(zone: &TimeZone) -> String {
    iana_id(zone)
}

fn iana_id(zone: &TimeZone) -> String {
    zone.iana_name().unwrap_or("UTC").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_zone_resolves_under_its_own_identifier() {
        let (id, zone) = SystemZones.local();
        assert_eq!(SystemZones.get(&id).unwrap(), zone);
        assert_eq!(
            SystemZones.get("Mars/Olympus_Mons").unwrap_err(),
            UnknownTimeZone("Mars/Olympus_Mons".into())
        );
    }
}
