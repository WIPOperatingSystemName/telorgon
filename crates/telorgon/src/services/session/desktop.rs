use super::{Command, Environment, Error, ManagedChild, RestartPolicy, Result, SessionHandle};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// A desktop launch can create supervised children or activate a bus-owned application. An
/// activation does not grant process ownership and is not claimed as supervised/recoverable.
pub struct ApplicationLaunch {
    pub children: Vec<ManagedChild>,
    /// Per-launch errors when a multi-file request partially succeeds.
    pub errors: Vec<Error>,
    pub activated: Option<String>,
}

#[derive(Clone)]
enum Target {
    File(PathBuf),
    Url(String),
}

/// Installed desktop-file launch request. Resolution happens when launch is polled.
pub struct ApplicationRequest {
    id: String,
    targets: Vec<Target>,
    restart: RestartPolicy,
    recover: bool,
    prefer_dbus: bool,
    activation_token: Option<String>,
    pub(crate) session: Option<SessionHandle>,
}

impl ApplicationRequest {
    pub(crate) fn new(id: String) -> Self {
        Self {
            id,
            targets: Vec::new(),
            restart: RestartPolicy::OnFailure,
            recover: true,
            prefer_dbus: false,
            activation_token: None,
            session: None,
        }
    }
    pub fn open_file(mut self, path: impl AsRef<Path>) -> Self {
        self.targets.push(Target::File(path.as_ref().into()));
        self
    }
    pub fn open_url(mut self, url: impl Into<String>) -> Self {
        self.targets.push(Target::Url(url.into()));
        self
    }
    pub fn restart(mut self, policy: RestartPolicy) -> Self {
        self.restart = policy;
        self
    }
    pub fn recover(mut self, enabled: bool) -> Self {
        self.recover = enabled;
        self
    }
    /// Prefer D-Bus activation when advertised. By default Exec is preferred so Telorgon can
    /// supervise the direct child. D-Bus-only entries always use activation.
    pub fn prefer_dbus(mut self, enabled: bool) -> Self {
        self.prefer_dbus = enabled;
        self
    }
    /// Supply an existing activation token from an input/activation grant. Tokens are never
    /// invented here and are consumed only on this initial launch, not on retries or recovery.
    pub fn activation_token(mut self, token: impl Into<String>) -> Self {
        self.activation_token = Some(token.into());
        self
    }

    pub async fn launch(self) -> Result<ApplicationLaunch> {
        let session = match &self.session {
            Some(session) => session.clone(),
            None => super::current()?,
        };
        #[cfg(not(target_os = "linux"))]
        {
            let _ = session;
            return Err(Error::Unsupported(
                "desktop-file application launching is Linux-only".into(),
            ));
        }
        #[cfg(target_os = "linux")]
        {
            // Resolving XDG directories and parsing desktop files performs blocking filesystem IO.
            // Keep it off the compositor's executor thread. Accepted requests remain supervised
            // even if their awaiting task is subsequently cancelled.
            blocking::unblock(move || {
                if session.phase() != super::SessionPhase::Ready {
                    return Err(Error::Closing);
                }
                let path = resolve_id(&self.id, session.env())?;
                let entry = Entry::read(&path)?;
                if entry.value("Type") != Some("Application")
                    || entry.value("Hidden") == Some("true")
                {
                    return Err(Error::Invalid(
                        "desktop entry is hidden or is not an application".into(),
                    ));
                }
                if entry.value("DBusActivatable") == Some("true")
                    && (self.prefer_dbus || entry.value("Exec").is_none())
                {
                    let uris = self
                        .targets
                        .iter()
                        .map(|target| match target {
                            Target::Url(url) => Ok(url.clone()),
                            Target::File(path) => Ok(file_uri(&absolute(path)?)),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let activated = super::bus::activate(
                        session.env(),
                        &self.id,
                        &uris,
                        self.activation_token.as_deref(),
                    )?;
                    return Ok(ApplicationLaunch {
                        children: Vec::new(),
                        errors: Vec::new(),
                        activated: Some(activated),
                    });
                }
                let commands = self.resolve(&session)?;
                let mut children = Vec::new();
                let mut errors = Vec::new();
                for command in commands {
                    match command.spawn() {
                        Ok(child) => children.push(child),
                        Err(error) => errors.push(error),
                    }
                }
                if children.is_empty() && !errors.is_empty() {
                    return Err(errors.remove(0));
                }
                Ok(ApplicationLaunch {
                    children,
                    errors,
                    activated: None,
                })
            })
            .await
        }
    }

    fn resolve(&self, session: &SessionHandle) -> Result<Vec<Command>> {
        let path = resolve_id(&self.id, session.env())?;
        let entry = Entry::read(&path)?;
        if entry.value("Type") != Some("Application") || entry.value("Hidden") == Some("true") {
            return Err(Error::Invalid(
                "desktop entry is hidden or is not an application".into(),
            ));
        }
        if let Some(program) = entry.value("TryExec") {
            if !executable(program.as_ref(), session.env()) {
                return Err(Error::Invalid(format!(
                    "desktop TryExec is unavailable: {program}"
                )));
            }
        }
        let exec = entry.value("Exec").ok_or_else(|| {
            Error::Unsupported(
                "this desktop entry requires D-Bus activation and has no Exec fallback".into(),
            )
        })?;
        let words = split_exec(exec)?;
        if words.is_empty() {
            return Err(Error::Invalid("empty desktop Exec".into()));
        }
        let singles = words.iter().any(|w| w == "%f" || w == "%u");
        let targets = if singles && self.targets.len() > 1 {
            self.targets
                .iter()
                .cloned()
                .map(|t| vec![t])
                .collect::<Vec<_>>()
        } else {
            vec![self.targets.clone()]
        };
        let mut commands = Vec::new();
        for targets in targets {
            let mut args = expand(&words, &entry, &path, &targets)?;
            if entry.value("Terminal") == Some("true") {
                let terminal = if session.config().terminal.is_empty() {
                    [
                        ("foot", "-e"),
                        ("kitty", "--"),
                        ("alacritty", "-e"),
                        ("xterm", "-e"),
                    ]
                    .into_iter()
                    .find(|(program, _)| executable(OsStr::new(program), session.env()))
                    .map(|(program, argument)| vec![program.to_owned(), argument.to_owned()])
                    .ok_or_else(|| {
                        Error::Invalid(
                            "no supported terminal found; configure SessionConfig::terminal".into(),
                        )
                    })?
                } else {
                    session.config().terminal.clone()
                };
                args.splice(0..0, terminal.iter().map(OsString::from));
            }
            let mut command = session
                .command(&args[0])
                .args(&args[1..])
                .restart(self.restart)
                .recover(self.recover);
            command.spec.application = Some(self.id.clone());
            command.activation_token = self.activation_token.clone();
            if let Some(path) = entry.value("Path").filter(|p| !p.is_empty()) {
                command = command.current_dir(path);
            }
            commands.push(command);
        }
        Ok(commands)
    }
}

#[derive(Debug)]
struct Entry(BTreeMap<String, String>);
impl Entry {
    fn read(path: &Path) -> Result<Self> {
        use std::io::Read;
        let mut text = String::new();
        std::fs::File::open(path)?
            .take(1024 * 1024 + 1)
            .read_to_string(&mut text)?;
        if text.len() > 1024 * 1024 {
            return Err(Error::Invalid("desktop entry exceeds 1 MiB".into()));
        }
        Self::parse(&text)
    }
    fn parse(text: &str) -> Result<Self> {
        let mut active = false;
        let mut values = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                active = line == "[Desktop Entry]";
                continue;
            }
            if !active {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| Error::Invalid("malformed desktop entry".into()))?;
            if values.insert(key.trim().into(), unescape(value)?).is_some() {
                return Err(Error::Invalid("duplicate desktop entry key".into()));
            }
        }
        Ok(Self(values))
    }
    fn value(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

fn unescape(value: &str) -> Result<String> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        out.push(match chars.next() {
            Some('s') => ' ',
            Some('n') => '\n',
            Some('t') => '\t',
            Some('r') => '\r',
            Some('\\') => '\\',
            _ => return Err(Error::Invalid("invalid desktop string escape".into())),
        });
    }
    Ok(out)
}

fn split_exec(exec: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = exec.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' => {
                let next = chars
                    .next()
                    .ok_or_else(|| Error::Invalid("trailing Exec escape".into()))?;
                if quoted && !"\"`$\\".contains(next) {
                    return Err(Error::Invalid("invalid quoted Exec escape".into()));
                }
                word.push(next);
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            '%' if quoted => return Err(Error::Invalid(
                "desktop field codes inside quoted arguments are unsupported by the specification"
                    .into(),
            )),
            c if !quoted && "'<>~|&;$*?#()`".contains(c) => {
                return Err(Error::Invalid(
                    "reserved Exec characters must be quoted".into(),
                ));
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err(Error::Invalid("unclosed desktop Exec quote".into()));
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

fn expand(
    words: &[String],
    entry: &Entry,
    desktop: &Path,
    targets: &[Target],
) -> Result<Vec<OsString>> {
    let mut result = Vec::new();
    let mut inputs = 0;
    for word in words {
        match word.as_str() {
            "%f" | "%F" | "%u" | "%U" => {
                inputs += 1;
                for target in targets {
                    let value = match (word.as_str(), target) {
                        ("%f" | "%F", Target::File(path)) => absolute(path)?.into_os_string(),
                        ("%f" | "%F", Target::Url(_)) => {
                            return Err(Error::Invalid(
                                "application accepts local files, not URLs".into(),
                            ));
                        }
                        (_, Target::Url(url)) => OsString::from(url),
                        (_, Target::File(path)) => OsString::from(file_uri(&absolute(path)?)),
                    };
                    result.push(value);
                }
            }
            "%i" => {
                if let Some(icon) = entry.value("Icon") {
                    result.extend([OsString::from("--icon"), icon.into()]);
                }
            }
            "%c" => result.push(entry.value("Name").unwrap_or("").into()),
            "%k" => result.push(desktop.as_os_str().into()),
            "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
            _ => {
                let mut out = String::new();
                let mut chars = word.chars();
                while let Some(ch) = chars.next() {
                    if ch != '%' {
                        out.push(ch);
                    } else if chars.next() == Some('%') {
                        out.push('%');
                    } else {
                        return Err(Error::Invalid(
                            "unknown or embedded desktop Exec field code".into(),
                        ));
                    }
                }
                result.push(out.into());
            }
        }
    }
    if inputs > 1 || result.first().is_none_or(|p| p.is_empty()) {
        return Err(Error::Invalid(
            "invalid desktop executable or repeated input field codes".into(),
        ));
    }
    if inputs == 0 && !targets.is_empty() {
        return Err(Error::Invalid(
            "this desktop entry does not accept files or URLs".into(),
        ));
    }
    Ok(result)
}

fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.into()
    } else {
        std::env::current_dir()?.join(path)
    })
}
fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-_.~".contains(byte) {
            uri.push(char::from(*byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

fn resolve_id(id: &str, env: &Environment) -> Result<PathBuf> {
    if !id.ends_with(".desktop") || id.contains(['/', '\\']) || id == ".desktop" {
        return Err(Error::Invalid(
            "application() expects an installed desktop-file ID, not a path".into(),
        ));
    }
    let mut roots = Vec::new();
    if let Some(home) = env
        .get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            env.get("HOME")
                .map(|home| PathBuf::from(home).join(".local/share"))
        })
    {
        roots.push(home);
    }
    roots.extend(
        std::env::split_paths(
            env.get("XDG_DATA_DIRS")
                .unwrap_or(OsStr::new("/usr/local/share:/usr/share")),
        )
        .filter(|p| p.is_absolute()),
    );
    for root in roots {
        let root = root.join("applications");
        let direct = root.join(id);
        if direct.is_file() {
            return Ok(direct);
        }
        let mut budget = 8192;
        if let Some(path) = find_nested(&root, &root, id, 0, &mut budget)? {
            return Ok(path);
        }
    }
    Err(Error::Invalid(format!(
        "installed application {id:?} was not found"
    )))
}

fn find_nested(
    root: &Path,
    dir: &Path,
    id: &str,
    depth: usize,
    budget: &mut usize,
) -> Result<Option<PathBuf>> {
    if depth > 8 {
        return Ok(None);
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        if *budget == 0 {
            return Err(Error::Invalid(
                "application directory scan limit exceeded".into(),
            ));
        }
        *budget -= 1;
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if let Some(path) = find_nested(root, &path, id, depth + 1, budget)? {
                return Ok(Some(path));
            }
        } else if kind.is_file()
            && path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('/', "-")
                == id
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn executable(program: &OsStr, env: &Environment) -> bool {
    let check = |path: &Path| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            path.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            path.is_file()
        }
    };
    let path = Path::new(program);
    if path.components().count() > 1 {
        return check(path);
    }
    std::env::split_paths(env.get("PATH").unwrap_or_default()).any(|dir| check(&dir.join(path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_arguments_are_literal_and_urls_are_not_shell_code() {
        let entry = Entry::parse(
            "[Desktop Entry]\nType=Application\nName=My Browser\nExec=browser --name %c %U\n",
        )
        .unwrap();
        let words = split_exec(entry.value("Exec").unwrap()).unwrap();
        let args = expand(
            &words,
            &entry,
            Path::new("/apps/browser.desktop"),
            &[Target::Url("https://example.com/;echo bad".into())],
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "browser",
                "--name",
                "My Browser",
                "https://example.com/;echo bad"
            ]
            .map(OsString::from)
        );
    }
    #[test]
    fn rejects_ambiguous_entries_and_unknown_field_codes() {
        assert!(Entry::parse("[Desktop Entry]\nExec=a\nExec=b").is_err());
        assert!(split_exec("a | b").is_err());
        assert!(split_exec("a \"%U\"").is_err());
        assert!(
            expand(
                &["app".into(), "%Z".into()],
                &Entry(BTreeMap::new()),
                Path::new("/a"),
                &[]
            )
            .is_err()
        );
    }
}
