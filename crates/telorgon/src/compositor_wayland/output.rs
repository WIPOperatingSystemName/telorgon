use std::fmt;

use crate::core::{PointI, RectI, SizeI};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OutputTransform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputMode {
    pub size: SizeI,
    pub refresh_millihertz: u32,
    pub preferred: bool,
}

impl OutputMode {
    pub fn validate(self) -> Result<Self, OutputError> {
        if self.size.width <= 0 || self.size.height <= 0 || self.refresh_millihertz == 0 {
            return Err(OutputError::InvalidMode);
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputDescription {
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub physical_millimeters: SizeI,
    pub logical_position: PointI,
    /// Physical pixels per logical surface unit.
    pub scale: crate::platform::ScaleFactor,
    pub transform: OutputTransform,
    pub modes: Vec<OutputMode>,
}

impl OutputDescription {
    pub fn validate(self) -> Result<Self, OutputError> {
        if self.name.is_empty()
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || self.physical_millimeters.width < 0
            || self.physical_millimeters.height < 0
            || self.modes.is_empty()
            || self.modes.len() > 128
        {
            return Err(OutputError::InvalidDescription);
        }
        for mode in &self.modes {
            mode.validate()?;
        }
        if self.modes.iter().filter(|mode| mode.preferred).count() > 1 {
            return Err(OutputError::MultiplePreferredModes);
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputState {
    pub description: OutputDescription,
    pub current_mode: usize,
    pub enabled: bool,
}

impl OutputState {
    pub fn new(description: OutputDescription, current_mode: usize) -> Result<Self, OutputError> {
        let description = description.validate()?;
        if current_mode >= description.modes.len() {
            return Err(OutputError::UnknownMode);
        }
        Ok(Self {
            description,
            current_mode,
            enabled: true,
        })
    }

    pub fn set_mode(&mut self, mode: usize) -> Result<(), OutputError> {
        if mode >= self.description.modes.len() {
            return Err(OutputError::UnknownMode);
        }
        self.current_mode = mode;
        Ok(())
    }

    /// Logical output extent after the physical transform and fractional scale.
    pub fn logical_size(&self) -> SizeI {
        let mut size = self.current_mode().size;
        if matches!(
            self.description.transform,
            OutputTransform::Rotate90
                | OutputTransform::Rotate270
                | OutputTransform::Flipped90
                | OutputTransform::Flipped270
        ) {
            std::mem::swap(&mut size.width, &mut size.height);
        }
        self.description.scale.logical_size(size)
    }

    pub fn current_mode(&self) -> OutputMode {
        self.description.modes[self.current_mode]
    }
}

/// Detached, revisioned output state for consumers sharing one layout decision.
/// Produced by the native compositor's output registration/update APIs.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputLayoutSnapshot {
    revision: u64,
    outputs: std::collections::BTreeMap<u32, OutputState>,
}
impl OutputLayoutSnapshot {
    pub(crate) fn new(
        revision: u64,
        outputs: std::collections::BTreeMap<u32, OutputState>,
    ) -> Self {
        Self { revision, outputs }
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn outputs(&self) -> &std::collections::BTreeMap<u32, OutputState> {
        &self.outputs
    }
    /// Bounding root rectangle for enabled outputs, retaining this snapshot's revision.
    /// Empty layouts have no root. Arithmetic failure never produces a clipped layout.
    pub fn root_geometry(&self) -> Result<Option<OutputRootGeometry>, OutputError> {
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for output in self.outputs.values().filter(|output| output.enabled) {
            let mode = output
                .description
                .modes
                .get(output.current_mode)
                .ok_or(OutputError::UnknownMode)?
                .validate()?;
            // Match logical_size's f32 rounding, but reject its saturating cast boundary.
            for pixels in [mode.size.width, mode.size.height] {
                let logical = (pixels as f32 / output.description.scale.get()).ceil();
                if !logical.is_finite() || logical < 1.0 || f64::from(logical) > f64::from(i32::MAX)
                {
                    return Err(OutputError::GeometryOverflow);
                }
            }
            let size = output.logical_size();
            let PointI { x, y } = output.description.logical_position;
            let right = x
                .checked_add(size.width)
                .ok_or(OutputError::GeometryOverflow)?;
            let bottom = y
                .checked_add(size.height)
                .ok_or(OutputError::GeometryOverflow)?;
            bounds = Some(match bounds {
                None => (x, y, right, bottom),
                Some((left, top, r, b)) => (left.min(x), top.min(y), r.max(right), b.max(bottom)),
            });
        }
        bounds
            .map(|(x, y, right, bottom)| {
                Ok(OutputRootGeometry {
                    revision: self.revision,
                    bounds: RectI {
                        x,
                        y,
                        width: right.checked_sub(x).ok_or(OutputError::GeometryOverflow)?,
                        height: bottom.checked_sub(y).ok_or(OutputError::GeometryOverflow)?,
                    },
                })
            })
            .transpose()
    }
}

/// Root normalization derived from one immutable layout, not a RandR monitor ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputRootGeometry {
    revision: u64,
    bounds: RectI,
}
impl OutputRootGeometry {
    pub fn revision(self) -> u64 {
        self.revision
    }
    pub fn desktop_bounds(self) -> RectI {
        self.bounds
    }
    /// Convert once at the protocol boundary. Offscreen coordinates remain offscreen.
    pub fn desktop_to_root(self, point: PointI) -> Result<PointI, OutputError> {
        Ok(PointI {
            x: point
                .x
                .checked_sub(self.bounds.x)
                .ok_or(OutputError::GeometryOverflow)?,
            y: point
                .y
                .checked_sub(self.bounds.y)
                .ok_or(OutputError::GeometryOverflow)?,
        })
    }
    pub fn root_to_desktop(self, point: PointI) -> Result<PointI, OutputError> {
        Ok(PointI {
            x: point
                .x
                .checked_add(self.bounds.x)
                .ok_or(OutputError::GeometryOverflow)?,
            y: point
                .y
                .checked_add(self.bounds.y)
                .ok_or(OutputError::GeometryOverflow)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputError {
    InvalidMode,
    InvalidDescription,
    MultiplePreferredModes,
    UnknownMode,
    GeometryOverflow,
}

impl fmt::Display for OutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Wayland output validation failed: {self:?}")
    }
}

impl std::error::Error for OutputError {}

#[cfg(test)]
mod root_geometry_tests {
    use super::*;
    fn output(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        scale: f32,
        transform: OutputTransform,
    ) -> OutputState {
        OutputState::new(
            OutputDescription {
                name: "test".into(),
                description: String::new(),
                make: String::new(),
                model: String::new(),
                physical_millimeters: SizeI::default(),
                logical_position: PointI { x, y },
                scale: crate::platform::ScaleFactor::new(scale).unwrap(),
                transform,
                modes: vec![OutputMode {
                    size: SizeI { width, height },
                    refresh_millihertz: 60000,
                    preferred: true,
                }],
            },
            0,
        )
        .unwrap()
    }
    fn layout() -> OutputLayoutSnapshot {
        OutputLayoutSnapshot::new(
            42,
            [
                (1, output(-800, 0, 800, 600, 1.0, OutputTransform::Normal)),
                (
                    2,
                    output(0, -300, 1920, 1080, 1.5, OutputTransform::Rotate90),
                ),
            ]
            .into(),
        )
    }
    #[test]
    fn negative_rotated_fractional_layout_roundtrips_without_clamping() {
        let root = layout().root_geometry().unwrap().unwrap();
        assert_eq!(root.revision(), 42);
        assert_eq!(
            root.desktop_bounds(),
            RectI {
                x: -800,
                y: -300,
                width: 1520,
                height: 1280
            }
        );
        assert_eq!(
            root.desktop_to_root(PointI::default()).unwrap(),
            PointI { x: 800, y: 300 }
        );
        for point in [
            PointI { x: -900, y: -500 },
            PointI::default(),
            PointI { x: 720, y: 980 },
        ] {
            assert_eq!(
                root.root_to_desktop(root.desktop_to_root(point).unwrap())
                    .unwrap(),
                point
            );
        }
        assert!(root.desktop_to_root(PointI { x: i32::MAX, y: 0 }).is_err());
        assert!(root.root_to_desktop(PointI { x: i32::MIN, y: 0 }).is_err());
    }
    #[test]
    fn disabled_empty_invalid_and_overflow_layouts() {
        assert_eq!(
            OutputLayoutSnapshot::new(0, Default::default()).root_geometry(),
            Ok(None)
        );
        let mut snapshot = layout();
        snapshot.outputs.get_mut(&2).unwrap().enabled = false;
        assert_eq!(
            snapshot
                .root_geometry()
                .unwrap()
                .unwrap()
                .desktop_bounds()
                .width,
            800
        );
        snapshot.outputs.get_mut(&1).unwrap().current_mode = 10;
        assert_eq!(snapshot.root_geometry(), Err(OutputError::UnknownMode));
        for (x, width, scale) in [
            (i32::MAX, 1, 1.0),
            (0, 100, f32::MIN_POSITIVE),
            (0, i32::MAX, 1.0),
        ] {
            let bad = OutputLayoutSnapshot::new(
                1,
                [(1, output(x, 0, width, 1, scale, OutputTransform::Normal))].into(),
            );
            assert_eq!(bad.root_geometry(), Err(OutputError::GeometryOverflow));
        }
        let wide = OutputLayoutSnapshot::new(
            1,
            [
                (1, output(i32::MIN, 0, 1, 1, 1.0, OutputTransform::Normal)),
                (2, output(0, 0, 1, 1, 1.0, OutputTransform::Normal)),
            ]
            .into(),
        );
        assert_eq!(wide.root_geometry(), Err(OutputError::GeometryOverflow));
    }
    #[cfg(feature = "desktop-xwayland")]
    #[test]
    fn x11_root_geometry_rejects_wire_overflow_and_preserves_dimensions() {
        use crate::xwayland::window::Geometry;
        let root = layout().root_geometry().unwrap().unwrap();
        let rect = RectI {
            x: -900,
            y: -500,
            width: 65535,
            height: 600,
        };
        let wire = Geometry::from_desktop(root, rect, 2).unwrap();
        assert_eq!((wire.x, wire.y, wire.border), (-100, -200, 2));
        assert_eq!(wire.desktop_rect(root).unwrap(), rect);
        for invalid in [
            RectI {
                x: 32768 - 800,
                ..rect
            },
            RectI {
                y: -32769 - 300,
                ..rect
            },
            RectI {
                width: 65536,
                ..rect
            },
            RectI { height: 0, ..rect },
            RectI { width: -1, ..rect },
        ] {
            assert!(Geometry::from_desktop(root, invalid, 0).is_err());
        }
        let boundary = RectI {
            x: 32767 - 800,
            y: -32768 - 300,
            width: 1,
            height: 1,
        };
        assert_eq!(
            Geometry::from_desktop(root, boundary, 0)
                .unwrap()
                .desktop_rect(root)
                .unwrap(),
            boundary
        );
    }
}
