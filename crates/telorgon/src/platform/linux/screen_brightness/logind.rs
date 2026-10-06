use crate::screen_brightness::ScreenBrightnessError as Error;
use std::time::Duration;
use zbus::{
    blocking::{Connection, Proxy, connection::Builder},
    zvariant::OwnedObjectPath,
};

pub(super) struct Session {
    connection: Connection,
    path: OwnedObjectPath,
    id: String,
    uid: u32,
    seat: String,
}
impl Session {
    pub fn connect(id: Option<&str>, pid: u32, timeout: Duration) -> Result<Self, Error> {
        let connection = Builder::system()
            .map_err(classify)?
            .method_timeout(timeout)
            .build()
            .map_err(classify)?;
        let manager = Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .map_err(classify)?;
        let path: OwnedObjectPath = if let Some(id) = id {
            manager.call("GetSession", &(id,))
        } else {
            manager.call("GetSessionByPID", &(pid,))
        }
        .map_err(classify)?;
        let proxy = Proxy::new(
            &connection,
            "org.freedesktop.login1",
            path.as_str(),
            "org.freedesktop.login1.Session",
        )
        .map_err(classify)?;
        let id: String = proxy.get_property("Id").map_err(classify)?;
        let (uid, _): (u32, OwnedObjectPath) = proxy.get_property("User").map_err(classify)?;
        let (seat, _): (String, OwnedObjectPath) = proxy.get_property("Seat").map_err(classify)?;
        drop(proxy);
        drop(manager);
        if seat.is_empty() {
            return Err(Error::SessionInactive);
        }
        Ok(Self {
            connection,
            path,
            id,
            uid,
            seat,
        })
    }
    #[cfg(feature = "shell-screen-brightness-ddc-linux")]
    pub fn uid(&self) -> u32 {
        self.uid
    }
    fn proxy(&self) -> Result<Proxy<'_>, Error> {
        Proxy::new(
            &self.connection,
            "org.freedesktop.login1",
            self.path.as_str(),
            "org.freedesktop.login1.Session",
        )
        .map_err(classify)
    }
    pub fn authorize(&self, uid: u32, seat: &str) -> Result<(), Error> {
        let proxy = self.proxy()?;
        let id: String = proxy.get_property("Id").map_err(classify)?;
        let (owner, _): (u32, OwnedObjectPath) = proxy.get_property("User").map_err(classify)?;
        let (current_seat, _): (String, OwnedObjectPath) =
            proxy.get_property("Seat").map_err(classify)?;
        if id != self.id
            || owner != self.uid
            || uid != owner
            || seat != self.seat
            || current_seat != self.seat
        {
            return Err(Error::PermissionDenied);
        }
        let active: bool = proxy.get_property("Active").map_err(classify)?;
        let locked: bool = proxy.get_property("LockedHint").map_err(classify)?;
        if !active {
            return Err(Error::SessionInactive);
        }
        if locked {
            return Err(Error::Locked);
        }
        Ok(())
    }
    pub fn set(&self, name: &str, raw: u32) -> Result<(), Error> {
        let result: Result<(), zbus::Error> = self
            .proxy()?
            .call("SetBrightness", &("backlight", name, raw));
        result.map_err(classify)
    }
}
pub(super) fn classify(error: zbus::Error) -> Error {
    match error {
        zbus::Error::MethodError(name, _, _)
            if name.as_str().ends_with("AccessDenied")
                || name.as_str().ends_with("NotYourDevice") =>
        {
            Error::PermissionDenied
        }
        zbus::Error::MethodError(name, _, _) if name.as_str().ends_with("UnknownMethod") => {
            Error::Unsupported
        }
        _ => Error::Transport,
    }
}
