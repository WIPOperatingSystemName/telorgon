//! Bounded X11 window lifetimes and wire-event routing. Policy decides whether to
//! honor requests; only server notifications change actual map/geometry state.
//! The owner must feed events in connection order, including initialization events.
use super::{
    Error, Result,
    association::{Association, Associations, XWindow},
};
use std::collections::BTreeMap;
use x11rb_protocol::{protocol::xproto, x11_utils::TryParse};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
    pub border: u16,
}
impl Geometry {
    /// Convert a root child's desktop geometry using the owner's selected layout.
    /// Callers must retain/check the layout revision before issuing a configure.
    /// Child-relative geometry must not use root normalization.
    pub fn from_desktop(
        root: crate::compositor_wayland::OutputRootGeometry,
        rect: crate::core::RectI,
        border: u16,
    ) -> Result<Self> {
        let point = root
            .desktop_to_root(crate::core::PointI {
                x: rect.x,
                y: rect.y,
            })
            .map_err(|error| Error(error.to_string()))?;
        let invalid = || Error("desktop geometry exceeds X11 core geometry limits".into());
        if rect.width <= 0 || rect.height <= 0 {
            return Err(invalid());
        }
        Ok(Self {
            x: point.x.try_into().map_err(|_| invalid())?,
            y: point.y.try_into().map_err(|_| invalid())?,
            width: rect.width.try_into().map_err(|_| invalid())?,
            height: rect.height.try_into().map_err(|_| invalid())?,
            border,
        })
    }
    /// Inverse for root-relative geometry; dimensions and border are not scaled.
    pub fn desktop_rect(
        self,
        root: crate::compositor_wayland::OutputRootGeometry,
    ) -> Result<crate::core::RectI> {
        let point = root
            .root_to_desktop(crate::core::PointI {
                x: self.x.into(),
                y: self.y.into(),
            })
            .map_err(|error| Error(error.to_string()))?;
        Ok(crate::core::RectI {
            x: point.x,
            y: point.y,
            width: self.width.into(),
            height: self.height.into(),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: XWindow,
    pub parent: u32,
    pub geometry: Geometry,
    pub override_redirect: bool,
    pub mapped: bool,
}
/// Only fields selected by the wire value mask are requested. Unselected wire
/// values have no policy meaning. Sibling XIDs are hints to resolve before acting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestedConfigure {
    pub x: Option<i16>,
    pub y: Option<i16>,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub border: Option<u16>,
    pub sibling: Option<u32>,
    pub stack_mode: Option<u8>,
}
impl RequestedConfigure {
    fn parse(e: &xproto::ConfigureRequestEvent) -> Result<Self> {
        let mask = u16::from(e.value_mask);
        if mask & !127 != 0 || (mask & 64 != 0 && u32::from(e.stack_mode) > 4) {
            return Err(Error("invalid XWM configure request mask/mode".into()));
        }
        Ok(Self {
            x: (mask & 1 != 0).then_some(e.x),
            y: (mask & 2 != 0).then_some(e.y),
            width: (mask & 4 != 0).then_some(e.width),
            height: (mask & 8 != 0).then_some(e.height),
            border: (mask & 16 != 0).then_some(e.border_width),
            sibling: (mask & 32 != 0).then_some(e.sibling),
            stack_mode: (mask & 64 != 0).then_some(u32::from(e.stack_mode) as u8),
        })
    }
}
#[derive(Clone, Copy)]
enum PendingPolicy {
    Map,
    Configure(RequestedConfigure),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Created(XWindow),
    Changed(XWindow),
    Destroyed(XWindow),
    RequestMap(XWindow),
    RequestConfigure(XWindow, RequestedConfigure),
    CommandFailed(XWindow),
    ProtocolsChanged(XWindow),
    HintsChanged(XWindow),
    NormalHintsChanged(XWindow),
    /// A root child not yet observed needs asynchronous attributes/geometry reads.
    Inspect(u32),
    Associated(Association),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inspection {
    pub(crate) generation: u64,
    pub(crate) xid: u32,
    nonce: u64,
}
pub struct Windows {
    generation: u64,
    capacity: usize,
    next_inspection: u64,
    inspections: BTreeMap<u32, Inspection>,
    pending_serials: BTreeMap<u32, Vec<u64>>,
    pending_serial_count: usize,
    pending_policy: BTreeMap<u32, Vec<PendingPolicy>>,
    pending_policy_count: usize,
    root: u32,
    manager: u32,
    serial_atom: u32,
    windows: BTreeMap<u32, Window>,
    associations: Associations,
}
impl Windows {
    pub fn new(
        generation: u64,
        capacity: usize,
        root: u32,
        manager: u32,
        serial_atom: u32,
    ) -> Result<Self> {
        if root == 0 || manager == 0 || root == manager || serial_atom == 0 {
            return Err(Error("invalid XWM window registry configuration".into()));
        }
        Ok(Self {
            generation,
            capacity,
            next_inspection: 1,
            inspections: BTreeMap::new(),
            pending_serials: BTreeMap::new(),
            pending_serial_count: 0,
            pending_policy: BTreeMap::new(),
            pending_policy_count: 0,
            root,
            manager,
            serial_atom,
            windows: BTreeMap::new(),
            associations: Associations::new(generation, capacity)?,
        })
    }
    pub(crate) fn inspection_context(&self) -> (u64, u32, u32, usize) {
        (self.generation, self.root, self.manager, self.capacity)
    }
    /// Start a snapshot read only for an unknown non-manager root child.
    pub fn begin_inspection(&mut self, xid: u32) -> Result<Option<Inspection>> {
        if xid == 0
            || xid == self.root
            || xid == self.manager
            || self.windows.contains_key(&xid)
            || self.inspections.contains_key(&xid)
        {
            return Ok(None);
        }
        if self.windows.len() + self.inspections.len() >= self.capacity {
            return Err(Error("XWM window inspection bound reached".into()));
        }
        let nonce = self.next_inspection;
        self.next_inspection = nonce
            .checked_add(1)
            .ok_or_else(|| Error("XWM inspection identity exhausted".into()))?;
        let token = Inspection {
            generation: self.generation,
            xid,
            nonce,
        };
        self.inspections.insert(xid, token);
        Ok(Some(token))
    }
    pub fn cancel_inspection(&mut self, token: Inspection) {
        if self.inspections.get(&token.xid) == Some(&token) {
            self.inspections.remove(&token.xid);
        }
    }
    fn clear_pending_serials(&mut self, xid: u32) -> Vec<u64> {
        let serials = self.pending_serials.remove(&xid).unwrap_or_default();
        self.pending_serial_count -= serials.len();
        serials
    }
    fn defer_policy(&mut self, xid: u32, request: PendingPolicy) -> Result<()> {
        if self.pending_policy_count >= self.capacity
            || self.pending_policy.get(&xid).is_some_and(|p| p.len() >= 16)
        {
            return Err(Error("pending XWM policy request bound reached".into()));
        }
        self.pending_policy.entry(xid).or_default().push(request);
        self.pending_policy_count += 1;
        Ok(())
    }
    fn take_policy(&mut self, xid: u32) -> Vec<PendingPolicy> {
        let pending = self.pending_policy.remove(&xid).unwrap_or_default();
        self.pending_policy_count -= pending.len();
        pending
    }
    fn discard_pending(&mut self, xid: u32) {
        self.clear_pending_serials(xid);
        self.take_policy(xid);
    }
    /// Discard metadata for an invalid/non-presentation window, provided this
    /// cancellation still belongs to its current inspection lifetime.
    pub fn discard_inspection(&mut self, token: Inspection) {
        if self.inspections.get(&token.xid) == Some(&token) {
            self.inspections.remove(&token.xid);
            self.discard_pending(token.xid);
        }
    }
    pub fn finish_inspection(
        &mut self,
        token: Inspection,
        parent: u32,
        geometry: Geometry,
        override_redirect: bool,
        mapped: bool,
    ) -> Result<Vec<Action>> {
        if self.inspections.get(&token.xid) != Some(&token) {
            return Ok(vec![]);
        }
        self.inspections.remove(&token.xid);
        if parent != self.root || self.windows.contains_key(&token.xid) {
            self.discard_pending(token.xid);
            return Ok(vec![]);
        }
        let id = self.associations.create_window(token.xid)?;
        self.windows.insert(
            token.xid,
            Window {
                id,
                parent,
                geometry,
                override_redirect,
                mapped,
            },
        );
        let mut actions = vec![Action::Created(id)];
        actions.extend(self.take_policy(token.xid).into_iter().map(|p| match p {
            PendingPolicy::Map => Action::RequestMap(id),
            PendingPolicy::Configure(request) => Action::RequestConfigure(id, request),
        }));
        let mut associated = None;
        for serial in self.clear_pending_serials(token.xid) {
            associated = self.associations.window_serial(id, serial)?.or(associated);
        }
        actions.extend(associated.map(Action::Associated));
        Ok(actions)
    }
    /// Live protocol windows; presentation eligibility still requires association.
    pub fn iter(&self) -> impl Iterator<Item = &Window> {
        self.windows.values()
    }
    pub fn get(&self, xid: u32) -> Option<&Window> {
        self.windows.get(&xid)
    }
    /// Presentation/input eligibility needs a mapped root child and a committed
    /// authenticated Wayland association. Metadata alone grants neither.
    pub fn presentable_surface(&self, id: XWindow) -> Option<u64> {
        let window = self.windows.get(&id.xid)?;
        if window.id != id || !window.mapped || window.parent != self.root {
            return None;
        }
        self.associations.surface_for(id)
    }
    /// Call only for authenticated, newly committed serials, not every buffer commit.
    pub fn committed_surface(
        &mut self,
        generation: u64,
        surface: u64,
        serial: u64,
    ) -> Result<Option<Action>> {
        Ok(self
            .associations
            .committed_surface(generation, surface, serial)?
            .map(Action::Associated))
    }
    pub fn destroy_surface(&mut self, generation: u64, surface: u64) -> Result<()> {
        self.associations.destroy_surface(generation, surface)
    }
    /// Returns None for unrelated events. Malformed relevant events fail this instance.
    /// Synthetic lifecycle notifications cannot mutate authoritative window state.
    pub fn event(&mut self, bytes: &[u8]) -> Result<Option<Action>> {
        let Some(&kind) = bytes.first() else {
            return Err(Error("empty XWM window event".into()));
        };
        if kind & 128 != 0 && kind & 127 != xproto::CLIENT_MESSAGE_EVENT {
            return Ok(None);
        }
        let malformed = |_| Error("malformed XWM window event".into());
        match kind & 127 {
            xproto::CREATE_NOTIFY_EVENT => {
                let (e, _) = xproto::CreateNotifyEvent::try_parse(bytes).map_err(malformed)?;
                self.discard_pending(e.window);
                self.inspections.remove(&e.window);
                if e.parent != self.root || e.window == self.manager {
                    return Ok(None);
                }
                let id = self.associations.create_window(e.window)?;
                self.windows.insert(
                    e.window,
                    Window {
                        id,
                        parent: e.parent,
                        geometry: Geometry {
                            x: e.x,
                            y: e.y,
                            width: e.width,
                            height: e.height,
                            border: e.border_width,
                        },
                        override_redirect: e.override_redirect,
                        mapped: false,
                    },
                );
                Ok(Some(Action::Created(id)))
            }
            xproto::MAP_REQUEST_EVENT => {
                let (e, _) = xproto::MapRequestEvent::try_parse(bytes).map_err(malformed)?;
                if e.parent != self.root || e.window == self.manager {
                    return Ok(None);
                }
                if let Some(w) = self.windows.get(&e.window) {
                    Ok(Some(Action::RequestMap(w.id)))
                } else {
                    self.defer_policy(e.window, PendingPolicy::Map)?;
                    Ok(Some(Action::Inspect(e.window)))
                }
            }
            xproto::CONFIGURE_REQUEST_EVENT => {
                let (e, _) = xproto::ConfigureRequestEvent::try_parse(bytes).map_err(malformed)?;
                if e.parent != self.root || e.window == self.manager {
                    return Ok(None);
                }
                let request = RequestedConfigure::parse(&e)?;
                if let Some(w) = self.windows.get(&e.window) {
                    Ok(Some(Action::RequestConfigure(w.id, request)))
                } else {
                    self.defer_policy(e.window, PendingPolicy::Configure(request))?;
                    Ok(Some(Action::Inspect(e.window)))
                }
            }
            xproto::MAP_NOTIFY_EVENT => {
                let (e, _) = xproto::MapNotifyEvent::try_parse(bytes).map_err(malformed)?;
                self.inspections.remove(&e.window);
                if e.window == self.manager {
                    return Ok(None);
                }
                let Some(w) = self.windows.get_mut(&e.window) else {
                    return Ok((e.event == self.root).then_some(Action::Inspect(e.window)));
                };
                w.mapped = true;
                w.override_redirect = e.override_redirect;
                Ok(Some(Action::Changed(w.id)))
            }
            xproto::UNMAP_NOTIFY_EVENT => {
                let (e, _) = xproto::UnmapNotifyEvent::try_parse(bytes).map_err(malformed)?;
                self.inspections.remove(&e.window);
                let Some(w) = self.windows.get_mut(&e.window) else {
                    return Ok((e.event == self.root).then_some(Action::Inspect(e.window)));
                };
                w.mapped = false;
                Ok(Some(Action::Changed(w.id)))
            }
            xproto::DESTROY_NOTIFY_EVENT => {
                let (e, _) = xproto::DestroyNotifyEvent::try_parse(bytes).map_err(malformed)?;
                self.discard_pending(e.window);
                self.inspections.remove(&e.window);
                let Some(w) = self.windows.remove(&e.window) else {
                    return Ok(None);
                };
                self.associations.destroy_window(w.id);
                Ok(Some(Action::Destroyed(w.id)))
            }
            xproto::CONFIGURE_NOTIFY_EVENT => {
                let (e, _) = xproto::ConfigureNotifyEvent::try_parse(bytes).map_err(malformed)?;
                self.inspections.remove(&e.window);
                let Some(w) = self.windows.get_mut(&e.window) else {
                    return Ok((e.event == self.root).then_some(Action::Inspect(e.window)));
                };
                w.geometry = Geometry {
                    x: e.x,
                    y: e.y,
                    width: e.width,
                    height: e.height,
                    border: e.border_width,
                };
                w.override_redirect = e.override_redirect;
                Ok(Some(Action::Changed(w.id)))
            }
            xproto::REPARENT_NOTIFY_EVENT => {
                let (e, _) = xproto::ReparentNotifyEvent::try_parse(bytes).map_err(malformed)?;
                self.inspections.remove(&e.window);
                if e.window == self.manager {
                    return Ok(None);
                }
                let Some(w) = self.windows.get_mut(&e.window) else {
                    return Ok((e.parent == self.root).then_some(Action::Inspect(e.window)));
                };
                w.parent = e.parent;
                w.geometry.x = e.x;
                w.geometry.y = e.y;
                w.override_redirect = e.override_redirect;
                Ok(Some(Action::Changed(w.id)))
            }
            xproto::CLIENT_MESSAGE_EVENT => {
                let (e, _) = xproto::ClientMessageEvent::try_parse(bytes).map_err(malformed)?;
                if e.type_ != self.serial_atom {
                    return Ok(None);
                }
                if e.format != 32 {
                    return Err(Error("invalid Xwayland serial message format".into()));
                }
                let data = e.data.as_data32();
                let serial = u64::from(data[0]) | (u64::from(data[1]) << 32);
                if e.window == 0 || e.window == self.root || e.window == self.manager {
                    return Ok(None);
                }
                if serial == 0 {
                    return Err(Error("zero Xwayland serial".into()));
                }
                let Some(w) = self.windows.get(&e.window) else {
                    let previous = self.pending_serials.get(&e.window);
                    if !previous.is_some_and(|values| values.contains(&serial)) {
                        if self.pending_serial_count >= self.capacity
                            || previous.is_some_and(|values| values.len() >= 16)
                        {
                            return Err(Error("pending Xwayland serial bound reached".into()));
                        }
                        self.pending_serials
                            .entry(e.window)
                            .or_default()
                            .push(serial);
                        self.pending_serial_count += 1;
                    }
                    return Ok(Some(Action::Inspect(e.window)));
                };
                Ok(self
                    .associations
                    .window_serial(w.id, serial)?
                    .map(Action::Associated))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn registry() -> Windows {
        Windows::new(1, 16, 1, 2, 100).unwrap()
    }
    fn create(w: &mut Windows, popup: bool) -> XWindow {
        let event = xproto::CreateNotifyEvent {
            response_type: xproto::CREATE_NOTIFY_EVENT,
            parent: 1,
            window: 10,
            width: 640,
            height: 480,
            override_redirect: popup,
            ..Default::default()
        };
        let bytes: [u8; 32] = event.into();
        let Some(Action::Created(id)) = w.event(&bytes).unwrap() else {
            panic!("missing creation");
        };
        id
    }
    fn serial(w: &mut Windows, value: u64) -> Option<Action> {
        let bytes: [u8; 32] = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT | 128,
            format: 32,
            sequence: 0,
            window: 10,
            type_: 100,
            data: [value as u32, (value >> 32) as u32, 0, 0, 0].into(),
        }
        .into();
        w.event(&bytes).unwrap()
    }
    fn map(w: &mut Windows, popup: bool) {
        let bytes: [u8; 32] = xproto::MapNotifyEvent {
            response_type: xproto::MAP_NOTIFY_EVENT,
            event: 1,
            window: 10,
            override_redirect: popup,
            ..Default::default()
        }
        .into();
        w.event(&bytes).unwrap();
    }
    fn finish(w: &mut Windows, token: Inspection) -> Vec<Action> {
        w.finish_inspection(
            token,
            1,
            Geometry {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
                border: 0,
            },
            false,
            true,
        )
        .unwrap()
    }
    fn configure_request(mask: u16, x: i16, width: u16) -> [u8; 32] {
        xproto::ConfigureRequestEvent {
            response_type: xproto::CONFIGURE_REQUEST_EVENT,
            sequence: 0,
            parent: 1,
            window: 10,
            sibling: 99,
            x,
            y: 999,
            width,
            height: 999,
            border_width: 99,
            stack_mode: xproto::StackMode::ABOVE,
            value_mask: mask.into(),
        }
        .into()
    }
    #[test]
    fn configure_mask_preserves_requested_fields_without_changing_actual_geometry() {
        let mut w = registry();
        let id = create(&mut w, false);
        let Some(Action::RequestConfigure(target, request)) =
            w.event(&configure_request(5, -25, 800)).unwrap()
        else {
            panic!("missing request");
        };
        assert_eq!(target, id);
        assert_eq!(
            request,
            RequestedConfigure {
                x: Some(-25),
                width: Some(800),
                ..Default::default()
            }
        );
        assert_eq!(w.get(10).unwrap().geometry.x, 0);
        assert_eq!(w.get(10).unwrap().geometry.width, 640);
        assert!(w.event(&configure_request(128, 0, 0)).is_err());
    }
    #[test]
    fn requests_during_inspection_replay_in_order_without_coalescing() {
        let mut w = registry();
        let token = w.begin_inspection(10).unwrap().unwrap();
        w.event(&configure_request(1, -25, 0)).unwrap();
        let map: [u8; 32] = xproto::MapRequestEvent {
            response_type: xproto::MAP_REQUEST_EVENT,
            parent: 1,
            window: 10,
            ..Default::default()
        }
        .into();
        w.event(&map).unwrap();
        w.event(&configure_request(4, 0, 800)).unwrap();
        let actions = finish(&mut w, token);
        let id = w.get(10).unwrap().id;
        assert_eq!(
            actions,
            vec![
                Action::Created(id),
                Action::RequestConfigure(
                    id,
                    RequestedConfigure {
                        x: Some(-25),
                        ..Default::default()
                    }
                ),
                Action::RequestMap(id),
                Action::RequestConfigure(
                    id,
                    RequestedConfigure {
                        width: Some(800),
                        ..Default::default()
                    }
                )
            ]
        );
        assert_eq!(w.pending_policy_count, 0);
    }
    #[test]
    fn request_retention_is_bounded_and_destroy_releases_it() {
        let mut w = Windows::new(1, 2, 1, 2, 100).unwrap();
        w.event(&configure_request(1, 1, 0)).unwrap();
        w.event(&configure_request(1, 2, 0)).unwrap();
        assert!(w.event(&configure_request(1, 3, 0)).is_err());
        assert_eq!(w.pending_policy_count, 2);
        let destroy: [u8; 32] = xproto::DestroyNotifyEvent {
            response_type: xproto::DESTROY_NOTIFY_EVENT,
            event: 1,
            window: 10,
            ..Default::default()
        }
        .into();
        w.event(&destroy).unwrap();
        assert_eq!(w.pending_policy_count, 0);
        let token = w.begin_inspection(10).unwrap().unwrap();
        assert_eq!(finish(&mut w, token).len(), 1);
    }
    #[test]
    fn unknown_serial_survives_inspection_in_either_commit_order() {
        for surface_first in [false, true] {
            let mut w = registry();
            assert_eq!(serial(&mut w, (5u64 << 32) | 7), Some(Action::Inspect(10)));
            let token = w.begin_inspection(10).unwrap().unwrap();
            if surface_first {
                assert_eq!(w.committed_surface(1, 200, (5u64 << 32) | 7).unwrap(), None);
            }
            let actions = finish(&mut w, token);
            let id = w.get(10).unwrap().id;
            assert_eq!(actions[0], Action::Created(id));
            if surface_first {
                assert_eq!(
                    actions[1],
                    Action::Associated(Association {
                        window: id,
                        surface: 200
                    })
                );
            } else {
                assert_eq!(actions.len(), 1);
                assert_eq!(
                    w.committed_surface(1, 200, (5u64 << 32) | 7).unwrap(),
                    Some(Action::Associated(Association {
                        window: id,
                        surface: 200
                    }))
                );
            }
            assert_eq!(w.presentable_surface(id), Some(200));
            assert_eq!(w.pending_serial_count, 0);
        }
    }
    #[test]
    fn retry_preserves_serials_and_stale_inspection_cannot_discard_them() {
        let mut w = registry();
        let old = w.begin_inspection(10).unwrap().unwrap();
        serial(&mut w, 7);
        serial(&mut w, 7); // Duplicate notifications consume no extra storage.
        assert_eq!(w.pending_serial_count, 1);
        w.cancel_inspection(old);
        let new = w.begin_inspection(10).unwrap().unwrap();
        w.discard_inspection(old);
        assert!(finish(&mut w, old).is_empty());
        w.committed_surface(1, 200, 7).unwrap();
        assert_eq!(finish(&mut w, new).len(), 2);
        let id = w.get(10).unwrap().id;
        assert_eq!(w.presentable_surface(id), Some(200));
    }
    #[test]
    fn destroy_discards_unobserved_serial_before_xid_reuse() {
        let mut w = registry();
        let old = w.begin_inspection(10).unwrap().unwrap();
        serial(&mut w, 7);
        w.committed_surface(1, 200, 7).unwrap();
        let bytes: [u8; 32] = xproto::DestroyNotifyEvent {
            response_type: xproto::DESTROY_NOTIFY_EVENT,
            event: 1,
            window: 10,
            ..Default::default()
        }
        .into();
        w.event(&bytes).unwrap();
        assert_eq!(w.pending_serial_count, 0);
        assert!(finish(&mut w, old).is_empty());
        let new = w.begin_inspection(10).unwrap().unwrap();
        assert_eq!(finish(&mut w, new).len(), 1);
        assert_eq!(w.presentable_surface(w.get(10).unwrap().id), None);
    }
    #[test]
    fn pending_serial_bounds_fail_explicitly_and_discard_reclaims_capacity() {
        let mut w = Windows::new(1, 2, 1, 2, 100).unwrap();
        let token = w.begin_inspection(10).unwrap().unwrap();
        serial(&mut w, 7);
        serial(&mut w, 8);
        let bytes: [u8; 32] = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT | 128,
            format: 32,
            sequence: 0,
            window: 10,
            type_: 100,
            data: [9u32, 0, 0, 0, 0].into(),
        }
        .into();
        assert!(w.event(&bytes).is_err());
        assert_eq!(w.pending_serial_count, 2);
        w.discard_inspection(token);
        assert_eq!(w.pending_serial_count, 0);
        assert_eq!(w.event(&bytes).unwrap(), Some(Action::Inspect(10)));
    }
    #[test]
    fn requests_do_not_map_and_either_association_order_works() {
        for surface_first in [false, true] {
            for popup in [false, true] {
                let mut w = registry();
                let id = create(&mut w, popup);
                let bytes: [u8; 32] = xproto::MapRequestEvent {
                    response_type: xproto::MAP_REQUEST_EVENT,
                    parent: 1,
                    window: 10,
                    ..Default::default()
                }
                .into();
                assert_eq!(w.event(&bytes).unwrap(), Some(Action::RequestMap(id)));
                assert!(!w.get(10).unwrap().mapped);
                let value = (7u64 << 32) | 42;
                let joined = if surface_first {
                    assert_eq!(w.committed_surface(1, 200, value).unwrap(), None);
                    serial(&mut w, value)
                } else {
                    assert_eq!(serial(&mut w, value), None);
                    w.committed_surface(1, 200, value).unwrap()
                };
                assert_eq!(
                    joined,
                    Some(Action::Associated(Association {
                        window: id,
                        surface: 200
                    }))
                );
                assert_eq!(w.presentable_surface(id), None);
                map(&mut w, popup);
                assert_eq!(w.presentable_surface(id), Some(200));
                assert_eq!(w.get(10).unwrap().override_redirect, popup);
            }
        }
    }
    #[test]
    fn unmap_surface_loss_and_reparent_preserve_identity_but_revoke_presentation() {
        let mut w = registry();
        let id = create(&mut w, false);
        serial(&mut w, 42);
        w.committed_surface(1, 200, 42).unwrap();
        map(&mut w, false);
        let bytes: [u8; 32] = xproto::UnmapNotifyEvent {
            response_type: xproto::UNMAP_NOTIFY_EVENT,
            event: 1,
            window: 10,
            ..Default::default()
        }
        .into();
        w.event(&bytes).unwrap();
        assert_eq!(w.presentable_surface(id), None);
        assert_eq!(w.get(10).unwrap().id, id);
        map(&mut w, false);
        let bytes: [u8; 32] = xproto::ReparentNotifyEvent {
            response_type: xproto::REPARENT_NOTIFY_EVENT,
            event: 1,
            window: 10,
            parent: 99,
            x: -10,
            y: 20,
            ..Default::default()
        }
        .into();
        w.event(&bytes).unwrap();
        assert_eq!(w.presentable_surface(id), None);
        assert_eq!(w.get(10).unwrap().geometry.x, -10);
        w.destroy_surface(1, 200).unwrap();
        assert_eq!(w.get(10).unwrap().id, id);
    }
    #[test]
    fn destroy_reuse_cannot_inherit_surface_or_accept_old_identity() {
        let mut w = registry();
        let old = create(&mut w, false);
        serial(&mut w, 42);
        let bytes: [u8; 32] = xproto::DestroyNotifyEvent {
            response_type: xproto::DESTROY_NOTIFY_EVENT,
            event: 1,
            window: 10,
            ..Default::default()
        }
        .into();
        assert_eq!(w.event(&bytes).unwrap(), Some(Action::Destroyed(old)));
        let new = create(&mut w, false);
        assert_ne!(new, old);
        assert_eq!(w.committed_surface(1, 200, 42).unwrap(), None);
        map(&mut w, false);
        assert_eq!(w.presentable_surface(old), None);
        assert_eq!(w.presentable_surface(new), None);
        assert_eq!(w.committed_surface(2, 300, 43).unwrap(), None);
    }
    #[test]
    fn synthetic_lifecycle_cannot_destroy_or_map_windows() {
        let mut w = registry();
        let id = create(&mut w, false);
        let bytes: [u8; 32] = xproto::MapNotifyEvent {
            response_type: xproto::MAP_NOTIFY_EVENT | 128,
            event: 1,
            window: 10,
            ..Default::default()
        }
        .into();
        assert_eq!(w.event(&bytes).unwrap(), None);
        assert!(!w.get(10).unwrap().mapped);
        let bytes: [u8; 32] = xproto::DestroyNotifyEvent {
            response_type: xproto::DESTROY_NOTIFY_EVENT | 128,
            event: 1,
            window: 10,
            ..Default::default()
        }
        .into();
        assert_eq!(w.event(&bytes).unwrap(), None);
        assert_eq!(w.get(10).unwrap().id, id);
    }
    #[test]
    fn malformed_serial_and_exhausted_window_capacity_fail_explicitly() {
        let mut w = Windows::new(1, 1, 1, 2, 100).unwrap();
        create(&mut w, false);
        let bytes: [u8; 32] = xproto::ClientMessageEvent {
            response_type: xproto::CLIENT_MESSAGE_EVENT | 128,
            format: 8,
            window: 10,
            type_: 100,
            sequence: 0,
            data: [0u32; 5].into(),
        }
        .into();
        assert!(w.event(&bytes).is_err());
        let bytes: [u8; 32] = xproto::CreateNotifyEvent {
            response_type: xproto::CREATE_NOTIFY_EVENT,
            parent: 1,
            window: 11,
            ..Default::default()
        }
        .into();
        assert!(w.event(&bytes).is_err());
        assert!(w.get(11).is_none());
        assert!(w.event(&[xproto::CREATE_NOTIFY_EVENT]).is_err());
    }
}
