//! Environment-owned installed-application metadata and asynchronous icon cache.
#[cfg(test)]
use super::SignalSnapshot;
use super::{Signal, context};
use crate::{assets::ImageSource, render::ImageResource, ui::ImageId};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApplicationId(String);
impl ApplicationId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplicationIcon(pub String);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationVisibility {
    Visible,
    HiddenFromLauncher,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationMetadata {
    pub id: ApplicationId,
    pub name: String,
    pub generic_name: Option<String>,
    pub description: Option<String>,
    pub keywords: Vec<String>,
    pub categories: Vec<String>,
    pub icon: Option<ApplicationIcon>,
    pub visibility: ApplicationVisibility,
    pub startup_wm_class: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationCatalogStatus {
    Loading,
    Ready,
    Failed(String),
}
#[derive(Clone, Debug)]
pub struct ApplicationQuery {
    text: String,
    limit: usize,
}
impl ApplicationQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            limit: 20,
        }
    }
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit.min(1000);
        self
    }
}
/// Requested icon raster size. The default scale comes from the mounted output.
#[derive(Clone, Copy, Debug)]
pub struct IconRequest {
    logical_size: u32,
    scale: Option<crate::platform::contracts::ScaleFactor>,
}
impl Default for IconRequest {
    fn default() -> Self {
        Self::new()
    }
}
impl IconRequest {
    pub const fn new() -> Self {
        Self {
            logical_size: 32,
            scale: None,
        }
    }
    pub fn logical_size(mut self, size: u32) -> Self {
        self.logical_size = size.clamp(1, 256);
        self
    }
    pub fn scale(mut self, scale: crate::platform::contracts::ScaleFactor) -> Self {
        self.scale = Some(scale);
        self
    }
}
/// Configuration only: no worker starts until the shell environment runs.
#[derive(Clone, Debug)]
pub struct ApplicationCatalog {
    roots: Vec<PathBuf>,
    locale: String,
    desktops: Vec<String>,
    path: Vec<PathBuf>,
    theme: String,
    watch: bool,
    memory: Option<Vec<ApplicationMetadata>>,
}
impl Default for ApplicationCatalog {
    fn default() -> Self {
        Self::system()
    }
}
impl ApplicationCatalog {
    pub fn system() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut roots = Vec::new();
        let user = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|p| p.join(".local/share")));
        if let Some(user) = user {
            roots.push(user);
        }
        let system = std::env::var_os("XDG_DATA_DIRS")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        roots.extend(std::env::split_paths(&system).filter(|p| p.is_absolute()));
        Self {
            roots,
            locale: ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .find_map(|key| std::env::var(key).ok().filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "C".into()),
            desktops: std::env::var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .map(str::to_owned)
                .collect(),
            path: std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect(),
            theme: "hicolor".into(),
            watch: true,
            memory: None,
        }
    }
    pub fn in_memory(applications: Vec<ApplicationMetadata>) -> Self {
        Self {
            memory: Some(applications),
            watch: false,
            ..Self::system()
        }
    }
    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = locale.into().replace('-', "_");
        self
    }
    pub fn watch_changes(mut self, watch: bool) -> Self {
        self.watch = watch;
        self
    }
    pub fn icon_theme(mut self, theme: impl Into<String>) -> Self {
        self.theme = theme.into();
        self
    }
    /// Ordered XDG data roots; primarily useful for isolated shells and tests.
    pub fn data_directories(mut self, roots: Vec<PathBuf>) -> Self {
        self.roots = roots.into_iter().filter(|p| p.is_absolute()).collect();
        self
    }
}
#[derive(Clone, Debug, PartialEq)]
struct CatalogSnapshot {
    status: ApplicationCatalogStatus,
    apps: Vec<ApplicationMetadata>,
}
#[derive(Clone)]
pub struct ApplicationCatalogHandle {
    state: Signal<CatalogSnapshot>,
    icons: Signal<BTreeMap<(String, u32), ImageResource>>,
    requests: mpsc::SyncSender<(String, u32)>,
    requested: Arc<std::sync::Mutex<BTreeSet<(String, u32)>>>,
    raster_size: u32,
}
impl ApplicationCatalogHandle {
    pub fn status(&self) -> ApplicationCatalogStatus {
        context::observe(&self.state).status.clone()
    }
    pub fn installed(&self) -> Vec<ApplicationMetadata> {
        context::observe(&self.state)
            .apps
            .iter()
            .filter(|a| a.visibility == ApplicationVisibility::Visible)
            .cloned()
            .collect()
    }
    pub fn get(&self, id: &ApplicationId) -> Option<ApplicationMetadata> {
        context::observe(&self.state)
            .apps
            .iter()
            .find(|a| &a.id == id)
            .cloned()
    }
    pub fn search(&self, query: ApplicationQuery) -> Vec<ApplicationMetadata> {
        let q = query.text.trim().to_lowercase();
        let mut values: Vec<_> = self
            .installed()
            .into_iter()
            .filter_map(|app| {
                let name = app.name.to_lowercase();
                let rank = if name == q {
                    0
                } else if name.starts_with(&q) {
                    1
                } else if name.contains(&q) {
                    2
                } else if app
                    .keywords
                    .iter()
                    .chain(app.generic_name.iter())
                    .chain(app.description.iter())
                    .any(|s| s.to_lowercase().contains(&q))
                {
                    3
                } else {
                    return None;
                };
                Some((rank, name, app))
            })
            .collect();
        values.sort_by(|a, b| (&a.0, &a.1, &a.2.id).cmp(&(&b.0, &b.1, &b.2.id)));
        values
            .into_iter()
            .take(query.limit)
            .map(|(_, _, a)| a)
            .collect()
    }
    /// Match protocol identity, never the human-readable window title. Ambiguous aliases remain unmatched.
    pub fn identify(&self, identity: &str) -> Option<ApplicationId> {
        if identity.is_empty() {
            return None;
        }
        let state = context::observe(&self.state);
        let id = if identity.ends_with(".desktop") {
            identity.to_owned()
        } else {
            format!("{identity}.desktop")
        };
        if let Some(app) = state.apps.iter().find(|a| a.id.0 == id) {
            return Some(app.id.clone());
        }
        let mut matches = state.apps.iter().filter(|a| {
            a.startup_wm_class.as_deref() == Some(identity) || a.id.0.eq_ignore_ascii_case(&id)
        });
        let first = matches.next()?;
        matches.next().is_none().then(|| first.id.clone())
    }
    pub fn icon(&self, id: &ApplicationId) -> ImageSource {
        self.get(id).and_then(|a| a.icon).map_or_else(
            || context::bind_image(fallback_image()),
            |i| self.icon_named(&i.0),
        )
    }
    /// Returns available artwork without substituting the generic icon. Pending lookups
    /// still subscribe the view to asynchronous completion, so fallback chains remain reactive.
    pub fn try_resolve_icon(&self, id: &ApplicationId, request: IconRequest) -> Option<ImageSource> {
        let icon = self.resolve_icon(id, request);
        (icon.image_id() != fallback_image().image).then_some(icon)
    }
    pub fn try_resolve_named_icon(&self, name: &str, request: IconRequest) -> Option<ImageSource> {
        let icon = self.resolve_named_icon(name, request);
        (icon.image_id() != fallback_image().image).then_some(icon)
    }
    pub fn resolve_icon(&self, id: &ApplicationId, request: IconRequest) -> ImageSource {
        self.get(id).and_then(|a| a.icon).map_or_else(
            || context::bind_image(fallback_image()),
            |icon| self.resolve_named_icon(&icon.0, request),
        )
    }
    pub fn resolve_named_icon(&self, name: &str, request: IconRequest) -> ImageSource {
        let scale = request
            .scale
            .map_or(self.raster_size as f32 / 32.0, |s| s.get());
        let size = (request.logical_size as f32 * scale)
            .ceil()
            .clamp(1.0, 256.0) as u32;
        self.icon_named_at(name, size)
    }
    pub(crate) fn icon_named(&self, name: &str) -> ImageSource {
        self.icon_named_at(name, self.raster_size)
    }
    fn icon_named_at(&self, name: &str, size: u32) -> ImageSource {
        let key = (name.to_owned(), size);
        let icons = context::observe(&self.icons);
        if let Some(icon) = icons.get(&key) {
            return context::bind_image(icon.clone());
        }
        let mut requested = self.requested.lock().unwrap();
        if requested.len() < 512
            && !requested.contains(&key)
            && self.requests.try_send(key.clone()).is_ok()
        {
            requested.insert(key);
        }
        context::bind_image(fallback_image())
    }
    #[cfg(test)]
    pub(crate) fn resources(&self) -> SignalSnapshot<BTreeMap<(String, u32), ImageResource>> {
        self.icons.snapshot()
    }
}
pub(crate) struct CatalogWorker {
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for CatalogWorker {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub(crate) fn start(
    config: ApplicationCatalog,
    raster_size: u32,
) -> (ApplicationCatalogHandle, CatalogWorker) {
    let initial = config.memory.clone();
    let (state, writer) = Signal::new(CatalogSnapshot {
        status: if initial.is_some() {
            ApplicationCatalogStatus::Ready
        } else {
            ApplicationCatalogStatus::Loading
        },
        apps: initial.unwrap_or_default(),
    });
    let (icons, icon_writer) = Signal::new(BTreeMap::new());
    let (requests, receive) = mpsc::sync_channel::<(String, u32)>(128);
    let (stop, stopped) = mpsc::channel();
    let failed = writer.clone();
    let thread = std::thread::Builder::new()
        .name("telorgon-applications".into())
        .spawn(move || {
            let mut next_scan = std::time::Instant::now();
            let mut last_apps = Vec::new();
            let mut cache = BTreeMap::new();
            let mut fingerprints = BTreeMap::new();
            let mut icon_requests = BTreeSet::new();
            let mut next_icons = std::time::Instant::now();
            loop {
                if stopped.try_recv().is_ok() {
                    break;
                }
                if config.memory.is_none() && std::time::Instant::now() >= next_scan {
                    match discover(&config) {
                        Ok(apps) => {
                            last_apps = apps.clone();
                            writer.publish_if_changed(CatalogSnapshot {
                                status: ApplicationCatalogStatus::Ready,
                                apps,
                            });
                        }
                        Err(error) => {
                            writer.publish_if_changed(CatalogSnapshot {
                                status: ApplicationCatalogStatus::Failed(error),
                                apps: last_apps.clone(),
                            });
                        }
                    }
                    next_scan = std::time::Instant::now()
                        + if config.watch {
                            Duration::from_secs(2)
                        } else {
                            Duration::from_secs(315360000)
                        };
                }
                if config.watch && std::time::Instant::now() >= next_icons {
                    for key in &icon_requests {
                        refresh_icon(&config, key, &mut fingerprints, &mut cache);
                    }
                    icon_writer.publish_if_changed(cache.clone());
                    next_icons = std::time::Instant::now() + Duration::from_secs(2);
                }
                match receive.recv_timeout(Duration::from_millis(50)) {
                    Ok((name, size)) => {
                        let key = (name, size);
                        icon_requests.insert(key.clone());
                        refresh_icon(&config, &key, &mut fingerprints, &mut cache);
                        // Also wake subscribers after a failed lookup: callers whose bounded
                        // request queue was full can now retry admission.
                        icon_writer.publish(cache.clone());
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(_) => {}
                }
            }
        });
    let thread = match thread {
        Ok(t) => Some(t),
        Err(e) => {
            failed.publish(CatalogSnapshot {
                status: ApplicationCatalogStatus::Failed(e.to_string()),
                apps: Vec::new(),
            });
            None
        }
    };
    (
        ApplicationCatalogHandle {
            state,
            icons,
            requests,
            requested: Arc::new(std::sync::Mutex::new(BTreeSet::new())),
            raster_size: raster_size.clamp(16, 256),
        },
        CatalogWorker { stop, thread },
    )
}

fn values(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut value = String::new();
    let mut escape = false;
    for c in text.chars() {
        if escape {
            value.push(match c {
                's' => ' ',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other,
            });
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else if c == ';' {
            result.push(std::mem::take(&mut value));
        } else {
            value.push(c);
        }
    }
    if !value.is_empty() {
        result.push(value);
    }
    result
}
fn string(text: &str) -> String {
    // Semicolons delimit lists only, not ordinary strings.
    let mut result = String::new();
    let mut escape = false;
    for c in text.chars() {
        if escape {
            result.push(match c {
                's' => ' ',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                v => v,
            });
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else {
            result.push(c);
        }
    }
    result
}
fn read_ini(path: &Path) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    file.take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 1024 * 1024 {
        return Err("metadata exceeds 1 MiB".into());
    }
    let mut groups = BTreeMap::<String, BTreeMap<String, String>>::new();
    let mut group = String::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            group = line[1..line.len() - 1].to_owned();
        } else if let Some((key, value)) = line.split_once('=') {
            groups
                .entry(group.clone())
                .or_default()
                .insert(key.trim().into(), value.trim().into());
        }
    }
    Ok(groups)
}
fn localized(fields: &BTreeMap<String, String>, key: &str, locale: &str) -> Option<String> {
    let (base, modifier) = locale
        .split_once('@')
        .map_or((locale, None), |(b, m)| (b, Some(m)));
    let base = base.split('.').next().unwrap_or(base);
    let lang = base.split('_').next().unwrap_or(base);
    let mut candidates = Vec::new();
    if let Some(m) = modifier {
        candidates.push(format!("{base}@{m}"));
    }
    candidates.push(base.into());
    if let Some(m) = modifier {
        candidates.push(format!("{lang}@{m}"));
    }
    candidates.push(lang.into());
    candidates
        .iter()
        .find_map(|l| fields.get(&format!("{key}[{l}]")))
        .or_else(|| fields.get(key))
        .cloned()
}
fn executable(value: &str, paths: &[PathBuf]) -> bool {
    let check = |p: &Path| {
        let Ok(m) = std::fs::metadata(p) else {
            return false;
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            m.is_file() && m.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            m.is_file()
        }
    };
    let p = Path::new(value);
    if p.is_absolute() {
        check(p)
    } else {
        !value.contains('/') && paths.iter().any(|p| check(&p.join(value)))
    }
}
fn discover(config: &ApplicationCatalog) -> Result<Vec<ApplicationMetadata>, String> {
    let mut seen = BTreeSet::new();
    let mut apps = Vec::new();
    for root in &config.roots {
        let base = root.join("applications");
        let mut pending = vec![(base.clone(), 0)];
        while let Some((dir, depth)) = pending.pop() {
            if depth > 16 {
                continue;
            }
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
            };
            let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    pending.push((path, depth + 1));
                    continue;
                }
                if path.extension().is_none_or(|e| e != "desktop") {
                    continue;
                }
                let id = path
                    .strip_prefix(&base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('/', "-");
                if !seen.insert(id.clone()) {
                    continue;
                }
                if seen.len() > 16384 {
                    return Err("application catalog exceeds 16384 entries".into());
                }
                let Ok(mut groups) = read_ini(&path) else {
                    continue;
                };
                let Some(fields) = groups.remove("Desktop Entry") else {
                    continue;
                };
                if fields.get("Hidden").is_some_and(|v| v == "true") {
                    continue;
                }
                if fields.get("Type").is_none_or(|v| v != "Application") {
                    continue;
                }
                let Some(name) = localized(&fields, "Name", &config.locale)
                    .map(|v| string(&v))
                    .filter(|n| !n.is_empty())
                else {
                    continue;
                };
                let only = fields.get("OnlyShowIn").map(|v| values(v));
                let not = fields
                    .get("NotShowIn")
                    .map(|v| values(v))
                    .unwrap_or_default();
                let visible = fields.get("NoDisplay").is_none_or(|v| v != "true")
                    && only.is_none_or(|v| v.iter().any(|d| config.desktops.contains(d)))
                    && !not.iter().any(|d| config.desktops.contains(d))
                    && fields
                        .get("TryExec")
                        .is_none_or(|v| executable(&string(v), &config.path))
                    && (fields.get("Exec").is_some_and(|v| !v.is_empty())
                        || fields.get("DBusActivatable").is_some_and(|v| v == "true"));
                apps.push(ApplicationMetadata {
                    id: ApplicationId(id),
                    name,
                    generic_name: localized(&fields, "GenericName", &config.locale)
                        .map(|v| string(&v)),
                    description: localized(&fields, "Comment", &config.locale).map(|v| string(&v)),
                    keywords: localized(&fields, "Keywords", &config.locale)
                        .map(|v| values(&v))
                        .unwrap_or_default(),
                    categories: fields
                        .get("Categories")
                        .map(|v| values(v))
                        .unwrap_or_default(),
                    icon: localized(&fields, "Icon", &config.locale)
                        .filter(|v| !v.is_empty())
                        .map(|v| ApplicationIcon(string(&v))),
                    startup_wm_class: fields.get("StartupWMClass").map(|v| string(v)),
                    visibility: if visible {
                        ApplicationVisibility::Visible
                    } else {
                        ApplicationVisibility::HiddenFromLauncher
                    },
                });
            }
        }
    }
    apps.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    Ok(apps)
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}
fn find_icon(config: &ApplicationCatalog, name: &str, size: u32) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_owned());
    }
    if !safe_name(name) {
        return None;
    }
    let mut bases = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        bases.push(PathBuf::from(home).join(".icons"));
    }
    bases.extend(config.roots.iter().map(|p| p.join("icons")));
    fn theme(
        bases: &[PathBuf],
        theme_name: &str,
        name: &str,
        size: u32,
        visited: &mut BTreeSet<String>,
    ) -> Option<PathBuf> {
        if !safe_name(theme_name) || visited.len() >= 32 || !visited.insert(theme_name.into()) {
            return None;
        }
        let groups = bases
            .iter()
            .find_map(|p| read_ini(&p.join(theme_name).join("index.theme")).ok())?;
        let header = groups.get("Icon Theme")?;
        let directories = header
            .get("Directories")
            .into_iter()
            .chain(header.get("ScaledDirectories"))
            .flat_map(|v| v.split(','));
        let mut best: Option<(u32, PathBuf)> = None;
        for directory in directories {
            let relative = Path::new(directory);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|p| !matches!(p, std::path::Component::Normal(_)))
            {
                continue;
            }
            let Some(fields) = groups.get(directory) else {
                continue;
            };
            let number = |key: &str, default: u32| {
                fields
                    .get(key)
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(default)
                    .min(4096)
            };
            let scale = number("Scale", 1).max(1);
            let nominal = number("Size", size);
            let (min, max) = match fields
                .get("Type")
                .map(String::as_str)
                .unwrap_or("Threshold")
            {
                "Scalable" => (number("MinSize", nominal), number("MaxSize", nominal)),
                "Fixed" => (nominal, nominal),
                _ => {
                    let threshold = number("Threshold", 2);
                    (
                        nominal.saturating_sub(threshold),
                        nominal.saturating_add(threshold),
                    )
                }
            };
            let min = min.saturating_mul(scale);
            let max = max.max(min / scale).saturating_mul(scale);
            let distance = if size < min {
                min - size
            } else {
                size.saturating_sub(max)
            };
            for base in bases {
                for ext in ["png", "svg"] {
                    let file = base
                        .join(theme_name)
                        .join(directory)
                        .join(format!("{name}.{ext}"));
                    if file.is_file() && best.as_ref().is_none_or(|b| distance < b.0) {
                        best = Some((distance, file));
                    }
                }
            }
        }
        if let Some((_, path)) = best {
            return Some(path);
        }
        for parent in header
            .get("Inherits")
            .into_iter()
            .flat_map(|v| v.split(','))
        {
            if let Some(path) = theme(bases, parent.trim(), name, size, visited) {
                return Some(path);
            }
        }
        None
    }
    let mut visited = BTreeSet::new();
    if let Some(path) = theme(&bases, &config.theme, name, size, &mut visited)
        .or_else(|| theme(&bases, "hicolor", name, size, &mut visited))
    {
        return Some(path);
    }
    for base in bases
        .into_iter()
        .chain(config.roots.iter().map(|p| p.join("pixmaps")))
    {
        for ext in ["png", "svg"] {
            let path = base.join(format!("{name}.{ext}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}
pub(crate) static NEXT_ICON: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0x6800_0000);
pub(crate) fn decode_icon(path: &Path, size: u32) -> Result<ImageResource, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("icon exceeds 4 MiB".into());
    }
    let (pixels, alpha) = if path.extension().is_some_and(|e| e == "svg") {
        let options = resvg::usvg::Options {
            resources_dir: None,
            image_href_resolver: resvg::usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..Default::default()
        };
        let tree = resvg::usvg::Tree::from_data(&bytes, &options).map_err(|e| e.to_string())?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or("invalid icon size")?;
        let intrinsic = tree.size();
        let scale = (size as f32 / intrinsic.width()).min(size as f32 / intrinsic.height());
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(
            (size as f32 - intrinsic.width() * scale) / 2.0,
            (size as f32 - intrinsic.height() * scale) / 2.0,
        );
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        (pixmap.take(), crate::graphics::render::ImageAlphaMode::Premultiplied)
    } else {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|e| e.to_string())?
            .resize(size, size, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let mut canvas = image::RgbaImage::new(size, size);
        image::imageops::overlay(
            &mut canvas,
            &decoded,
            ((size - decoded.width()) / 2).into(),
            ((size - decoded.height()) / 2).into(),
        );
        (canvas.into_raw(), crate::graphics::render::ImageAlphaMode::Straight)
    };
    Ok(ImageResource {
        image: ImageId(NEXT_ICON.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
        content_version: 1,
        extent: crate::SizeI {
            width: size as i32,
            height: size as i32,
        },
        color_encoding: crate::graphics::render::ImageColorEncoding::Srgb,
        alpha_mode: alpha,
        pixel_format: crate::graphics::render::ImagePixelFormat::Rgba8,
        pixels: pixels.into(),
    })
}
pub(crate) fn fallback_image() -> ImageResource {
    static IMAGE: std::sync::OnceLock<ImageResource> = std::sync::OnceLock::new();
    IMAGE
        .get_or_init(|| {
            let mut pixels = vec![0u8; 32 * 32 * 4];
            for y in 5..27 {
                for x in 3..29 {
                    let i = (y * 32 + x) * 4;
                    pixels[i..i + 4].copy_from_slice(if y < 10 {
                        &[110, 165, 240, 255]
                    } else {
                        &[195, 207, 224, 255]
                    });
                }
            }
            ImageResource {
                image: ImageId(0x6800_0000 - 1),
                content_version: 1,
                extent: crate::SizeI {
                    width: 32,
                    height: 32,
                },
                color_encoding: crate::graphics::render::ImageColorEncoding::Srgb,
                alpha_mode: crate::graphics::render::ImageAlphaMode::Straight,
                pixel_format: crate::graphics::render::ImagePixelFormat::Rgba8,
                pixels: pixels.into(),
            }
        })
        .clone()
}

fn refresh_icon(
    config: &ApplicationCatalog,
    key: &(String, u32),
    fingerprints: &mut BTreeMap<(String, u32), (PathBuf, Option<std::time::SystemTime>, u64)>,
    cache: &mut BTreeMap<(String, u32), ImageResource>,
) {
    let found = find_icon(config, &key.0, key.1).and_then(|path| {
        std::fs::metadata(&path)
            .ok()
            .map(|m| (path, m.modified().ok(), m.len()))
    });
    match found {
        Some(stamp) if fingerprints.get(key) != Some(&stamp) => {
            cache.remove(key);
            if let Ok(resource) = decode_icon(&stamp.0, key.1) {
                cache.insert(key.clone(), resource);
            }
            fingerprints.insert(key.clone(), stamp);
        }
        None => {
            fingerprints.remove(key);
            cache.remove(key);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;

impl From<ApplicationIcon> for ImageSource {
    fn from(icon: ApplicationIcon) -> Self {
        if icon.0.is_empty() {
            return context::bind_image(fallback_image());
        }
        context::provided::<super::ShellContext>()
            .expect("ApplicationIcon requires a mounted ShellContext")
            .applications()
            .icon_named(&icon.0)
    }
}
