//! Small streaming PCM16 writer shared by the media examples; never overwrites a file.
use std::{
    fs::{File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    path::Path,
};
pub struct Wave {
    file: File,
    bytes: u32,
    channels: u16,
    scratch: Vec<u8>,
}
impl Wave {
    pub fn create(path: &Path, rate: u32, channels: u16) -> io::Result<Self> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut header = Vec::with_capacity(44);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&36u32.to_le_bytes());
        header.extend_from_slice(b"WAVEfmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&channels.to_le_bytes());
        header.extend_from_slice(&rate.to_le_bytes());
        header.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        header.extend_from_slice(&(channels * 2).to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&0u32.to_le_bytes());
        file.write_all(&header)?;
        Ok(Self {
            file,
            bytes: 0,
            channels,
            scratch: Vec::with_capacity(32768),
        })
    }
    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        if samples.len() % usize::from(self.channels) != 0 {
            return Err(io::Error::other("incomplete PCM frame"));
        }
        let extra = u32::try_from(samples.len())
            .ok()
            .and_then(|n| n.checked_mul(2));
        let next = extra
            .and_then(|n| self.bytes.checked_add(n))
            .filter(|n| *n <= u32::MAX - 36)
            .ok_or_else(|| io::Error::other("PCM WAVE size limit"))?;
        self.scratch.clear();
        for sample in samples {
            let sample = if sample.is_finite() { *sample } else { 0.0 };
            self.scratch.extend_from_slice(
                &((sample * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes(),
            );
        }
        self.file.write_all(&self.scratch)?;
        self.bytes = next;
        Ok(())
    }
    pub fn finish(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&(self.bytes + 36).to_le_bytes())?;
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&self.bytes.to_le_bytes())?;
        self.file.seek(SeekFrom::End(0))?;
        self.file.flush()
    }
}
impl Drop for Wave {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
