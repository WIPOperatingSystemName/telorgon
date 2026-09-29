use super::*;
use crate::host::application::display_control::{
    DisplayCommand, DisplayConfiguration, DisplayControl, DisplayMode, DisplaySnapshot,
};

pub(super) fn mode_value(
    mode: &crate::graphics::presentation::kms::KmsConnectorMode,
) -> DisplayMode {
    DisplayMode {
        width: mode.size().width,
        height: mode.size().height,
        refresh_millihertz: mode.refresh_millihertz(),
    }
}
pub(super) fn mode_index(
    connector: &crate::graphics::presentation::kms::KmsConnector,
    config: &DisplayConfiguration,
) -> AppResult<usize> {
    config.validate().map_err(AppError::new)?;
    let name = format!(
        "DRM-{}-{}",
        connector.connector_type, connector.connector_type_id
    );
    if !config.connector.is_empty() && config.connector != name {
        return Err(AppError::new(
            "The selected monitor is not the shell's active output",
        ));
    }
    match config.mode {
        Some(wanted) => connector
            .modes
            .iter()
            .position(|m| mode_value(m) == wanted)
            .ok_or_else(|| AppError::new("The monitor does not advertise that display mode")),
        None => Ok(connector
            .modes
            .iter()
            .position(|m| m.preferred())
            .unwrap_or(0)),
    }
}

pub(super) struct DisplaySession<'a> {
    control: Option<DisplayControl>,
    snapshot: DisplaySnapshot,
    previous: Option<DisplayConfiguration>,
    deadline: Option<Instant>,
    retry: Option<Instant>,
    next_token: u64,
    pool: Option<renderer::LivePool<'a>>,
}
impl<'a> DisplaySession<'a> {
    pub fn new(
        control: Option<DisplayControl>,
        connector: &crate::graphics::presentation::kms::KmsConnector,
        index: usize,
        scale: crate::host::application::OutputScale,
        resolved: f32,
        wake: &EventNotifier,
    ) -> Self {
        let name = format!(
            "DRM-{}-{}",
            connector.connector_type, connector.connector_type_id
        );
        let snapshot = DisplaySnapshot {
            ready: true,
            connector: name.clone(),
            connector_name: Some(connector.name()),
            monitor_name: connector.identity.name.clone(),
            manufacturer: connector.identity.manufacturer.clone(),
            model: connector.identity.name.clone(),
            product_code: connector.identity.product_code,
            serial_number: connector.identity.serial_number.clone(),
            physical_millimeters: (connector.physical_millimeters.width > 0 && connector.physical_millimeters.height > 0)
                .then_some(connector.physical_millimeters),
            modes: connector.modes.iter().map(mode_value).collect(),
            preferred_mode: connector
                .modes
                .iter()
                .find(|m| m.preferred())
                .or(connector.modes.first())
                .map(mode_value),
            current: DisplayConfiguration {
                connector: name,
                mode: Some(mode_value(&connector.modes[index])),
                scale: match scale {
                    crate::host::application::OutputScale::Auto => None,
                    crate::host::application::OutputScale::Fixed(s) => Some(s),
                },
            },
            resolved_scale: resolved,
            ..Default::default()
        };
        if let Some(control) = &control {
            let wake = wake.clone();
            control.connect(Arc::new(move || wake.notify()));
            control.publish(snapshot.clone(), None);
        }
        Self {
            control,
            snapshot,
            previous: None,
            deadline: None,
            retry: None,
            next_token: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            pool: None,
        }
    }
    pub fn wait(&self, wait: Option<Duration>) -> Option<Duration> {
        if self
            .control
            .as_ref()
            .is_some_and(DisplayControl::has_requests)
        {
            return Some(
                wait.unwrap_or(Duration::from_millis(10))
                    .min(Duration::from_millis(10)),
            );
        }
        self.deadline
            .map(|deadline| self.retry.unwrap_or(deadline))
            .map_or(wait, |deadline| {
                let remaining = deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(10));
                Some(wait.map_or(remaining, |w| w.min(remaining)))
            })
    }
    pub fn process(
        &mut self,
        locked: bool,
        mut apply: impl FnMut(
            &DisplayConfiguration,
            &mut Option<renderer::LivePool<'a>>,
        ) -> AppResult<f32>,
    ) -> bool {
        let Some(control) = self.control.clone() else {
            return false;
        };
        let mut changed = false;
        if self.deadline.is_some_and(|d| Instant::now() >= d)
            && self.retry.is_none_or(|d| Instant::now() >= d)
        {
            let previous = self
                .previous
                .as_ref()
                .expect("preview has previous configuration");
            match apply(previous, &mut self.pool) {
                Ok(scale) => {
                    self.snapshot.current = self.previous.take().unwrap();
                    self.snapshot.resolved_scale = scale;
                    self.snapshot.preview = None;
                    self.snapshot.error =
                        Some("Display changes reverted because they were not confirmed".into());
                    self.deadline = None;
                    self.retry = None;
                    self.pool = None;
                    changed = true;
                }
                Err(error) => {
                    self.snapshot.error =
                        Some(format!("Display rollback failed: {error}; retrying"));
                    self.retry = Some(Instant::now() + Duration::from_secs(1));
                }
            }
        }
        if let Some(request) = control.take_request() {
            let result = if Instant::now() >= request.expires {
                Err("Display request expired before execution".into())
            } else {
                match request.command {
                    DisplayCommand::Preview(mut desired) => {
                        if locked {
                            Err("Unlock the session before changing the display".into())
                        } else if self.previous.is_some() {
                            Err("Confirm or revert the current display preview first".into())
                        } else {
                            desired.connector = if desired.connector.is_empty() {
                                self.snapshot.connector.clone()
                            } else {
                                desired.connector
                            };
                            desired.mode = desired.mode.or(self.snapshot.preferred_mode);
                            match apply(&desired, &mut self.pool) {
                                Ok(scale) => {
                                    let token = self.next_token;
                                    self.next_token += 1;
                                    self.previous = Some(std::mem::replace(
                                        &mut self.snapshot.current,
                                        desired,
                                    ));
                                    self.snapshot.resolved_scale = scale;
                                    self.snapshot.preview = Some(token);
                                    self.snapshot.error = None;
                                    self.deadline = Some(Instant::now() + Duration::from_secs(20));
                                    changed = true;
                                    Ok(token)
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        }
                    }
                    DisplayCommand::Confirm(token) => {
                        if self.snapshot.preview != Some(token)
                            || self.deadline.is_none_or(|d| Instant::now() >= d)
                        {
                            Err("The display preview has expired".into())
                        } else {
                            self.previous = None;
                            self.deadline = None;
                            self.pool = None;
                            self.snapshot.preview = None;
                            self.snapshot.error = None;
                            Ok(token)
                        }
                    }
                    DisplayCommand::Revert(token) => {
                        if self.snapshot.preview != Some(token) {
                            Err("The display preview has expired".into())
                        } else {
                            match apply(self.previous.as_ref().unwrap(), &mut self.pool) {
                                Ok(scale) => {
                                    self.snapshot.current = self.previous.take().unwrap();
                                    self.snapshot.resolved_scale = scale;
                                    self.snapshot.preview = None;
                                    self.snapshot.error = None;
                                    self.deadline = None;
                                    self.pool = None;
                                    changed = true;
                                    Ok(token)
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        }
                    }
                }
            };
            // Callers may fetch a snapshot immediately after the reply arrives.
            control.publish(self.snapshot.clone(), self.deadline);
            let _ = request.reply.send(result);
        }
        control.publish(self.snapshot.clone(), self.deadline);
        changed
    }
}
impl Drop for DisplaySession<'_> {
    fn drop(&mut self) {
        if let Some(control) = &self.control {
            control.disconnect();
        }
    }
}

pub(super) struct AppliedDisplay<'a> {
    pub index: usize,
    pub scale: crate::platform::contracts::ScaleFactor,
    pub output: OutputState,
    pub blob: Option<crate::graphics::presentation::kms::PropertyBlob<'a>>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply<'a>(
    desired: &DisplayConfiguration,
    previous_pool: &mut Option<renderer::LivePool<'a>>,
    kms: &'a KmsDevice,
    gbm: Option<&'a GbmDevice<'a>>,
    connector: &crate::graphics::presentation::kms::KmsConnector,
    current_index: usize,
    crtc: KmsCrtcId,
    plane: KmsPlaneId,
    connector_properties: &KmsObjectProperties,
    crtc_properties: &KmsObjectProperties,
    plane_properties: &KmsObjectProperties,
    renderer: &mut renderer::ShellRenderer,
    buffers: &mut Vec<crate::graphics::presentation::kms::ScanoutBuffer<'a>>,
    framebuffers: &mut Vec<KmsFramebuffer<'a>>,
) -> AppResult<AppliedDisplay<'a>> {
    let index = mode_index(connector, desired)?;
    let mode = &connector.modes[index];
    let extent = mode.size();
    let scale = desired
        .scale
        .map_or(
            crate::host::application::OutputScale::Auto,
            crate::host::application::OutputScale::Fixed,
        )
        .resolve(extent, connector.physical_millimeters)?;
    let output = output_state(connector, index, scale)?;
    let blob = if index != current_index {
        let blob = kms.create_mode_blob(mode).map_err(app_error)?;
        let mut prepared = if previous_pool.is_none() {
            Some(renderer::prepare_live_pool(
                kms,
                gbm,
                renderer,
                &buffers[0],
                extent,
            )?)
        } else {
            None
        };
        let pool = previous_pool
            .as_ref()
            .or(prepared.as_ref())
            .expect("prepared pool");
        let request = kms
            .primary_modeset_request(
                connector.id,
                connector_properties,
                crtc,
                crtc_properties,
                plane,
                plane_properties,
                blob.id(),
                pool.framebuffers[0].id(),
                extent.width as u32,
                extent.height as u32,
            )
            .map_err(app_error)?;
        request.test(true).map_err(app_error)?;
        request.commit(true, false).map_err(app_error)?;
        let pool = previous_pool.take().or(prepared.take()).unwrap();
        *previous_pool = Some(renderer::exchange_pool(
            pool,
            renderer,
            framebuffers,
            buffers,
        ));
        Some(blob)
    } else {
        None
    };
    Ok(AppliedDisplay {
        index,
        scale,
        output,
        blob,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> DisplaySession<'static> {
        let control = DisplayControl::new(DisplayConfiguration::default()).unwrap();
        let snapshot = DisplaySnapshot {
            ready: true,
            ..Default::default()
        };
        control.publish(snapshot.clone(), None);
        DisplaySession {
            control: Some(control),
            snapshot,
            previous: None,
            deadline: None,
            retry: None,
            next_token: 1,
            pool: None,
        }
    }
    fn request(
        session: &mut DisplaySession<'static>,
        command: DisplayCommand,
        mut apply: impl FnMut(
            &DisplayConfiguration,
            &mut Option<renderer::LivePool<'static>>,
        ) -> AppResult<f32>,
    ) -> Result<u64, String> {
        let control = session.control.as_ref().unwrap().clone();
        let worker = std::thread::spawn(move || match command {
            DisplayCommand::Preview(config) => control.preview(config),
            DisplayCommand::Confirm(token) => control.confirm(token).map(|_| token),
            DisplayCommand::Revert(token) => control.revert(token).map(|_| token),
        });
        let limit = Instant::now() + Duration::from_secs(2);
        while !worker.is_finished() {
            assert!(Instant::now() < limit, "display command was not serviced");
            session.process(false, &mut apply);
            std::thread::sleep(Duration::from_millis(1));
        }
        worker.join().unwrap()
    }
    #[test]
    fn rejected_preview_never_changes_current_configuration() {
        let mut session = session();
        let desired = DisplayConfiguration {
            scale: Some(2.0),
            ..Default::default()
        };
        assert!(
            request(&mut session, DisplayCommand::Preview(desired), |_, _| Err(
                AppError::new("unsupported")
            ))
            .is_err()
        );
        assert!(session.deadline.is_none());
        assert_eq!(session.snapshot.current, DisplayConfiguration::default());
    }
    #[test]
    fn unconfirmed_preview_reverts_even_without_a_client() {
        let mut session = session();
        let original = session.snapshot.current.clone();
        let desired = DisplayConfiguration {
            scale: Some(1.5),
            ..Default::default()
        };
        let token = request(
            &mut session,
            DisplayCommand::Preview(desired.clone()),
            |_, _| Ok(1.5),
        )
        .unwrap();
        assert_eq!(session.snapshot.current, desired);
        session.deadline = Some(Instant::now());
        assert!(session.process(false, |restore, _| {
            assert_eq!(restore, &original);
            Ok(1.0)
        }));
        assert_eq!(session.snapshot.current, original);
        assert!(session.snapshot.preview.is_none());
        assert!(
            request(&mut session, DisplayCommand::Confirm(token), |_, _| panic!(
                "confirmation cannot apply"
            ))
            .is_err()
        );
    }
    #[test]
    fn confirmation_cancels_rollback_and_second_preview_is_rejected() {
        let mut session = session();
        let desired = DisplayConfiguration {
            scale: Some(2.0),
            ..Default::default()
        };
        let token = request(
            &mut session,
            DisplayCommand::Preview(desired.clone()),
            |_, _| Ok(2.0),
        )
        .unwrap();
        assert!(
            request(
                &mut session,
                DisplayCommand::Preview(desired.clone()),
                |_, _| panic!("overlapping preview")
            )
            .is_err()
        );
        request(&mut session, DisplayCommand::Confirm(token), |_, _| {
            panic!("confirmation cannot apply")
        })
        .unwrap();
        assert!(session.deadline.is_none());
        assert!(session.previous.is_none());
        assert_eq!(session.snapshot.current, desired);
    }
}
