use crate::ui::layout::LayoutDiagnostics;
use crate::graphics::render::CompileStats;
use crate::theme::ThemeRuntimeDiagnostics;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameDiagnostics {
    pub layout: LayoutDiagnostics,
    pub compile: CompileStats,
    pub theme: ThemeRuntimeDiagnostics,
    pub delta_queue_high_water: usize,
}
