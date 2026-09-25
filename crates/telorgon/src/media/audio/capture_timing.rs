//! Two SPSC rings: sample publication precedes its descriptor. Readers never consume
//! samples without first owning the descriptor, including during partial reads/flush.
use super::AudioTiming;
use crate::integrations::pipewire::MediaError;

#[derive(Clone, Copy, Debug)]
pub struct CapturedAudio {
    /// Timing of the original processing block; `offset_frames` locates this fragment.
    pub timing: AudioTiming,
    pub offset_frames: usize,
    pub frames: usize,
    pub discontinuity: bool,
}
impl CapturedAudio {
    /// Stream-relative first-frame position, including any skipped capture quanta.
    pub fn frame_position(self) -> Option<u64> {
        self.timing
            .frame_position
            .checked_add(u64::try_from(self.offset_frames).ok()?)
    }
}
#[derive(Clone, Copy)]
pub(crate) struct CaptureBlock {
    pub timing: AudioTiming,
    pub frames: usize,
    pub discontinuity: bool,
}
pub(crate) struct CaptureReader {
    pub samples: rtrb::Consumer<f32>,
    pub blocks: rtrb::Consumer<CaptureBlock>,
    pub pending: Option<CaptureBlock>,
    pub offset: usize,
    pub next_discontinuity: bool,
}
impl CaptureReader {
    pub fn read(
        &mut self,
        destination: &mut [f32],
        channels: usize,
    ) -> Result<Option<CapturedAudio>, MediaError> {
        if destination.len() < channels {
            return Err(MediaError::InvalidArgument(
                "capture destination needs one complete frame",
            ));
        }
        if self.pending.is_none() {
            self.pending = self.blocks.pop().ok();
            self.offset = 0;
        }
        let Some(block) = self.pending else {
            return Ok(None);
        };
        let frames = (destination.len() / channels).min(block.frames - self.offset);
        let count = frames * channels;
        let chunk = self
            .samples
            .read_chunk(count)
            .map_err(|_| MediaError::NotReady)?;
        let (a, b) = chunk.as_slices();
        destination[..a.len()].copy_from_slice(a);
        destination[a.len()..count].copy_from_slice(b);
        chunk.commit_all();
        let result = CapturedAudio {
            timing: block.timing,
            offset_frames: self.offset,
            frames,
            discontinuity: self.next_discontinuity || (self.offset == 0 && block.discontinuity),
        };
        self.next_discontinuity = false;
        self.offset += frames;
        if self.offset == block.frames {
            self.pending = None;
            self.offset = 0;
        }
        Ok(Some(result))
    }
    pub fn discard_queued(&mut self, channels: usize) {
        // Bound work to the descriptors published at entry. A concurrent producer may
        // publish later blocks; those stay paired and remain available after this flush.
        let count = self.blocks.slots() + usize::from(self.pending.is_some());
        self.next_discontinuity = true;
        for _ in 0..count {
            let block = match self.pending.take() {
                Some(block) => Some(block),
                None => {
                    self.offset = 0;
                    self.blocks.pop().ok()
                }
            };
            let Some(block) = block else {
                break;
            };
            let remaining = (block.frames - self.offset) * channels;
            match self.samples.read_chunk(remaining) {
                Ok(chunk) => chunk.commit_all(),
                Err(_) => {
                    self.pending = Some(block);
                    break;
                }
            }
            self.offset = 0;
        }
    }
}
