use super::{
    ApplicationLaunch, ApplicationRequest, Command, Error, Result, SessionHandle, SessionPhase,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplicationSpec {
    Executable {
        program: PathBuf,
        arguments: Vec<OsString>,
    },
    DesktopEntry(String, Vec<OsString>),
}
impl ApplicationSpec {
    pub fn executable(program: impl Into<PathBuf>) -> Self {
        Self::Executable {
            program: program.into(),
            arguments: Vec::new(),
        }
    }
    pub fn desktop_entry(id: impl Into<String>) -> Self {
        Self::DesktopEntry(id.into(), Vec::new())
    }
    pub fn arg(mut self, argument: impl Into<OsString>) -> Self {
        match &mut self {
            Self::Executable { arguments, .. } | Self::DesktopEntry(_, arguments) => {
                arguments.push(argument.into())
            }
        }
        self
    }
}

/// Explicit directories take precedence over optional XDG and PATH discovery.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplicationRegistry {
    entries: Vec<(String, ApplicationSpec)>,
    desktops: Vec<PathBuf>,
    binaries: Vec<PathBuf>,
    xdg: bool,
    path: bool,
}
impl ApplicationRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn discover_xdg_applications(mut self) -> Self {
        self.xdg = true;
        self
    }
    pub fn discover_path_executables(mut self) -> Self {
        self.path = true;
        self
    }
    pub fn desktop_entry_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.desktops.push(directory.into());
        self
    }
    pub fn binary_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.binaries.push(directory.into());
        self
    }
    pub fn register(mut self, key: impl Into<String>, application: ApplicationSpec) -> Self {
        self.entries.push((key.into(), application));
        self
    }
    pub(super) fn validate(&self) -> Result<()> {
        for (i, (key, _)) in self.entries.iter().enumerate() {
            if key.is_empty() || self.entries[..i].iter().any(|(other, _)| other == key) {
                return Err(Error::Invalid(format!(
                    "empty or duplicate application key: {key:?}"
                )));
            }
        }
        if self
            .desktops
            .iter()
            .chain(&self.binaries)
            .any(|p| !p.is_absolute())
        {
            return Err(Error::Invalid(
                "application search directories must be absolute".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum ApplicationRef {
    Registered(String),
    Resolved(ApplicationHandle),
}
impl ApplicationRef {
    pub fn registered(key: impl Into<String>) -> Self {
        Self::Registered(key.into())
    }
    pub(crate) fn resolve(&self) -> Result<ApplicationHandle> {
        match self {
            Self::Registered(key) => super::applications()?.get(key),
            Self::Resolved(handle) => {
                handle.ready()?;
                Ok(handle.clone())
            }
        }
    }
}
impl From<ApplicationHandle> for ApplicationRef {
    fn from(handle: ApplicationHandle) -> Self {
        Self::Resolved(handle)
    }
}

#[derive(Clone)]
pub struct SessionApplications {
    session: SessionHandle,
}
impl SessionApplications {
    pub(super) fn new(session: SessionHandle) -> Self {
        Self { session }
    }
    pub fn get(&self, key: &str) -> Result<ApplicationHandle> {
        let spec = self
            .session
            .config()
            .applications
            .entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, spec)| spec.clone())
            .ok_or_else(|| Error::Invalid(format!("unregistered application: {key:?}")))?;
        self.resolve(spec)
    }
    pub fn executable(&self, program: impl Into<PathBuf>) -> Result<ApplicationHandle> {
        self.resolve(ApplicationSpec::executable(program))
    }
    pub fn desktop_entry(&self, id: impl Into<String>) -> Result<ApplicationHandle> {
        self.resolve(ApplicationSpec::desktop_entry(id))
    }
    fn resolve(&self, mut spec: ApplicationSpec) -> Result<ApplicationHandle> {
        if self.session.phase() != SessionPhase::Ready {
            return Err(Error::Closing);
        }
        let desktop = match &mut spec {
            ApplicationSpec::DesktopEntry(id, _) => Some(self.desktop_path(id)?),
            ApplicationSpec::Executable { program, .. } => {
                let registry = &self.session.config().applications;
                let mut searched = Vec::new();
                if program.is_absolute() {
                    searched.push(program.clone());
                } else if program.components().count() != 1 {
                    return Err(Error::Invalid(
                        "executable must be an absolute path or a binary name".into(),
                    ));
                } else {
                    searched.extend(registry.binaries.iter().map(|dir| dir.join(&*program)));
                    if registry.path {
                        if let Some(path) = self.session.env().get("PATH") {
                            searched.extend(
                                std::env::split_paths(path)
                                    .filter(|p| p.is_absolute())
                                    .map(|p| p.join(&*program)),
                            );
                        }
                    }
                }
                *program = searched
                    .iter()
                    .find(|p| super::desktop::executable(p.as_os_str(), self.session.env()))
                    .cloned()
                    .ok_or_else(|| {
                        Error::Invalid(format!(
                            "executable {program:?} not found; searched {searched:?}"
                        ))
                    })?;
                None
            }
        };
        Ok(ApplicationHandle {
            session: self.session.clone(),
            spec,
            desktop,
        })
    }
    pub(super) fn desktop_path(&self, id: &str) -> Result<PathBuf> {
        if !id.ends_with(".desktop") || id == ".desktop" || id.contains(['/', '\\']) {
            return Err(Error::Invalid(
                "expected a desktop-file ID, not a path".into(),
            ));
        }
        let registry = &self.session.config().applications;
        for dir in &registry.desktops {
            let direct = dir.join(id);
            if direct.is_file() {
                return Ok(direct);
            }
            if let Some(path) = super::desktop::find_nested(dir, dir, id, 0, &mut 8192)? {
                return Ok(path);
            }
        }
        if registry.xdg {
            return super::desktop::resolve_id(id, self.session.env());
        }
        Err(Error::Invalid(format!(
            "desktop entry {id:?} not found; searched {:?}; XDG discovery disabled",
            registry.desktops
        )))
    }
}

#[derive(Clone)]
pub struct ApplicationHandle {
    session: SessionHandle,
    spec: ApplicationSpec,
    desktop: Option<PathBuf>,
}
impl std::fmt::Debug for ApplicationHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplicationHandle")
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}
impl ApplicationHandle {
    pub(crate) fn ready(&self) -> Result<()> {
        if self.session.phase() == SessionPhase::Ready {
            Ok(())
        } else {
            Err(Error::Closing)
        }
    }
    pub fn command(&self) -> Result<Command> {
        self.ready()?;
        let (program, arguments) = self.executable_parts()?;
        Ok(self.session.command(program).args(arguments))
    }
    pub fn request(&self) -> Result<ApplicationRequest> {
        self.ready()?;
        let ApplicationSpec::DesktopEntry(id, arguments) = &self.spec else {
            return Err(Error::Invalid(
                "desktop requests require a desktop entry".into(),
            ));
        };
        let mut request = self.session.application(id).prefer_dbus(true);
        request.path = self.desktop.clone();
        request.extra_args = arguments.clone();
        Ok(request)
    }
    pub async fn launch(&self) -> Result<ApplicationLaunch> {
        match &self.spec {
            ApplicationSpec::DesktopEntry(_, _) => self.request()?.launch().await,
            ApplicationSpec::Executable { .. } => Ok(ApplicationLaunch {
                children: vec![self.command()?.spawn()?],
                errors: Vec::new(),
                activated: None,
            }),
        }
    }
    pub(crate) fn executable_parts(&self) -> Result<(&Path, &[OsString])> {
        match &self.spec {
            ApplicationSpec::Executable { program, arguments } => Ok((program, arguments)),
            _ => Err(Error::Unsupported("a portal picker requires a direct executable with private stdio; desktop activation is not supported".into())),
        }
    }
    pub(crate) fn spawn_helper(
        &self,
        command: &mut std::process::Command,
    ) -> Result<std::process::Child> {
        self.session.spawn_helper(command)
    }
}
