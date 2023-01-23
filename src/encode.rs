use crate::utilities::CountingSink;
use anyhow::Result;
use png::Compression;
use std::io::Write;

pub trait Encode {
    fn encode<W: Write>(&self, priority: Priority, into: W) -> Result<()>;

    fn estimate(&self) -> Result<usize> {
        let mut sink = CountingSink::default();
        self.encode(Priority::Speed, &mut sink)?;
        Ok(sink.len())
    }
}

pub enum Priority {
    Size,
    Speed,
}

impl From<Priority> for Compression {
    fn from(priority: Priority) -> Self {
        match priority {
            Priority::Size => Self::Best,
            Priority::Speed => Self::Fast,
        }
    }
}
