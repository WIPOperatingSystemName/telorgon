use std::ffi::CStr;
use std::os::fd::AsRawFd;
use std::ptr::NonNull;
use std::slice;

use crate::foundation::SizeI;

use crate::graphics::presentation::kms::ffi;
use crate::graphics::presentation::kms::{
    KmsConnectorId, KmsDevice, KmsError, KmsErrorKind, KmsPlaneId, KmsPropertyId,
};

const DRM_MODE_CONNECTED: u32 = 1;
const DRM_MODE_TYPE_PREFERRED: u32 = 1 << 3;
const DRM_MODE_OBJECT_CRTC: u32 = 0xcccc_cccc;
const DRM_MODE_OBJECT_CONNECTOR: u32 = 0xc0c0_c0c0;
const DRM_MODE_OBJECT_PLANE: u32 = 0xeeee_eeee;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectorStatus {
    Connected,
    Disconnected,
    Unknown,
}

#[derive(Clone, Copy)]
pub struct KmsConnectorMode {
    native: ffi::drmModeModeInfo,
}

impl KmsConnectorMode {
    pub fn name(&self) -> String {
        unsafe { CStr::from_ptr(self.native.name.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    pub fn size(&self) -> SizeI {
        SizeI {
            width: i32::from(self.native.hdisplay),
            height: i32::from(self.native.vdisplay),
        }
    }

    pub fn refresh_millihertz(&self) -> u32 {
        self.native.vrefresh.saturating_mul(1000)
    }

    pub fn preferred(&self) -> bool {
        self.native.type_ & DRM_MODE_TYPE_PREFERRED != 0
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        unsafe {
            slice::from_raw_parts(
                (&self.native as *const ffi::drmModeModeInfo).cast::<u8>(),
                std::mem::size_of::<ffi::drmModeModeInfo>(),
            )
        }
    }
}

impl std::fmt::Debug for KmsConnectorMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KmsConnectorMode")
            .field("name", &self.name())
            .field("size", &self.size())
            .field("refresh_millihertz", &self.refresh_millihertz())
            .field("preferred", &self.preferred())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct KmsConnector {
    pub id: KmsConnectorId,
    pub connector_type: u32,
    pub connector_type_id: u32,
    pub status: ConnectorStatus,
    pub physical_millimeters: SizeI,
    pub modes: Vec<KmsConnectorMode>,
    pub possible_encoders: Vec<u32>,
    pub possible_crtcs_mask: u32,
}

#[derive(Clone, Debug)]
pub struct KmsPlane {
    pub id: KmsPlaneId,
    pub possible_crtcs_mask: u32,
    pub formats: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct KmsProperty {
    pub id: KmsPropertyId,
    pub name: String,
    pub flags: u32,
    pub value: u64,
}

#[derive(Clone, Debug, Default)]
pub struct KmsObjectProperties {
    pub properties: Vec<KmsProperty>,
}

impl KmsObjectProperties {
    pub fn named(&self, name: &str) -> Option<&KmsProperty> {
        self.properties
            .iter()
            .find(|property| property.name == name)
    }
}

#[derive(Clone, Debug)]
pub struct KmsTopology {
    pub crtcs: Vec<u32>,
    pub connectors: Vec<KmsConnector>,
    pub planes: Vec<KmsPlane>,
    pub minimum_size: SizeI,
    pub maximum_size: SizeI,
}

impl KmsTopology {
    pub fn query(device: &KmsDevice) -> Result<Self, KmsError> {
        let guard = ResourcesGuard::new(device)?;
        let resources = unsafe { guard.raw.as_ref() };
        let crtcs = checked_slice(resources.crtcs, resources.count_crtcs)?.to_vec();
        let connector_ids = checked_slice(resources.connectors, resources.count_connectors)?;
        let mut connectors = Vec::with_capacity(connector_ids.len());
        for id in connector_ids {
            if let Some(connector) = query_connector(device, *id)? {
                connectors.push(connector);
            }
        }
        Ok(Self {
            crtcs,
            connectors,
            planes: query_planes(device)?,
            minimum_size: SizeI {
                width: resources.min_width as i32,
                height: resources.min_height as i32,
            },
            maximum_size: SizeI {
                width: resources.max_width as i32,
                height: resources.max_height as i32,
            },
        })
    }

    pub fn object_properties(
        device: &KmsDevice,
        object_id: u32,
        object: KmsPropertyObject,
    ) -> Result<KmsObjectProperties, KmsError> {
        let object_type = match object {
            KmsPropertyObject::Crtc => DRM_MODE_OBJECT_CRTC,
            KmsPropertyObject::Connector => DRM_MODE_OBJECT_CONNECTOR,
            KmsPropertyObject::Plane => DRM_MODE_OBJECT_PLANE,
        };
        let guard = ObjectPropertiesGuard::new(device, object_id, object_type)?;
        let native = unsafe { guard.raw.as_ref() };
        if native.count_props > 65_536
            || (native.count_props != 0 && (native.props.is_null() || native.prop_values.is_null()))
        {
            return Err(malformed());
        }
        let ids = checked_slice(native.props, native.count_props as i32)?;
        let values = checked_slice(native.prop_values, native.count_props as i32)?;
        let mut properties = Vec::with_capacity(ids.len());
        for (id, value) in ids.iter().zip(values) {
            let Some(property_raw) =
                NonNull::new(unsafe { ffi::drmModeGetProperty(device.fd().as_raw_fd(), *id) })
            else {
                continue;
            };
            let property = unsafe { property_raw.as_ref() };
            let name = unsafe { CStr::from_ptr(property.name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            if let (Some(id), false) = (KmsPropertyId::from_raw(property.prop_id), name.is_empty())
            {
                properties.push(KmsProperty {
                    id,
                    name,
                    flags: property.flags,
                    value: *value,
                });
            }
            unsafe { ffi::drmModeFreeProperty(property_raw.as_ptr()) };
        }
        Ok(KmsObjectProperties { properties })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KmsPropertyObject {
    Crtc,
    Connector,
    Plane,
}

impl KmsDevice {
    pub fn create_mode_blob<'device>(
        &'device self,
        mode: &KmsConnectorMode,
    ) -> Result<crate::graphics::presentation::kms::PropertyBlob<'device>, KmsError> {
        self.create_property_blob(mode.bytes())
    }
}

struct ResourcesGuard {
    raw: NonNull<ffi::drmModeRes>,
}

impl ResourcesGuard {
    fn new(device: &KmsDevice) -> Result<Self, KmsError> {
        Ok(Self {
            raw: NonNull::new(unsafe { ffi::drmModeGetResources(device.fd().as_raw_fd()) })
                .ok_or_else(|| KmsError::new(KmsErrorKind::Native, "DRM resource query failed"))?,
        })
    }
}

impl Drop for ResourcesGuard {
    fn drop(&mut self) {
        unsafe { ffi::drmModeFreeResources(self.raw.as_ptr()) };
    }
}

struct ObjectPropertiesGuard {
    raw: NonNull<ffi::drmModeObjectProperties>,
}

impl ObjectPropertiesGuard {
    fn new(device: &KmsDevice, object_id: u32, object_type: u32) -> Result<Self, KmsError> {
        Ok(Self {
            raw: NonNull::new(unsafe {
                ffi::drmModeObjectGetProperties(device.fd().as_raw_fd(), object_id, object_type)
            })
            .ok_or_else(|| KmsError::new(KmsErrorKind::Native, "DRM property query failed"))?,
        })
    }
}

impl Drop for ObjectPropertiesGuard {
    fn drop(&mut self) {
        unsafe { ffi::drmModeFreeObjectProperties(self.raw.as_ptr()) };
    }
}

fn query_connector(device: &KmsDevice, id: u32) -> Result<Option<KmsConnector>, KmsError> {
    let Some(raw) = NonNull::new(unsafe { ffi::drmModeGetConnector(device.fd().as_raw_fd(), id) })
    else {
        return Ok(None);
    };
    struct Guard(NonNull<ffi::drmModeConnector>);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe { ffi::drmModeFreeConnector(self.0.as_ptr()) };
        }
    }
    let guard = Guard(raw);
    let connector = unsafe { guard.0.as_ref() };
    let modes = checked_slice(connector.modes, connector.count_modes)?;
    let encoders = checked_slice(connector.encoders, connector.count_encoders)?;
    let mut possible_crtcs_mask = 0_u32;
    for encoder in encoders {
        let Some(raw) =
            NonNull::new(unsafe { ffi::drmModeGetEncoder(device.fd().as_raw_fd(), *encoder) })
        else {
            continue;
        };
        possible_crtcs_mask |= unsafe { raw.as_ref() }.possible_crtcs;
        unsafe { ffi::drmModeFreeEncoder(raw.as_ptr()) };
    }
    Ok(Some(KmsConnector {
        id: KmsConnectorId::from_raw(connector.connector_id)
            .ok_or_else(|| KmsError::new(KmsErrorKind::Native, "DRM returned connector id zero"))?,
        connector_type: connector.connector_type,
        connector_type_id: connector.connector_type_id,
        status: match connector.connection {
            DRM_MODE_CONNECTED => ConnectorStatus::Connected,
            2 => ConnectorStatus::Disconnected,
            _ => ConnectorStatus::Unknown,
        },
        physical_millimeters: SizeI {
            width: connector.mm_width as i32,
            height: connector.mm_height as i32,
        },
        modes: modes
            .iter()
            .copied()
            .map(|native| KmsConnectorMode { native })
            .collect(),
        possible_encoders: encoders.to_vec(),
        possible_crtcs_mask,
    }))
}

fn query_planes(device: &KmsDevice) -> Result<Vec<KmsPlane>, KmsError> {
    let raw = NonNull::new(unsafe { ffi::drmModeGetPlaneResources(device.fd().as_raw_fd()) })
        .ok_or_else(|| KmsError::new(KmsErrorKind::Native, "DRM plane query failed"))?;
    struct Guard(NonNull<ffi::drmModePlaneRes>);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe { ffi::drmModeFreePlaneResources(self.0.as_ptr()) };
        }
    }
    let guard = Guard(raw);
    let native = unsafe { guard.0.as_ref() };
    if native.count_planes > 65_536 || (native.count_planes != 0 && native.planes.is_null()) {
        return Err(malformed());
    }
    let ids = checked_slice(native.planes, native.count_planes as i32)?;
    let mut planes = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(raw) = NonNull::new(unsafe { ffi::drmModeGetPlane(device.fd().as_raw_fd(), *id) })
        else {
            continue;
        };
        let plane = unsafe { raw.as_ref() };
        if plane.count_formats <= 65_536
            && (plane.count_formats == 0 || !plane.formats.is_null())
            && let Some(id) = KmsPlaneId::from_raw(plane.plane_id)
        {
            let formats = checked_slice(plane.formats, plane.count_formats as i32)?;
            planes.push(KmsPlane {
                id,
                possible_crtcs_mask: plane.possible_crtcs,
                formats: formats.to_vec(),
            });
        }
        unsafe { ffi::drmModeFreePlane(raw.as_ptr()) };
    }
    Ok(planes)
}

fn checked_slice<'a, T>(pointer: *const T, count: i32) -> Result<&'a [T], KmsError> {
    if !(0..=65_536).contains(&count) || (pointer.is_null() && count != 0) {
        return Err(malformed());
    }
    // Native DRM collections may use NULL for an empty array. Rust slices must
    // have a non-null, aligned pointer even when their length is zero.
    if count == 0 {
        return Ok(&[]);
    }
    Ok(unsafe { slice::from_raw_parts(pointer, count as usize) })
}

fn malformed() -> KmsError {
    KmsError::new(
        KmsErrorKind::Native,
        "DRM returned malformed collection metadata",
    )
}

#[cfg(test)]
mod tests {
    use super::checked_slice;

    #[test]
    fn empty_native_arrays_accept_null() {
        assert!(
            checked_slice::<u32>(std::ptr::null(), 0)
                .unwrap()
                .is_empty()
        );
        assert!(
            checked_slice::<u64>(std::ptr::null(), 0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn empty_native_arrays_accept_nonnull() {
        let values = [42_u32];
        assert!(checked_slice(values.as_ptr(), 0).unwrap().is_empty());
    }

    #[test]
    fn nonempty_native_arrays_preserve_values() {
        let values = [7_u32, 11];
        assert_eq!(checked_slice(values.as_ptr(), 2).unwrap(), &values);
    }

    #[test]
    fn invalid_native_array_metadata_is_rejected() {
        assert!(checked_slice::<u32>(std::ptr::null(), 1).is_err());
        assert!(checked_slice::<u32>(std::ptr::null(), -1).is_err());
        let value = 1_u32;
        assert!(checked_slice(&value, 65_537).is_err());
    }
}
