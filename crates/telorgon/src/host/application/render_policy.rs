/// Renderer policy selected by an application declaration.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Renderer {
    /// Uses the entrypoint's platform default renderer policy.
    #[default]
    Auto,
    /// Requires the Vulkan renderer.
    Vulkan,
    /// Requires the deterministic software renderer.
    Software,
}
