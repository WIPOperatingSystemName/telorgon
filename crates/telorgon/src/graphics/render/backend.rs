use crate::graphics::render::{RenderRequest, RenderResult, RenderSceneDelta, RenderStats, SceneUpdateStats};

/// Backend execution over one device and any number of independent retained scenes.
///
/// Rendering records or executes work into a caller-provided frame and target. Submission and
/// presentation are deliberately outside this contract.
pub trait RenderBackend {
    type Scene;
    type FrameContext<'frame>
    where
        Self: 'frame;
    type Target<'frame>
    where
        Self: 'frame;

    fn create_scene(&self) -> RenderResult<Self::Scene>;

    fn apply_scene_delta(
        &self,
        scene: &mut Self::Scene,
        delta: &RenderSceneDelta,
    ) -> RenderResult<SceneUpdateStats>;

    fn render<'frame>(
        &self,
        scene: &mut Self::Scene,
        frame: &mut Self::FrameContext<'frame>,
        target: &Self::Target<'frame>,
        request: &RenderRequest,
    ) -> RenderResult<RenderStats>;
}
