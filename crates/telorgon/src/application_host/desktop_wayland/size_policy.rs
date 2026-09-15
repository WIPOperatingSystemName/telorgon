//! Shared logical sizing policy. Protocol adapters keep their wire constraints.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct SizePolicy {
    pub preferred: SizeI,
    pub available: SizeI,
    pub minimum: Option<SizeI>,
    pub maximum: Option<SizeI>,
}
impl Default for SizePolicy {
    fn default() -> Self {
        Self {
            preferred: SizeI {
                width: 1,
                height: 1,
            },
            available: SizeI {
                width: 65535,
                height: 65535,
            },
            minimum: None,
            maximum: None,
        }
    }
}
impl SizePolicy {
    pub fn resolve(self, wanted: SizeI) -> SizeI {
        fn axis(wanted: i32, preferred: i32, available: i32, minimum: i32, maximum: i32) -> i32 {
            let mut lo = minimum.max(1);
            let mut hi = if maximum <= 0 { i32::MAX } else { maximum };
            if lo > hi {
                lo = 1;
                hi = i32::MAX;
            } // discard contradictory limits
            // A fixed/minimum-size client can exceed the work area, but its titlebar remains reachable.
            hi = hi.min(available.max(1)).max(lo);
            let preferred = preferred.max(lo).min(hi);
            wanted.clamp(preferred, hi)
        }
        let min = self.minimum.unwrap_or_default();
        let max = self.maximum.unwrap_or_default();
        SizeI {
            width: axis(
                wanted.width,
                self.preferred.width,
                self.available.width,
                min.width,
                max.width,
            ),
            height: axis(
                wanted.height,
                self.preferred.height,
                self.available.height,
                min.height,
                max.height,
            ),
        }
    }
}

pub(super) fn apply(
    wayland: &NativeCompositor<'_>,
    windows: &mut BTreeMap<WaylandSurfaceId, ClientWindow>,
    scheduler: &mut ConfigureScheduler,
    config: &LinuxDesktopConfig,
    area: RectI,
) {
    for (surface, window) in windows {
        if window.backend.is_none() || window.maximized || window.fullscreen {
            continue;
        }
        let outer = if window_has_frame(window) {
            window
                .chrome_outer
                .unwrap_or_else(|| legacy_window_outer(window, config))
        } else {
            window.requested_size
        };
        let overhead = SizeI {
            width: (outer.width - window.requested_size.width).max(0),
            height: (outer.height - window.requested_size.height).max(0),
        };
        window.size_policy.preferred = config.preferred_window_minimum;
        window.size_policy.available = SizeI {
            width: (area.width - overhead.width).max(1),
            height: (area.height - overhead.height).max(1),
        };
        keep_reachable(window, area, config.titlebar_height);
        if window.backend != Some(WindowBackend::Wayland) {
            continue;
        }
        if let Some(metadata) = wayland.toplevel_metadata(*surface) {
            window.size_policy.minimum = metadata.minimum_size;
            window.size_policy.maximum = metadata.maximum_size;
        }
        let size = window.size_policy.resolve(window.requested_size);
        if size == window.requested_size || window.last_policy_request == Some(size) {
            continue;
        }
        window.requested_size = size;
        if let Some(anchor) = window.native_configure.resize_anchor {
            window.position = anchor.reconcile_position(window.position, size);
        }
        keep_reachable(window, area, config.titlebar_height);
        if window.native_configure.resize_anchor.is_none()
            || window.native_configure.resize_final.is_some()
        {
            scheduler.schedule_final(*surface, size);
            window.last_policy_request = Some(size);
        }
    }
}

fn keep_reachable(window: &mut ClientWindow, area: RectI, titlebar: i32) {
    let grip = window
        .requested_size
        .width
        .clamp(1, 32)
        .min(area.width.max(1));
    window.position.x = window.position.x.clamp(
        area.x
            .saturating_sub(window.requested_size.width)
            .saturating_add(grip),
        area.x.saturating_add(area.width).saturating_sub(grip),
    );
    window.position.y = window.position.y.clamp(
        area.y,
        area.y
            .saturating_add(area.height.saturating_sub(titlebar.max(1)).max(0)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_client_declining_the_same_preference_is_not_reconfigured_forever() {
        let display = Display::new().unwrap();
        let wayland = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
        let surface = WaylandSurfaceId::from_raw(1).unwrap();
        let small = SizeI {
            width: 100,
            height: 80,
        };
        let mut windows = BTreeMap::from([(
            surface,
            super::super::client::maximize_preview_tests::test_window(small, PointI::default()),
        )]);
        let mut scheduler = ConfigureScheduler::default();
        let config = LinuxDesktopConfig::default();
        let area = RectI {
            x: 0,
            y: 0,
            width: 1000,
            height: 700,
        };
        apply(&wayland, &mut windows, &mut scheduler, &config, area);
        assert_eq!(
            windows[&surface].requested_size,
            config.preferred_window_minimum
        );
        windows.get_mut(&surface).unwrap().requested_size = small;
        apply(&wayland, &mut windows, &mut scheduler, &config, area);
        assert_eq!(windows[&surface].requested_size, small);
    }
    #[test]
    fn preferred_minimum_respects_client_maximum_fixed_size_and_work_area() {
        let mut p = SizePolicy {
            preferred: SizeI {
                width: 300,
                height: 200,
            },
            ..Default::default()
        };
        assert_eq!(
            p.resolve(SizeI {
                width: 100,
                height: 100
            }),
            p.preferred
        );
        p.maximum = Some(SizeI {
            width: 180,
            height: 90,
        });
        assert_eq!(
            p.resolve(SizeI {
                width: 100,
                height: 100
            }),
            p.maximum.unwrap()
        );
        p.minimum = p.maximum;
        assert_eq!(
            p.resolve(SizeI {
                width: 1000,
                height: 1000
            }),
            p.maximum.unwrap()
        );
        p.minimum = None;
        p.maximum = None;
        p.available = SizeI {
            width: 120,
            height: 80,
        };
        assert_eq!(
            p.resolve(SizeI {
                width: 1,
                height: 1
            }),
            p.available
        );
        p.minimum = Some(SizeI {
            width: 400,
            height: 300,
        });
        p.maximum = Some(SizeI {
            width: 100,
            height: 50,
        });
        assert_eq!(
            p.resolve(SizeI {
                width: 1,
                height: 1
            }),
            p.available
        );
    }
}
