//! Generation-safe immutable theme scopes and atomic replacement.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::foundation::MonotonicInstant;
use crate::ui::{ComponentStyleId, MountedUi, ThemeScopeId};

use crate::theme::{
    CompiledComponentStyle, CompiledTheme, MotionPreference, ThemeCatalog, ThemeDomain, ThemeError,
    ThemeResult, ThemeRuntimeDiagnostics, ThemeScope, ThemeScopeKind, ThemeSource, ThemeUpdate,
    application_catalog, shell_catalog,
};

pub const TELORGON_APPLICATION_THEME_V4: &str = r##"
format = "v4"
domain = "application"
[tokens.color]
button_hovered = "#424a5cff"
button_pressed = "#2a303dff"
button_disabled = "#2b2e37b4"
text_active = "#ffffffff"
text_busy = "#d6d9e1ff"
text_disabled = "#999da8ff"
focus = "#a6dbffff"
invalid = "#ff6b78ff"
[tokens.length]
outline = 2
[tokens.duration]
fast = 120
[tokens.easing]
standard = "ease-out"
[components.button.default]
transition = { duration = { token = "duration.fast" }, easing = { token = "easing.standard" } }
[components.button.default.states.hovered.slots.root]
background = { token = "color.button_hovered" }
[components.button.default.states.pressed.slots.root]
background = { token = "color.button_pressed" }
[components.button.default.states.busy.slots.root]
opacity = 0.82
[components.button.default.states.disabled.slots.root]
background = { token = "color.button_disabled" }
[components.button.default.states.focus-visible.slots.root]
outline_color = { token = "color.focus" }
outline_width = { token = "length.outline" }
outline_offset = 2
[components.button.default.states.invalid.slots.root]
outline_color = { token = "color.invalid" }
outline_width = { token = "length.outline" }
[components.text.default]
transition = { duration = { token = "duration.fast" }, easing = { token = "easing.standard" } }
[components.text.default.states.pressed.slots.root]
foreground = { token = "color.text_active" }
[components.text.default.states.busy.slots.root]
foreground = { token = "color.text_busy" }
[components.text.default.states.disabled.slots.root]
foreground = { token = "color.text_disabled" }
[components.activity-indicator.default.states.busy]
transition = { duration = 900, easing = "linear", repeat = true }
[components.activity-indicator.default.states.busy.slots.marker]
rotation = 6.2831855
[components.toggle.default.states.focus-visible.slots.root]
outline_color = { token = "color.focus" }
outline_width = { token = "length.outline" }
outline_offset = 2
[components.slider.default.states.focus-visible.slots.root]
outline_color = { token = "color.focus" }
outline_width = { token = "length.outline" }
outline_offset = 2
[components.text-input.default.states.focus-visible.slots.root]
outline_color = { token = "color.focus" }
outline_width = { token = "length.outline" }
outline_offset = 2
"##;

pub const TELORGON_SHELL_THEME_V4: &str = r##"
format = "v4"
domain = "shell"
[tokens.color]
button_hovered = "#424a5cff"
button_pressed = "#2a303dff"
text_disabled = "#999da8ff"
focus = "#a6dbffff"
[tokens.length]
outline = 2
[tokens.duration]
fast = 120
[tokens.easing]
standard = "ease-out"
[components.button.default]
transition = { duration = { token = "duration.fast" }, easing = { token = "easing.standard" } }
[components.button.default.states.hovered.slots.root]
background = { token = "color.button_hovered" }
[components.button.default.states.pressed.slots.root]
background = { token = "color.button_pressed" }
[components.button.default.states.focus-visible.slots.root]
outline_color = { token = "color.focus" }
outline_width = { token = "length.outline" }
outline_offset = 2
[components.text.default.states.disabled.slots.root]
foreground = { token = "color.text_disabled" }
"##;

#[derive(Clone, Debug)]
pub(crate) struct RegisteredTheme {
    pub scope: ThemeScope,
    pub theme: Arc<CompiledTheme>,
    pub style_revisions: BTreeMap<ComponentStyleId, u64>,
}

#[derive(Clone, Debug)]
struct ScopeSlot {
    generation: u32,
    registered: Option<RegisteredTheme>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeReplacement {
    pub changed_styles: Vec<ComponentStyleId>,
}

#[derive(Clone, Debug)]
pub struct ThemeRuntime {
    scopes: Vec<ScopeSlot>,
    free_scopes: Vec<u32>,
    pub(crate) runtime_revision: u64,
    pub(crate) processor: crate::theme::processor::StyleProcessor,
    pending_invalidated: u64,
    pub(crate) pending_changed_styles: BTreeSet<ComponentStyleId>,
}

impl Default for ThemeRuntime {
    fn default() -> Self {
        let application_catalog = application_catalog();
        let shell_catalog = shell_catalog();
        let application = CompiledTheme::compile(
            &ThemeSource::parse(TELORGON_APPLICATION_THEME_V4)
                .expect("built-in application Theme v4 source must parse"),
            &application_catalog,
        )
        .expect("empty application Theme v4 must compile");
        let shell = CompiledTheme::compile(
            &ThemeSource::parse(TELORGON_SHELL_THEME_V4)
                .expect("built-in shell Theme v4 source must parse"),
            &shell_catalog,
        )
        .expect("empty shell Theme v4 must compile");
        Self::new(application, shell).expect("built-in Theme v4 roots are valid")
    }
}

impl ThemeRuntime {
    pub const APPLICATION_SCOPE: ThemeScopeId = ThemeScopeId::new(0, 1);
    pub const SHELL_SCOPE: ThemeScopeId = ThemeScopeId::new(1, 1);

    pub fn new(application: CompiledTheme, shell: CompiledTheme) -> ThemeResult<Self> {
        if application.domain() != ThemeDomain::Application {
            return Err(ThemeError::new(
                "application root requires an application theme",
            ));
        }
        if shell.domain() != ThemeDomain::Shell {
            return Err(ThemeError::new("shell root requires a shell theme"));
        }
        let application_scope = Self::root_scope(ThemeDomain::Application);
        let shell_scope = Self::root_scope(ThemeDomain::Shell);
        Ok(Self {
            scopes: vec![
                ScopeSlot {
                    generation: 1,
                    registered: Some(registered(application_scope, application)),
                },
                ScopeSlot {
                    generation: 1,
                    registered: Some(registered(shell_scope, shell)),
                },
            ],
            free_scopes: Vec::new(),
            runtime_revision: 1,
            processor: crate::theme::processor::StyleProcessor::default(),
            pending_invalidated: 0,
            pending_changed_styles: BTreeSet::new(),
        })
    }

    pub const fn root_scope(domain: ThemeDomain) -> ThemeScope {
        match domain {
            ThemeDomain::Application => ThemeScope::new(
                Self::APPLICATION_SCOPE,
                ThemeDomain::Application,
                ThemeScopeKind::Root,
            ),
            ThemeDomain::Shell => {
                ThemeScope::new(Self::SHELL_SCOPE, ThemeDomain::Shell, ThemeScopeKind::Root)
            }
        }
    }

    /// Atomically swaps one immutable snapshot after all validation has already succeeded.
    pub fn replace_theme(
        &mut self,
        scope: ThemeScope,
        replacement: CompiledTheme,
    ) -> ThemeResult<ThemeReplacement> {
        if replacement.domain() != scope.domain() {
            return Err(ThemeError::new(
                "replacement theme domain does not match its scope",
            ));
        }
        let slot = self
            .scopes
            .get_mut(scope.id().index() as usize)
            .ok_or_else(|| ThemeError::new("stale theme scope"))?;
        if slot.generation != scope.id().generation() {
            return Err(ThemeError::new("stale theme scope generation"));
        }
        let registered = slot
            .registered
            .as_mut()
            .filter(|registered| registered.scope == scope)
            .ok_or_else(|| ThemeError::new("discarded theme scope"))?;
        let changed_styles = registered.theme.changed_style_ids(&replacement);
        if changed_styles.is_empty() {
            return Ok(ThemeReplacement::default());
        }
        for id in &changed_styles {
            let revision = registered.style_revisions.entry(*id).or_insert(1);
            *revision = revision.wrapping_add(1).max(1);
        }
        registered.theme = Arc::new(replacement);
        self.runtime_revision = self.runtime_revision.wrapping_add(1).max(1);
        self.pending_invalidated = self
            .pending_invalidated
            .saturating_add(changed_styles.len() as u64);
        self.pending_changed_styles
            .extend(changed_styles.iter().copied());
        Ok(ThemeReplacement { changed_styles })
    }

    pub fn compile_and_replace(
        &mut self,
        scope: ThemeScope,
        source: &ThemeSource,
        catalog: &ThemeCatalog,
    ) -> ThemeResult<ThemeReplacement> {
        let compiled = CompiledTheme::compile(source, catalog)?;
        self.replace_theme(scope, compiled)
    }

    pub fn create_preview(&mut self, theme: CompiledTheme) -> ThemeResult<ThemeScope> {
        let domain = theme.domain();
        let index = self.free_scopes.pop().unwrap_or(self.scopes.len() as u32);
        let generation = if let Some(slot) = self.scopes.get(index as usize) {
            slot.generation
        } else {
            1
        };
        let scope = ThemeScope::new(
            ThemeScopeId::new(index, generation),
            domain,
            ThemeScopeKind::Preview,
        );
        let slot = ScopeSlot {
            generation,
            registered: Some(registered(scope, theme)),
        };
        if index as usize == self.scopes.len() {
            self.scopes.push(slot);
        } else {
            self.scopes[index as usize] = slot;
        }
        self.runtime_revision = self.runtime_revision.wrapping_add(1).max(1);
        Ok(scope)
    }

    pub fn compile_preview(
        &mut self,
        source: &ThemeSource,
        catalog: &ThemeCatalog,
    ) -> ThemeResult<ThemeScope> {
        self.create_preview(CompiledTheme::compile(source, catalog)?)
    }

    pub fn discard_preview(&mut self, scope: ThemeScope) -> bool {
        if scope.kind() != ThemeScopeKind::Preview {
            return false;
        }
        let Some(slot) = self.scopes.get_mut(scope.id().index() as usize) else {
            return false;
        };
        if slot.generation != scope.id().generation()
            || slot
                .registered
                .as_ref()
                .is_none_or(|registered| registered.scope != scope)
        {
            return false;
        }
        slot.registered = None;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free_scopes.push(scope.id().index());
        self.runtime_revision = self.runtime_revision.wrapping_add(1).max(1);
        true
    }

    pub fn theme(&self, scope: ThemeScope) -> Option<&CompiledTheme> {
        Some(self.registered(scope)?.theme.as_ref())
    }

    pub fn resolve_in(
        &self,
        scope: ThemeScope,
        style: ComponentStyleId,
    ) -> Option<&CompiledComponentStyle> {
        self.theme(scope)?.style(style)
    }

    pub(crate) fn registered(&self, scope: ThemeScope) -> Option<&RegisteredTheme> {
        let slot = self.scopes.get(scope.id().index() as usize)?;
        if slot.generation != scope.id().generation() {
            return None;
        }
        slot.registered
            .as_ref()
            .filter(|registered| registered.scope == scope)
    }

    pub(crate) fn scope_from_id(&self, id: ThemeScopeId) -> Option<ThemeScope> {
        let slot = self.scopes.get(id.index() as usize)?;
        if slot.generation != id.generation() {
            return None;
        }
        slot.registered.as_ref().map(|registered| registered.scope)
    }

    pub(crate) fn resolve_binding_style(
        &self,
        scope: ThemeScopeId,
        style: ComponentStyleId,
    ) -> Option<(&CompiledComponentStyle, u64)> {
        let scope = self.scope_from_id(scope)?;
        let registered = self.registered(scope)?;
        let revision = registered.style_revisions.get(&style).copied().unwrap_or(0);
        Some((registered.theme.style(style)?, revision))
    }

    /// Resolves changed bindings, retargets tracks, samples motion, and applies atomic slot patches.
    pub fn update_styles(
        &mut self,
        ui: &mut MountedUi,
        now: MonotonicInstant,
        preference: MotionPreference,
    ) -> ThemeUpdate {
        #[cfg(feature = "instrumentation")]
        let _span = crate::runtime::instrumentation::span!("theme.resolve");
        let mut processor = std::mem::take(&mut self.processor);
        let mut update = processor.update(self, ui, now, preference);
        processor.diagnostics.entries_invalidated = processor
            .diagnostics
            .entries_invalidated
            .saturating_add(self.pending_invalidated);
        self.pending_invalidated = 0;
        self.pending_changed_styles.clear();
        update.diagnostics = processor.diagnostics;
        #[cfg(feature = "instrumentation")]
        {
            crate::runtime::instrumentation::counter!(
                "theme.bindings.evaluated",
                update.diagnostics.bindings_evaluated
            );
            crate::runtime::instrumentation::counter!(
                "theme.bindings.skipped",
                update.diagnostics.bindings_skipped
            );
            crate::runtime::instrumentation::counter!(
                "theme.entries.invalidated",
                update.diagnostics.entries_invalidated
            );
            crate::runtime::instrumentation::counter!(
                "theme.animations.active",
                update.diagnostics.active_animations
            );
            crate::runtime::instrumentation::counter!("theme.retargets", update.diagnostics.retargets);
        }
        self.processor = processor;
        update
    }

    pub fn diagnostics(&self) -> ThemeRuntimeDiagnostics {
        let mut diagnostics = self.processor.diagnostics;
        diagnostics.entries_invalidated = diagnostics
            .entries_invalidated
            .saturating_add(self.pending_invalidated);
        diagnostics
    }
}

fn registered(scope: ThemeScope, theme: CompiledTheme) -> RegisteredTheme {
    let style_revisions = theme.styles.keys().map(|id| (*id, 1)).collect();
    RegisteredTheme {
        scope,
        theme: Arc::new(theme),
        style_revisions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ComponentStyleContract, StylePropertyMask};
    use crate::ui::StylePropertyPatch;

    fn catalog(domain: ThemeDomain) -> ThemeCatalog {
        let mut catalog = ThemeCatalog::new(domain);
        catalog
            .register(
                ComponentStyleContract::new("button")
                    .slot("root", StylePropertyMask::ALL)
                    .style(
                        "default",
                        [("root".to_owned(), StylePropertyPatch::default())],
                    ),
            )
            .unwrap();
        catalog
    }

    fn compile(domain: ThemeDomain, color: &str) -> CompiledTheme {
        let source = ThemeSource::parse(&format!(
            "format='v4'\ndomain='{}'\n[components.button.default.slots.root]\nbackground='{color}'",
            domain.name()
        ))
        .unwrap();
        CompiledTheme::compile(&source, &catalog(domain)).unwrap()
    }

    #[test]
    fn replacement_is_atomic_and_reports_only_changed_stable_ids() {
        let mut runtime = ThemeRuntime::new(
            compile(ThemeDomain::Application, "#112233ff"),
            compile(ThemeDomain::Shell, "#223344ff"),
        )
        .unwrap();
        let scope = ThemeRuntime::root_scope(ThemeDomain::Application);
        let replacement = runtime
            .replace_theme(scope, compile(ThemeDomain::Application, "#334455ff"))
            .unwrap();
        assert_eq!(replacement.changed_styles.len(), 1);
        assert!(
            runtime
                .theme(scope)
                .unwrap()
                .style_id("button", "default")
                .is_some()
        );
        assert!(
            runtime
                .replace_theme(scope, compile(ThemeDomain::Shell, "#000000ff"))
                .is_err()
        );
    }

    #[test]
    fn discarded_preview_ids_never_alias_reused_slots() {
        let mut runtime = ThemeRuntime::default();
        let preview = runtime
            .create_preview(compile(ThemeDomain::Application, "#112233ff"))
            .unwrap();
        assert!(runtime.discard_preview(preview));
        let replacement = runtime
            .create_preview(compile(ThemeDomain::Application, "#334455ff"))
            .unwrap();
        assert_eq!(preview.id().index(), replacement.id().index());
        assert_ne!(preview.id().generation(), replacement.id().generation());
        assert!(runtime.theme(preview).is_none());
        assert!(runtime.theme(replacement).is_some());
    }
}
