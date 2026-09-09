# Managed sessions and process launching

This document describes the current implementation. Command supervision and recovery have real
subprocess unit-test coverage. TTY/KMS logout, signal handling with real clients, and shared user-bus
integration still require manual qualification; these features are not production-qualified.

## Startup

Both managed `Application::gui(...).run()` and
`Application::desktop_environment(...).run()` establish the process-wide `telorgon::session` service
before application callbacks can launch children. Builders can be constructed earlier, but executing
one before startup returns `session::Error::NotReady`. Only one managed session runs per process.
An explicitly retained `SessionHandle` is optional; old handles cannot launch into a later session.

For an ordinary GUI, the host inherits the existing graphical session. On Linux it requires
`WAYLAND_DISPLAY` or `DISPLAY`; it does not invent a display server when run on a bare TTY.
Set the optional GUI `.session(session::SessionConfig::new("my-app"))` builder value for a stable
recovery identity. Otherwise the GUI derives an identity from its application name.

For a desktop environment, `LinuxDesktopConfig::default()` now selects the GPU and Wayland socket
automatically. `drm_device: None` searches DRM card devices through the seat and selects a device
with a connected output and mode. An explicit override is `Some(path.into())`.
`socket_name: None` binds the first free `wayland-0` through `wayland-32` socket inside the
validated runtime directory. Libwayland owns socket locking and cleanup. Startup logs the selected
socket. Configure identity and launch policy through `LinuxDesktopConfig.session`.

A user can start the DE executable directly from a properly established local TTY login. Linux
still must provide an active seat, working DRM/input permissions through logind or seatd, and the
native libraries/drivers required by the desktop feature. The runtime directory must be an absolute,
non-symlink directory owned by the user with mode 0700. If `XDG_RUNTIME_DIR` is absent, the host uses
an existing valid `/run/user/<uid>`. It does not create a fake PAM login, runtime directory, seat,
or D-Bus daemon. Logind/seatd grant device access during backend startup; a display manager, login
shell, or appropriately configured system service is responsible for boot-time execution.

## Child environment

The session snapshots the inherited environment and applies overrides to each child, without
mutating process-global environment variables. DE children receive the actual `WAYLAND_DISPLAY`,
`XDG_RUNTIME_DIR`, `XDG_SESSION_TYPE=wayland`, and configured desktop identity. Stale X11 `DISPLAY`
and `XAUTHORITY` values are removed. If no session-bus address was inherited, an existing runtime
`bus` socket supplies the address. GUI children preserve their parent display environment.
Inherited `WAYLAND_SOCKET`, `XDG_ACTIVATION_TOKEN`, and `DESKTOP_STARTUP_ID` are removed so that
one-shot connection/activation grants are not accidentally reused.

The optional [X11 foundation components](X11_COMPATIBILITY.md) do not yet change
this environment policy. A separate private-helper supervisor and startup barrier
controller now have component coverage, but their managed-host/XWM integration
must be completed before publishing session-owned `DISPLAY`/`XAUTHORITY`. Unlike
ordinary application handles, dropping the helper handle requests termination and
asynchronous reaping; it never opts applications into recovery or automatic retry.

This environment applies to launches through the session API. Third-party code calling
`std::process::Command` directly does not receive these overrides automatically.

## Commands and applications

```rust,ignore
use telorgon::session;

// In a mounted callback, after the managed host has started:
let terminal = session::command("foot")
    .restart(session::RestartPolicy::OnFailure)
    .recover(true)
    .spawn()?;

// In an async task on the existing application executor:
let output = session::command("cat")
    .input(b"hello\n".to_vec())
    .output()
    .await?;
let success = output.status.success();

let launch = session::application("org.example.Editor.desktop")
    .open_file("/home/user/notes.txt")
    .launch()
    .await?;
// A multi-file request can partially succeed: inspect launch.errors as well as launch.children.
```

`command` passes literal arguments; it does not parse a command line. Use `.arg()`/`.args()` for
arguments, `.current_dir()` for a working directory, and `.env()`/`.env_remove()` for child-only
changes. `session::shell(script)` explicitly opts into shell interpretation, so do not concatenate
untrusted values into scripts. Commands default to no retries and no persistent recovery.

`spawn()` performs process creation synchronously and returns a `ManagedChild`. `status().await`
waits without blocking the application's executor thread. `output().await` captures both output
streams; its result includes exit status, bytes, and truncation flags. `.input(bytes)` feeds at most
8 MiB through stdin on the supervisor thread and closes stdin afterwards. Output defaults to an
8 MiB limit per stream; `.output_limit(bytes)` accepts 1 byte through 64 MiB. Excess output is drained
and discarded to avoid pipe deadlock. Descendant-inherited output pipes are closed 250 ms after the
direct child exits, marking incomplete output as truncated. Interactive commands normally inherit
streams; `.stdin/.stdout/.stderr` can select `Stream::Inherit`, `Null`, or output `Capture`.

These are ordinary Rust futures using wakers, with no Tokio runtime requirement. `.await` suspends
the calling task until a result is ready; `?` propagates an error to the caller. A dedicated worker
supervises/reaps children and services their pipes; desktop-file lookup and blocking D-Bus calls run
on a blocking task pool. Dropping a child handle or cancelling its wait does not kill the accepted
process. At most 256 children and pending recovery entries are retained, with at most 64 concurrent
waiters per completion. Poll `session::current()?.take_diagnostics()` to present supervision or
journal failures in the shell; remaining diagnostics are also reported when the owner shuts down.

Linux application launching resolves installed desktop IDs with XDG user-over-system precedence,
including nested desktop-file IDs. It checks hidden/type/TryExec entries, parses standard `Exec`
quoting and field codes without invoking a shell, and supports file/URL requests and terminal
wrapping. Set `SessionConfig.terminal` to the terminal executable plus prefix arguments; otherwise
foot, kitty, alacritty, and xterm are tried in order. `Path` supplies a working directory.
Applications default to crash retries and persistent recovery.

`Exec` is preferred for direct-child supervision. `.prefer_dbus(true)` requests advertised
`org.freedesktop.Application` activation; D-Bus-only entries also use that path. A bus activation
returns `ApplicationLaunch.activated` and grants no child-process ownership: existing or bus-owned
apps are not claimed as supervised or recoverable. `.activation_token(token)` accepts an existing
input/activation grant for the initial launch only, and never persists or reuses it on retry.
Desktop-file launching is Linux-only; generic command execution uses the platform process API.

## Shutdown and recovery

`RestartPolicy::OnFailure` retries a failed direct child at most three times, with 1, 2, and 4 second
backoff. Successful exits do not restart. Retries stop when the session begins closing, and exhausted
recoverable failures remain available for an explicit recovery action. This supervises direct
children: applications that daemonize, hand work to an existing instance, or delegate to services
cannot be tracked as an entire application by this mechanism.

DE `request_exit()`, SIGTERM, and SIGINT enter a quiescing phase that refuses launches and retries.
The compositor requests normal close from its Wayland toplevel clients and continues dispatching
so save prompts can work. If windows remain past the configured timeout (default 30 seconds), logout
is cancelled and the session resumes. Once windows close, tracked children receive SIGTERM and get
another bounded grace period. There is no automatic SIGKILL. Unresponsive children are reported and
retained for recovery rather than silently force-killed. GUI host exit also closes its managed
children; Wayland-wide logout and POSIX termination-signal handling belong to the DE host.
Graceful child signalling currently uses Linux pidfds; other platforms report unsupported graceful
termination instead of pretending that a forceful process kill is a normal close.

Recoverable launch specifications are periodically checkpointed to a private, atomically replaced
journal at `$XDG_STATE_HOME/telorgon/<identity>` (fallback `$HOME/.local/state/telorgon/<identity>`).
This records executable, arguments, working directory and process identity, not arbitrary app
memory or exported environment. Arguments can contain private filenames/URLs. Recovery requires
inherited streams, no custom environment, and no piped input; opt out with `.recover(false)` for
customized commands. `SessionConfig.recovery = false` disables disk persistence. Journal updates
are asynchronous, so a crash immediately following a launch can precede its durable checkpoint.
Files are bounded to 1 MiB and 256 entries, protected against symlink replacement and concurrent
writers. A DE refuses a conflicting journal owner; another GUI instance runs without persistence
and reports the conflict through diagnostics.

After an abnormal host exit, the next startup exposes saved launches through
`session::pending_recovery()`. Present those entries to the user and call `entry.restore()` after
their recovery action; use `session::current()?.dismiss_recovery(entry.id())` to discard one.
Automatic replay of every command on boot is deliberately avoided. Linux boot/process-start identity
prevents duplicate restoration while an original child is still running, and generation checks reject
stale entry objects. Restored apps receive the new session environment/socket. App-specific document
or window restoration still depends on the app's own saved-state support.

## Optional user-service environment

`SessionConfig.publish_user_service_environment` defaults to false and is rejected by GUI hosts.
A DE that owns the user's single graphical session may enable it to publish display variables to
systemd's user manager and D-Bus activation environment after the socket is ready. This requires an
existing reachable user bus and systemd user manager; startup errors are reported, not ignored.

The shared environment is per user, not per compositor. On exit, the guard restores values only
where the current systemd value still matches its own publication. D-Bus cannot expose/unset its
activation environment, so restoration uses the previous systemd snapshot and empty values for
previously absent keys. This assumes exclusive graphical-session ownership; it is not an isolation
mechanism for simultaneous desktops or unrelated concurrent environment publishers.

## Integration and qualification

The sibling test compositor now uses the local framework dependency, automatic DRM/socket selection,
and session-owned launches. Ctrl+T starts a supervised terminal, Ctrl+Q requests logout, and
Ctrl+Shift+R restores the pending launch entries as an explicit recovery action.

Unit fixtures cover literal argument/environment passing, bounded bidirectional pipe handling,
cancelled waiters, child reaping, retry exhaustion, graceful shutdown, recovery identity, journal
permissions/locking, desktop-file precedence/expansion, and automatic absolute-path socket binding.
Manual qualification must still exercise a real TTY seat, multi-GPU selection, real save prompts,
SIGTERM/SIGINT logout, D-Bus activation/environment restoration, and application-owned state recovery.
Repository policy leaves interactive runs and services to the user.

The adjacent `../other-rendering-libs` source library was unavailable during this work; no reference
code was copied and no renderer lifetime or synchronization mechanism was redesigned. Environment
and protocol invariants were checked against the official
[Rust child environment API](https://doc.rust-lang.org/std/process/struct.Command.html#method.env),
[Wayland server socket API](https://wayland.freedesktop.org/docs/html/apc.html#wl_display_add_socket),
[Desktop Entry specification](https://specifications.freedesktop.org/desktop-entry-spec/latest/),
and [D-Bus application activation contract](https://specifications.freedesktop.org/desktop-entry-spec/latest/dbus.html).
Child-local environments avoid unsafe global mutation; absolute socket binding avoids temporary
exports; bounded retries avoid crash loops; explicit recovery avoids replaying arbitrary commands
without a user action. Corresponding invariants are covered by the unit fixtures above.

### Embedded compatibility startup

An embedded-Xwayland desktop queues initial direct managed launches until helper
startup succeeds or fails. The compositor owner does not wait for those launches.
Success supplies DISPLAY/XAUTHORITY alongside WAYLAND_DISPLAY; failure resumes
native launches. A queued `ManagedChild` has `id() == 0` and no OS process yet;
never pass that value to process-signalling APIs. Its status/output future resolves
after execution or reports cancellation if the session closes first. Recoverable
queued commands remain offered for explicit recovery after cancellation.
