use std::io::{self, BufRead};

pub const MAX_MILESTONE_LINE_BYTES: usize = 1024;
pub const MAX_MILESTONE_TEXT_BYTES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplashMilestone {
    Status(String),
    Progress { completed: u64, total: u64 },
    Ready,
    Failed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum MilestoneError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("invalid splash milestone: {0}")]
    Invalid(&'static str),
}

impl SplashMilestone {
    /// One UTF-8 command per line: status TEXT, progress COMPLETED TOTAL, ready, or fail TEXT.
    /// Empty lines are ignored. Text is bounded and excludes control characters.
    pub fn parse(line: &str) -> Result<Option<Self>, MilestoneError> {
        if line.len() > MAX_MILESTONE_LINE_BYTES {
            return Err(MilestoneError::Invalid("line exceeds 1024 bytes"));
        }
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return Ok(None);
        }
        if line == "ready" {
            return Ok(Some(Self::Ready));
        }
        if let Some(text) = line.strip_prefix("status ") {
            return Ok(Some(Self::Status(validate_text(text)?.to_owned())));
        }
        if let Some(text) = line.strip_prefix("fail ") {
            return Ok(Some(Self::Failed(validate_text(text)?.to_owned())));
        }
        if let Some(progress) = line.strip_prefix("progress ") {
            let mut words = progress.split(' ');
            let completed = number(words.next())?;
            let total = number(words.next())?;
            if words.next().is_some() || total == 0 || completed > total {
                return Err(MilestoneError::Invalid(
                    "progress requires completed <= a positive total",
                ));
            }
            return Ok(Some(Self::Progress { completed, total }));
        }
        Err(MilestoneError::Invalid("unknown command"))
    }
}

/// Reads a bounded record without `read_line` allocating for an unbounded producer-controlled line.
pub fn read_milestone(
    reader: &mut impl BufRead,
) -> Result<Option<SplashMilestone>, MilestoneError> {
    loop {
        let mut line = Vec::with_capacity(MAX_MILESTONE_LINE_BYTES);
        loop {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                if line.is_empty() {
                    return Ok(None);
                }
                break;
            }
            let end = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|n| n + 1);
            let take = end.unwrap_or(available.len());
            if take > MAX_MILESTONE_LINE_BYTES - line.len() {
                return Err(MilestoneError::Invalid("line exceeds 1024 bytes"));
            }
            line.extend_from_slice(&available[..take]);
            reader.consume(take);
            if end.is_some() {
                break;
            }
        }
        let line =
            std::str::from_utf8(&line).map_err(|_| MilestoneError::Invalid("line is not UTF-8"))?;
        if let Some(milestone) = SplashMilestone::parse(line)? {
            return Ok(Some(milestone));
        }
    }
}

fn validate_text(text: &str) -> Result<&str, MilestoneError> {
    if text.trim().is_empty()
        || text.len() > MAX_MILESTONE_TEXT_BYTES
        || text.chars().any(char::is_control)
    {
        Err(MilestoneError::Invalid(
            "text must use 1..=512 bytes without control characters",
        ))
    } else {
        Ok(text)
    }
}

fn number(value: Option<&str>) -> Result<u64, MilestoneError> {
    let value = value
        .filter(|text| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or(MilestoneError::Invalid(
            "progress requires two unsigned decimal integers",
        ))?;
    value
        .parse()
        .map_err(|_| MilestoneError::Invalid("progress integer exceeds u64"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_real_milestones_and_preserves_operation_progress() {
        let mut reader = Cursor::new(b"\nstatus Mounting root\r\nprogress 12 30\nready\n");
        assert_eq!(
            read_milestone(&mut reader).unwrap(),
            Some(SplashMilestone::Status("Mounting root".into()))
        );
        assert_eq!(
            read_milestone(&mut reader).unwrap(),
            Some(SplashMilestone::Progress {
                completed: 12,
                total: 30
            })
        );
        assert_eq!(
            read_milestone(&mut reader).unwrap(),
            Some(SplashMilestone::Ready)
        );
        assert_eq!(read_milestone(&mut reader).unwrap(), None);
    }

    #[test]
    fn rejects_invalid_bounds_and_preserves_utf8_text() {
        for line in [
            "progress 1 0",
            "progress 4 3",
            "progress +1 3",
            "progress 1 3 extra",
            "status bad\ttext",
            "fail ",
            "unknown",
        ] {
            assert!(SplashMilestone::parse(line).is_err(), "{line}");
        }
        assert_eq!(
            SplashMilestone::parse("fail 磁盘错误").unwrap(),
            Some(SplashMilestone::Failed("磁盘错误".into()))
        );
        let mut reader = Cursor::new(vec![b'x'; MAX_MILESTONE_LINE_BYTES + 1]);
        assert!(read_milestone(&mut reader).is_err());
        let mut reader = Cursor::new(vec![0xff, b'\n']);
        assert!(read_milestone(&mut reader).is_err());
    }
}
