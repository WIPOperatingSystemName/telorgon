use super::*;

impl CompositionDriver {
    pub(super) fn commit_signal_dependencies(
        &mut self,
        id: ComponentInstanceId,
        dependencies: Vec<SignalDependency>,
    ) -> Result<(), ViewError> {
        let mut subscriptions = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            let invalidations = self.signal_invalidations.clone();
            let wake = self.wake.clone();
            subscriptions.push(dependency.subscribe(Arc::new(move || {
                let mut invalidations = invalidations
                    .lock()
                    .expect("composition invalidation queue poisoned");
                if !invalidations.contains(&id) {
                    invalidations.push(id);
                }
                drop(invalidations);
                if let Some(wake) = wake
                    .read()
                    .expect("composition wake lock poisoned")
                    .as_ref()
                    .cloned()
                {
                    wake();
                }
            })));

            // A writer can publish between the view snapshot and subscription registration.
            if dependency.changed() {
                self.signal_invalidations
                    .lock()
                    .expect("composition invalidation queue poisoned")
                    .push(id);
            }
        }
        self.arena
            .get_mut(id)
            .ok_or(ViewError::StaleParent)?
            .signal_subscriptions = subscriptions;
        Ok(())
    }

    pub(super) fn signal_updates_ready(&self) -> bool {
        !self
            .signal_invalidations
            .lock()
            .expect("composition invalidation queue poisoned")
            .is_empty()
    }

    pub(super) fn process_signal_updates(&mut self, context: &mut DriverContext<'_>) -> usize {
        #[cfg(feature = "instrumentation")]
        let signal_span = crate::runtime::instrumentation::span!("signals.drain");
        {
            let mut queue = self
                .signal_invalidations
                .lock()
                .expect("composition invalidation queue poisoned");
            std::mem::swap(&mut *queue, &mut self.signal_scratch);
        }
        self.signal_scratch.sort_unstable();
        self.signal_scratch.dedup();
        #[cfg(feature = "instrumentation")]
        drop(signal_span);
        self.diagnostics.externally_invalidated_components += self.signal_scratch.len() as u64;
        let mut invalidated = std::mem::take(&mut self.signal_scratch);
        let mut processed = 0;
        #[cfg(feature = "instrumentation")]
        let _reconcile_span = crate::runtime::instrumentation::span!("element.reconcile");
        for id in invalidated.drain(..) {
            if self.arena.get(id).is_none() {
                self.diagnostics.stale_events += 1;
                continue;
            }
            match self.reconcile_component(context.ui, id) {
                Ok(()) => {
                    processed += 1;
                    self.diagnostics.externally_reconciled_components += 1;
                    *context.frame_requested = true;
                }
                Err(error) => self.record_error(error),
            }
        }
        self.signal_scratch = invalidated;
        self.signal_scratch.clear();
        processed
    }
}
