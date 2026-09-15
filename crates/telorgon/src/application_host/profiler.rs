#[cfg(any(not(feature = "profiler"), test))]
use std::ffi::OsStr;

use crate::application_host::{AppError, AppResult};

#[cfg(any(not(feature = "profiler"), test))]
const PROFILER_ARGUMENT: &str = "--telorgon-profile";

#[cfg(feature = "profiler")]
pub(crate) struct ManagedProfiler {
    _server: Option<crate::profiler_server::ProfilerServer>,
}

#[cfg(not(feature = "profiler"))]
pub(crate) struct ManagedProfiler;

impl ManagedProfiler {
    pub(crate) fn start(_target: ProfileTarget) -> AppResult<Self> {
        #[cfg(feature = "profiler")]
        {
            use crate::profiler_server::{ProfilerRequest, ServerConfig};

            let server = crate::profiler_server::ProfilerServer::start_if_requested(
                ProfilerRequest::from_process_args(),
                ServerConfig::for_target(_target.into_server_target()),
            )
            .map_err(|error| AppError::new(format!("Telorgon profiler startup failed: {error}")))?;
            Ok(Self { _server: server })
        }
        #[cfg(not(feature = "profiler"))]
        {
            if profiler_argument_present(std::env::args_os()) {
                return Err(AppError::new(
                    "--telorgon-profile requires profiler code; run `cargo profile` or enable the application `telorgon-profiler` feature",
                ));
            }
            Ok(Self)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Each feature-selected managed host constructs its corresponding target.
pub(crate) enum ProfileTarget {
    Gui,
    ShellEnvironment,
}

#[cfg(feature = "profiler")]
impl ProfileTarget {
    fn into_server_target(self) -> crate::profiler_server::ProfileTarget {
        match self {
            Self::Gui => crate::profiler_server::ProfileTarget::Gui,
            Self::ShellEnvironment => crate::profiler_server::ProfileTarget::ShellEnvironment,
        }
    }
}

#[cfg(any(not(feature = "profiler"), test))]
fn profiler_argument_present<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .any(|argument| argument.as_ref() == OsStr::new(PROFILER_ARGUMENT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_reserved_argument_requests_profiling() {
        assert!(profiler_argument_present(["app", "--telorgon-profile"]));
        assert!(!profiler_argument_present(["app", "--telorgon-profiled"]));
        assert!(!profiler_argument_present(["app", "profile"]));
    }

    #[test]
    #[cfg(feature = "profiler")]
    fn every_declaration_target_maps_to_generic_profiler_metadata() {
        use crate::profiler_server::ProfileTarget as ServerTarget;

        assert_eq!(ProfileTarget::Gui.into_server_target(), ServerTarget::Gui);
        assert_eq!(
            ProfileTarget::ShellEnvironment.into_server_target(),
            ServerTarget::ShellEnvironment
        );
    }
}
