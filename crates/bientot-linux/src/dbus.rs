//! The method calls the session and system buses share.

pub(crate) const BUS: &str = "org.freedesktop.DBus";
pub(crate) const BUS_PATH: &str = "/org/freedesktop/DBus";

pub(crate) async fn call<B, R>(
    bus: &zbus::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &B,
) -> zbus::Result<R>
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> zbus::export::serde::Deserialize<'d> + zbus::zvariant::Type,
{
    bus.call_method(Some(destination), path, Some(interface), method, body)
        .await?
        .body()
        .deserialize()
}

/// Whether a client owns `name`. Calling a name nobody owns could start its service.
pub(crate) async fn has_owner(bus: &zbus::Connection, name: &str) -> bool {
    call(bus, BUS, BUS_PATH, BUS, "NameHasOwner", &(name,))
        .await
        .unwrap_or(false)
}
