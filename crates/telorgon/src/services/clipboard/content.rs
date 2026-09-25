use super::*;

/// Called on a transfer worker, never on the compositor thread. Implementations
/// must honor cancellation and write incrementally to the bounded destination.
pub trait ClipboardProvider: Send + Sync + 'static {
    fn write(
        &self,
        format: &DataFormat,
        output: &mut dyn std::io::Write,
        cancelled: &AtomicBool,
    ) -> Result<()>;
}
#[derive(Clone)]
pub struct ClipboardContent {
    pub(crate) formats: Vec<DataFormat>,
    pub(crate) provider: Arc<dyn ClipboardProvider>,
}
impl std::fmt::Debug for ClipboardContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardContent")
            .field("format_count", &self.formats.len())
            .finish_non_exhaustive()
    }
}
impl ClipboardContent {
    pub fn provider(
        formats: Vec<DataFormat>,
        provider: Arc<dyn ClipboardProvider>,
    ) -> Result<Self> {
        if formats.is_empty()
            || formats.len() > 64
            || formats
                .iter()
                .enumerate()
                .any(|(i, f)| formats[..i].contains(f))
        {
            return Err(ClipboardError::InvalidFormat);
        }
        if formats
            .iter()
            .any(|f| f.kind() != crate::platform::contracts::DataFormatKind::Mime)
        {
            return Err(ClipboardError::InvalidFormat);
        }
        Ok(Self { formats, provider })
    }
    pub fn bytes(entries: Vec<(DataFormat, Vec<u8>)>) -> Result<Self> {
        if entries
            .iter()
            .try_fold(0usize, |sum, (_, bytes)| sum.checked_add(bytes.len()))
            .is_none_or(|size| size > MAX_BYTES)
        {
            return Err(ClipboardError::TooLarge);
        }
        let formats = entries.iter().map(|(f, _)| f.clone()).collect();
        Self::provider(
            formats,
            Arc::new(Bytes(
                entries
                    .into_iter()
                    .map(|(format, bytes)| (format, bytes.into()))
                    .collect(),
            )),
        )
    }
    pub fn text(text: String) -> Result<Self> {
        if text.len() > MAX_BYTES {
            return Err(ClipboardError::TooLarge);
        }
        let formats = vec![
            DataFormat::mime("text/plain;charset=utf-8").unwrap(),
            DataFormat::mime("text/plain").unwrap(),
        ];
        let bytes: Arc<[u8]> = text.into_bytes().into();
        Self::provider(
            formats.clone(),
            Arc::new(Bytes(
                formats
                    .into_iter()
                    .map(|format| (format, bytes.clone()))
                    .collect(),
            )),
        )
    }

    pub fn formats(&self) -> &[DataFormat] {
        &self.formats
    }
}
struct Bytes(Vec<(DataFormat, Arc<[u8]>)>);
impl ClipboardProvider for Bytes {
    fn write(
        &self,
        format: &DataFormat,
        output: &mut dyn std::io::Write,
        cancelled: &AtomicBool,
    ) -> Result<()> {
        let bytes = &self
            .0
            .iter()
            .find(|(f, _)| f == format)
            .ok_or(ClipboardError::InvalidFormat)?
            .1;
        for chunk in bytes.chunks(65536) {
            if cancelled.load(Ordering::Acquire) {
                return Err(ClipboardError::Cancelled);
            }
            output
                .write_all(chunk)
                .map_err(|_| ClipboardError::TransferFailed)?;
        }
        Ok(())
    }
}
