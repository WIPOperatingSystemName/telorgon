//! Pinned protocol-profile metadata. Runtime globals are advertised only after their handlers are
//! complete and registered by `telorgon-compositor-wayland`.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProtocolStage {
    Core,
    Stable,
    Staging,
    Unstable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InterfaceSpec {
    pub name: &'static str,
    pub source_version: u32,
    pub advertised_version: u32,
}

impl InterfaceSpec {
    pub const fn new(name: &'static str, source_version: u32, advertised_version: u32) -> Self {
        assert!(!name.is_empty(), "Wayland interface name must not be empty");
        assert!(source_version > 0, "Wayland source version must be nonzero");
        assert!(
            advertised_version > 0 && advertised_version <= source_version,
            "advertised Wayland version must be supported by the source"
        );
        Self {
            name,
            source_version,
            advertised_version,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProtocolSpec {
    pub name: &'static str,
    pub stage: ProtocolStage,
    pub source: &'static str,
    pub interfaces: &'static [InterfaceSpec],
}

pub const CORE_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("wl_display", 1, 1),
    InterfaceSpec::new("wl_registry", 1, 1),
    InterfaceSpec::new("wl_callback", 1, 1),
    InterfaceSpec::new("wl_compositor", 6, 6),
    InterfaceSpec::new("wl_surface", 6, 6),
    InterfaceSpec::new("wl_region", 1, 1),
    InterfaceSpec::new("wl_shm", 2, 2),
    InterfaceSpec::new("wl_shm_pool", 2, 2),
    InterfaceSpec::new("wl_buffer", 1, 1),
    InterfaceSpec::new("wl_subcompositor", 1, 1),
    InterfaceSpec::new("wl_subsurface", 1, 1),
    InterfaceSpec::new("wl_output", 4, 4),
    InterfaceSpec::new("wl_seat", 9, 9),
    InterfaceSpec::new("wl_pointer", 9, 9),
    InterfaceSpec::new("wl_keyboard", 9, 9),
    InterfaceSpec::new("wl_touch", 9, 9),
    InterfaceSpec::new("wl_data_device_manager", 3, 3),
    InterfaceSpec::new("wl_data_device", 3, 3),
    InterfaceSpec::new("wl_data_source", 3, 3),
    InterfaceSpec::new("wl_data_offer", 3, 3),
];

pub const XDG_DECORATION_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("zxdg_decoration_manager_v1", 1, 1),
    InterfaceSpec::new("zxdg_toplevel_decoration_v1", 1, 1),
];

pub const CURSOR_SHAPE_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("wp_cursor_shape_manager_v1", 2, 2),
    InterfaceSpec::new("wp_cursor_shape_device_v1", 2, 2),
];

pub const XDG_TOPLEVEL_ICON_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("xdg_toplevel_icon_manager_v1", 1, 1),
    InterfaceSpec::new("xdg_toplevel_icon_v1", 1, 1),
];

pub const FRACTIONAL_SCALE_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("wp_fractional_scale_manager_v1", 1, 1),
    InterfaceSpec::new("wp_fractional_scale_v1", 1, 1),
];

pub const RELATIVE_POINTER_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("zwp_relative_pointer_manager_v1", 1, 1),
    InterfaceSpec::new("zwp_relative_pointer_v1", 1, 1),
];

pub const POINTER_CONSTRAINTS_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("zwp_pointer_constraints_v1", 1, 1),
    InterfaceSpec::new("zwp_locked_pointer_v1", 1, 1),
    InterfaceSpec::new("zwp_confined_pointer_v1", 1, 1),
];

pub const XDG_OUTPUT_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("zxdg_output_manager_v1", 3, 3),
    InterfaceSpec::new("zxdg_output_v1", 3, 3),
];

pub const XWAYLAND_KEYBOARD_GRAB_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("zwp_xwayland_keyboard_grab_manager_v1", 1, 1),
    InterfaceSpec::new("zwp_xwayland_keyboard_grab_v1", 1, 1),
];

pub const SHORTCUT_INHIBIT_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("zwp_keyboard_shortcuts_inhibit_manager_v1", 1, 1),
    InterfaceSpec::new("zwp_keyboard_shortcuts_inhibitor_v1", 1, 1),
];

pub const IDLE_INHIBIT_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("zwp_idle_inhibit_manager_v1", 1, 1),
    InterfaceSpec::new("zwp_idle_inhibitor_v1", 1, 1),
];

pub const XDG_ACTIVATION_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("xdg_activation_v1", 1, 1),
    InterfaceSpec::new("xdg_activation_token_v1", 1, 1),
];

pub const SESSION_LOCK_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("ext_session_lock_manager_v1", 1, 1),
    InterfaceSpec::new("ext_session_lock_v1", 1, 1),
    InterfaceSpec::new("ext_session_lock_surface_v1", 1, 1),
];

pub const EXPLICIT_SYNC_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("zwp_linux_explicit_synchronization_v1", 2, 2),
    InterfaceSpec::new("zwp_linux_surface_synchronization_v1", 2, 2),
    InterfaceSpec::new("zwp_linux_buffer_release_v1", 1, 1),
];

pub const XDG_SHELL_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("xdg_wm_base", 7, 7),
    InterfaceSpec::new("xdg_positioner", 7, 7),
    InterfaceSpec::new("xdg_surface", 7, 7),
    InterfaceSpec::new("xdg_toplevel", 7, 7),
    InterfaceSpec::new("xdg_popup", 7, 7),
];

pub const VIEWPORTER_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("wp_viewporter", 1, 1),
    InterfaceSpec::new("wp_viewport", 1, 1),
];

pub const PRESENTATION_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("wp_presentation", 2, 2),
    InterfaceSpec::new("wp_presentation_feedback", 2, 2),
];

pub const LINUX_DMABUF_INTERFACES: &[InterfaceSpec] = &[
    InterfaceSpec::new("zwp_linux_dmabuf_v1", 5, 5),
    InterfaceSpec::new("zwp_linux_buffer_params_v1", 5, 5),
    InterfaceSpec::new("zwp_linux_dmabuf_feedback_v1", 5, 5),
];

pub const DESKTOP_PROTOCOLS: &[ProtocolSpec] = &[
    ProtocolSpec {
        name: "xwayland-shell-v1",
        stage: ProtocolStage::Staging,
        source: "staging/xwayland-shell/xwayland-shell-v1.xml",
        interfaces: &[
            InterfaceSpec::new("xwayland_shell_v1", 1, 1),
            InterfaceSpec::new("xwayland_surface_v1", 1, 1),
        ],
    },
    ProtocolSpec {
        name: "wayland",
        stage: ProtocolStage::Core,
        source: "wayland/src/wayland.xml",
        interfaces: CORE_INTERFACES,
    },
    ProtocolSpec {
        name: "xdg-shell",
        stage: ProtocolStage::Stable,
        source: "stable/xdg-shell/xdg-shell.xml",
        interfaces: XDG_SHELL_INTERFACES,
    },
    ProtocolSpec {
        name: "viewporter",
        stage: ProtocolStage::Stable,
        source: "stable/viewporter/viewporter.xml",
        interfaces: VIEWPORTER_INTERFACES,
    },
    ProtocolSpec {
        name: "presentation-time",
        stage: ProtocolStage::Stable,
        source: "stable/presentation-time/presentation-time.xml",
        interfaces: PRESENTATION_INTERFACES,
    },
    ProtocolSpec {
        name: "linux-dmabuf-v1",
        stage: ProtocolStage::Stable,
        source: "stable/linux-dmabuf/linux-dmabuf-v1.xml",
        interfaces: LINUX_DMABUF_INTERFACES,
    },
    ProtocolSpec {
        name: "xdg-decoration-unstable-v1",
        stage: ProtocolStage::Unstable,
        source: "unstable/xdg-decoration/xdg-decoration-unstable-v1.xml",
        interfaces: XDG_DECORATION_INTERFACES,
    },
    ProtocolSpec {
        name: "cursor-shape-v1",
        stage: ProtocolStage::Staging,
        source: "staging/cursor-shape/cursor-shape-v1.xml",
        interfaces: CURSOR_SHAPE_INTERFACES,
    },
    ProtocolSpec {
        name: "xdg-toplevel-icon-v1",
        stage: ProtocolStage::Staging,
        source: "staging/xdg-toplevel-icon/xdg-toplevel-icon-v1.xml",
        interfaces: XDG_TOPLEVEL_ICON_INTERFACES,
    },
    ProtocolSpec {
        name: "fractional-scale-v1",
        stage: ProtocolStage::Staging,
        source: "staging/fractional-scale/fractional-scale-v1.xml",
        interfaces: FRACTIONAL_SCALE_INTERFACES,
    },
    ProtocolSpec {
        name: "relative-pointer-unstable-v1",
        stage: ProtocolStage::Unstable,
        source: "unstable/relative-pointer/relative-pointer-unstable-v1.xml",
        interfaces: RELATIVE_POINTER_INTERFACES,
    },
    ProtocolSpec {
        name: "pointer-constraints-unstable-v1",
        stage: ProtocolStage::Unstable,
        source: "unstable/pointer-constraints/pointer-constraints-unstable-v1.xml",
        interfaces: POINTER_CONSTRAINTS_INTERFACES,
    },
    ProtocolSpec {
        name: "xdg-output-unstable-v1",
        stage: ProtocolStage::Unstable,
        source: "unstable/xdg-output/xdg-output-unstable-v1.xml",
        interfaces: XDG_OUTPUT_INTERFACES,
    },
    ProtocolSpec {
        name: "xwayland-keyboard-grab-unstable-v1",
        stage: ProtocolStage::Unstable,
        source: "unstable/xwayland-keyboard-grab/xwayland-keyboard-grab-unstable-v1.xml",
        interfaces: XWAYLAND_KEYBOARD_GRAB_INTERFACES,
    },
    ProtocolSpec {
        name: "keyboard-shortcuts-inhibit-unstable-v1",
        stage: ProtocolStage::Unstable,
        source: "unstable/keyboard-shortcuts-inhibit/keyboard-shortcuts-inhibit-unstable-v1.xml",
        interfaces: SHORTCUT_INHIBIT_INTERFACES,
    },
    ProtocolSpec {
        name: "idle-inhibit-unstable-v1",
        stage: ProtocolStage::Unstable,
        source: "unstable/idle-inhibit/idle-inhibit-unstable-v1.xml",
        interfaces: IDLE_INHIBIT_INTERFACES,
    },
    ProtocolSpec {
        name: "xdg-activation-v1",
        stage: ProtocolStage::Staging,
        source: "staging/xdg-activation/xdg-activation-v1.xml",
        interfaces: XDG_ACTIVATION_INTERFACES,
    },
    ProtocolSpec {
        name: "ext-session-lock-v1",
        stage: ProtocolStage::Staging,
        source: "staging/ext-session-lock/ext-session-lock-v1.xml",
        interfaces: SESSION_LOCK_INTERFACES,
    },
    ProtocolSpec {
        name: "linux-explicit-synchronization-unstable-v1",
        stage: ProtocolStage::Unstable,
        source: "unstable/linux-explicit-synchronization/linux-explicit-synchronization-unstable-v1.xml",
        interfaces: EXPLICIT_SYNC_INTERFACES,
    },
];

pub fn interface(name: &str) -> Option<InterfaceSpec> {
    DESKTOP_PROTOCOLS
        .iter()
        .flat_map(|protocol| protocol.interfaces.iter())
        .copied()
        .find(|interface| interface.name == name)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn protocol_profile_has_unique_bounded_interface_versions() {
        let mut names = BTreeSet::new();
        for protocol in DESKTOP_PROTOCOLS {
            assert!(!protocol.interfaces.is_empty());
            for interface in protocol.interfaces {
                assert!(names.insert(interface.name));
                assert!(interface.advertised_version <= interface.source_version);
            }
        }
    }

    #[test]
    fn interface_lookup_is_exact() {
        assert_eq!(interface("xdg_wm_base").unwrap().advertised_version, 7);
        assert_eq!(interface("wl_surface").unwrap().advertised_version, 6);
        assert!(interface("wl_shell").is_none());
    }

    #[test]
    fn profile_advertises_the_complete_toplevel_icon_pair() {
        assert_eq!(
            interface("xdg_toplevel_icon_manager_v1"),
            Some(InterfaceSpec::new("xdg_toplevel_icon_manager_v1", 1, 1))
        );
        assert_eq!(
            interface("xdg_toplevel_icon_v1"),
            Some(InterfaceSpec::new("xdg_toplevel_icon_v1", 1, 1))
        );
        let protocol = DESKTOP_PROTOCOLS
            .iter()
            .find(|protocol| protocol.name == "xdg-toplevel-icon-v1")
            .unwrap();
        assert_eq!(protocol.stage, ProtocolStage::Staging);
        assert_eq!(
            protocol.source,
            "staging/xdg-toplevel-icon/xdg-toplevel-icon-v1.xml"
        );
    }
}
