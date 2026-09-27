use std::io::SeekFrom;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// Incremental log tailer: opens a file and only ever reads new data.
pub struct Tailer {
    file: File,
    pos: u64,
    buf: Vec<u8>,
}

impl Tailer {
    /// Open and seek to EOF (only see appends from now on).
    pub async fn follow(path: &std::path::Path) -> std::io::Result<Self> {
        let mut file = File::open(path).await?;
        let pos = file.seek(SeekFrom::End(0)).await?;
        Ok(Self {
            file,
            pos,
            buf: Vec::with_capacity(64 * 1024),
        })
    }

    pub async fn from_start(path: &std::path::Path) -> std::io::Result<Self> {
        let file = File::open(path).await?;
        Ok(Self {
            file,
            pos: 0,
            buf: Vec::with_capacity(64 * 1024),
        })
    }

    /// Read whatever new data is available; return complete lines.
    /// A trailing half-line is kept in the internal buffer.
    pub async fn poll(&mut self) -> std::io::Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        let mut chunk = vec![0u8; 65536];
        loop {
            match self.file.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    self.pos += n as u64;
                    self.buf.extend_from_slice(&chunk[..n]);
                    while let Some(idx) = self.buf.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = self.buf.drain(..=idx).collect();
                        if let Ok(s) = String::from_utf8(line) {
                            let s = s.trim();
                            if !s.is_empty() {
                                out.push(s.to_string());
                            }
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
        // Safety valve: a single JSONL line should never be this large.
        if self.buf.len() > 1_000_000 {
            self.buf.clear();
        }
        Ok(out)
    }

    pub fn pos(&self) -> u64 {
        self.pos
    }

    pub async fn size(&self) -> std::io::Result<u64> {
        Ok(self.file.metadata().await?.len())
    }
}
