use crate::{
    authoring::compose::{Signal, SignalWriter},
    input::*,
    tray::*,
};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrayMenuNavigationState {
    pub path: Vec<i32>,
    pub highlighted: Option<i32>,
    pub level: usize,
}
#[derive(Clone)]
pub struct TrayMenuNavigation {
    signal: Signal<TrayMenuNavigationState>,
    writer: SignalWriter<TrayMenuNavigationState>,
}
impl PartialEq for TrayMenuNavigation {
    fn eq(&self, other: &Self) -> bool {
        self.signal == other.signal
    }
}
impl Default for TrayMenuNavigation {
    fn default() -> Self {
        let (signal, writer) = Signal::new(TrayMenuNavigationState::default());
        Self { signal, writer }
    }
}
impl TrayMenuNavigation {
    pub fn signal(&self) -> Signal<TrayMenuNavigationState> {
        self.signal.clone()
    }
    pub fn levels<'a>(&self, menu: &'a TrayMenu) -> Vec<&'a [TrayMenuItem]> {
        let state = self.signal.snapshot();
        let mut levels = vec![menu.items.as_slice()];
        for id in &state.path {
            if let Some(item) = levels
                .last()
                .unwrap()
                .iter()
                .find(|i| i.id == *id && i.visible && i.enabled)
            {
                levels.push(&item.children);
            } else {
                break;
            }
        }
        levels
    }
    pub fn highlight(&self, id: i32, level: usize) {
        let mut s = (*self.signal.snapshot()).clone();
        s.highlighted = Some(id);
        s.level = level;
        self.writer.publish_if_changed(s);
    }
    pub fn activate(&self, host: &TrayHandle, id: i32, level: usize) -> Result<(), TrayError> {
        let snapshot = host.snapshot();
        let menu = snapshot.menu.ok_or(TrayError::Unavailable)?;
        let levels = self.levels(&menu.menu);
        let item = levels
            .get(level)
            .and_then(|items| items.iter().find(|i| i.id == id))
            .filter(|i| i.enabled && i.visible && !i.separator)
            .ok_or(TrayError::Unavailable)?;
        if item.is_submenu || !item.children.is_empty() {
            let mut s = (*self.signal.snapshot()).clone();
            s.path.truncate(level);
            s.path.push(id);
            s.level = level + 1;
            s.highlighted = None;
            self.writer.publish_if_changed(s);
            host.prepare_submenu(menu.owner, id)
        } else {
            host.select_menu_item(menu.owner, menu.revision, id)
        }
    }
    pub fn key(&self, host: &TrayHandle, event: &KeyEvent) -> bool {
        if event.state != ButtonState::Pressed {
            return false;
        }
        let Some(code) = event.physical_key.code() else {
            return false;
        };
        use PhysicalKeyCode::*;
        let Some(menu) = host.snapshot().menu else {
            return false;
        };
        let levels = self.levels(&menu.menu);
        let mut s = (*self.signal.snapshot()).clone();
        let level = s.level.min(levels.len() - 1);
        if code == ArrowLeft {
            if !s.path.is_empty() {
                s.highlighted = s.path.pop();
                s.level = s.path.len();
                self.writer.publish_if_changed(s);
            }
            return true;
        }
        if matches!(code, Enter | Space | ArrowRight) {
            let id = s.highlighted.or_else(|| {
                levels[level]
                    .iter()
                    .find(|i| i.visible && i.enabled && !i.separator)
                    .map(|i| i.id)
            });
            if let Some(id) = id {
                if code == ArrowRight
                    && !levels[level]
                        .iter()
                        .any(|i| i.id == id && (i.is_submenu || !i.children.is_empty()))
                {
                    return true;
                }
                let _ = self.activate(host, id, level);
            }
            return true;
        }
        let command = match code {
            ArrowUp => CompositeNavigationCommand::Up,
            ArrowDown => CompositeNavigationCommand::Down,
            Home => CompositeNavigationCommand::Home,
            End => CompositeNavigationCommand::End,
            _ => return false,
        };
        let mut navigation =
            CompositeStateMachine::new(crate::components::application::menu_navigation_policy());
        let _ = navigation.update_items(
            levels[level]
                .iter()
                .filter(|i| i.visible && !i.separator)
                .map(|i| CompositeItem {
                    key: i.id,
                    enabled: i.enabled,
                }),
        );
        let _ = navigation.enter(s.highlighted);
        if s.highlighted.is_some() || matches!(code, Home | End) {
            let _ = navigation.navigate(command, WritingDirection::LeftToRight);
        }
        s.highlighted = navigation.active_descendant();
        s.level = level;
        self.writer.publish_if_changed(s);
        true
    }
}
