#[test]
fn neutral_desktop_modules_do_not_depend_on_a_renderer() {
    let modules = [
        include_str!("client.rs"),
        include_str!("layers.rs"),
        include_str!("pointer_visual.rs"),
        include_str!("scene.rs"),
        include_str!("state.rs"),
        include_str!("geometry.rs"),
    ];
    for source in modules {
        for forbidden in [
            "renderer_software",
            "renderer_vulkan",
            "SoftwareRenderer",
            "SoftwareScene",
            "SoftwareSurface",
            "VulkanDevice",
            "VulkanScene",
        ] {
            assert!(
                !source.contains(forbidden),
                "neutral desktop module contains backend-specific symbol {forbidden}"
            );
        }
    }
}

#[test]
fn desktop_backend_assemblies_do_not_cross_reference() {
    let software = include_str!("renderer/software.rs");
    for forbidden in ["renderer_vulkan", "VulkanDevice", "VulkanScene", "vk::"] {
        assert!(
            !software.contains(forbidden),
            "software desktop assembly contains Vulkan symbol {forbidden}"
        );
    }

    let vulkan = include_str!("renderer/vulkan.rs");
    for forbidden in [
        "renderer_software",
        "SoftwareRenderer",
        "SoftwareScene",
        "SoftwareSurface",
        "software.raster",
    ] {
        assert!(
            !vulkan.contains(forbidden),
            "Vulkan desktop assembly contains software-renderer symbol {forbidden}"
        );
    }
}
