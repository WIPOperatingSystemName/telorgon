use super::{
    controller::{Job, Shared, validate_target},
    *,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

struct Pending {
    token: u64,
    completion: Arc<super::request::Completion>,
    deadline: Instant,
}

pub(super) fn run(shared: Arc<Shared>, mut provider: Box<dyn NetworkProvider>) {
    // Always terminate outstanding waiters even if an injected backend panics.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        work(&shared, provider.as_mut())
    }));
    let (jobs, active) = {
        let mut state = shared.lock();
        state.stopped = true;
        (
            state.queue.drain(..).collect::<Vec<_>>(),
            state.active.take(),
        )
    };
    if let Some(completion) = active {
        completion.finish(NetworkOutcome::Unconfirmed(NetworkError::BackendFailure));
    }
    for job in jobs {
        job.completion
            .finish(NetworkOutcome::Failed(NetworkError::Stopped));
    }
    shared.challenge_writer.publish(Vec::new());
    let mut snapshot = (*shared.signal.snapshot()).clone();
    snapshot.state = NetworkServiceState::Stopped;
    if result.is_err() {
        snapshot.last_error = Some(NetworkError::BackendFailure);
    }
    snapshot.revision += 1;
    shared.writer.publish(snapshot);
}

fn work(shared: &Shared, provider: &mut dyn NetworkProvider) {
    let mut pending: Option<Pending> = None;
    let mut next_refresh = Instant::now();
    loop {
        if shared.lock().stopped {
            if let Some(p) = pending.take() {
                provider.abandon(p.token);
                p.completion
                    .finish(NetworkOutcome::Unconfirmed(NetworkError::Stopped));
            }
            return;
        }
        let now = Instant::now();
        if now >= next_refresh || provider.has_changes() {
            publish(shared, provider.snapshot());
            next_refresh = Instant::now()
                + if pending.is_some() {
                    shared.config.poll_interval.min(Duration::from_millis(250))
                } else {
                    shared.config.poll_interval
                };
        }
        if let Some(p) = &pending {
            let outcome = if now >= p.deadline {
                Some(NetworkOutcome::Unconfirmed(NetworkError::TimedOut))
            } else {
                match provider.poll(p.token, &shared.signal.snapshot()) {
                    Ok(Some(result)) => Some(NetworkOutcome::Applied(result)),
                    Ok(None) => None,
                    Err(error) => Some(NetworkOutcome::Unconfirmed(error)),
                }
            };
            if let Some(outcome) = outcome {
                let p = pending.take().unwrap();
                provider.abandon(p.token);
                if matches!(outcome, NetworkOutcome::Applied(_)) {
                    publish(shared, provider.snapshot());
                }
                p.completion.finish(outcome);
                shared.lock().active = None;
            }
        }
        let (expired, challenges, job) = {
            let mut state = shared.lock();
            let mut expired = Vec::new();
            state.queue.retain(|job| {
                if job.completion.terminal() {
                    false
                } else if Instant::now() >= job.deadline {
                    expired.push(job.completion.clone());
                    false
                } else {
                    true
                }
            });
            let job = if pending.is_none() {
                state
                    .queue
                    .iter()
                    .position(|j| !j.command.needs_credentials())
                    .and_then(|i| state.queue.remove(i))
            } else {
                None
            };
            let challenges = state
                .queue
                .iter()
                .filter_map(|job| {
                    if job.command.needs_credentials() {
                        let NetworkCommand::ConnectWifi { interface, options } = &job.command
                        else {
                            return None;
                        };
                        Some(NetworkCredentialChallenge {
                            request: job.id,
                            interface: *interface,
                            ssid: options.ssid.clone(),
                            security: options.security,
                        })
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            (expired, challenges, job)
        };
        for completion in expired {
            completion.finish(NetworkOutcome::Failed(NetworkError::TimedOut));
        }
        shared.challenge_writer.publish_if_changed(challenges);
        if let Some(job) = job {
            if job.completion.begin() {
                shared.lock().active = Some(job.completion.clone());
                publish(shared, provider.snapshot());
                if shared.lock().stopped {
                    job.completion
                        .finish(NetworkOutcome::Failed(NetworkError::Stopped));
                    shared.lock().active = None;
                    continue;
                }
                if Instant::now() >= job.deadline {
                    job.completion
                        .finish(NetworkOutcome::Failed(NetworkError::TimedOut));
                    shared.lock().active = None;
                    continue;
                }
                if let Err(error) = validate_target(&job.command, &shared.signal.snapshot()) {
                    job.completion.finish(NetworkOutcome::Failed(error));
                    shared.lock().active = None;
                } else {
                    // execute consumes passwords; pending jobs retain only nonsecret metadata.
                    let Job {
                        command,
                        completion,
                        deadline,
                        ..
                    } = job;
                    match provider.execute(command) {
                        Ok(NetworkDispatch::Complete(result)) => {
                            publish(shared, provider.snapshot());
                            completion.finish(NetworkOutcome::Applied(result));
                            shared.lock().active = None;
                        }
                        Ok(NetworkDispatch::Pending(token)) => {
                            pending = Some(Pending {
                                token,
                                completion,
                                deadline,
                            });
                            next_refresh = Instant::now();
                        }
                        Err(error) => {
                            publish(shared, provider.snapshot());
                            completion.finish(NetworkOutcome::Failed(error));
                            shared.lock().active = None;
                        }
                    }
                }
            }
        }
        let guard = shared.lock();
        if guard.stopped {
            continue;
        }
        let _ = shared
            .wake
            .wait_timeout(guard, Duration::from_millis(100))
            .unwrap_or_else(|e| e.into_inner());
    }
}

fn publish(shared: &Shared, result: Result<NetworkSnapshot, NetworkError>) {
    let previous = shared.signal.snapshot();
    let mut snapshot = match result {
        Ok(snapshot) => snapshot,
        Err(error) => NetworkSnapshot {
            state: if error == NetworkError::PermissionDenied {
                NetworkServiceState::Restricted
            } else {
                NetworkServiceState::Unavailable
            },
            last_error: Some(error),
            ..Default::default()
        },
    };
    snapshot.revision = previous.revision;
    if *previous != snapshot {
        snapshot.revision += 1;
        shared.writer.publish(snapshot);
    }
}
