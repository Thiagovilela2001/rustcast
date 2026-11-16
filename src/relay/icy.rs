use bytes::{Bytes, BytesMut};
use tokio::sync::watch;

pub const METAINT: usize = 16000;

pub fn meta_block(title: &str) -> Bytes {
    let mut s = format!("StreamTitle='{}';", title.replace('\'', "\u{2019}"));
    while s.len() > 255 * 16 {
        s.pop();
    }
    let n = s.len().div_ceil(16);
    let mut v = BytesMut::with_capacity(1 + n * 16);
    v.extend_from_slice(&[n as u8]);
    v.extend_from_slice(s.as_bytes());
    v.resize(1 + n * 16, 0);
    v.freeze()
}

/// Splits audio so a metadata block lands every METAINT bytes. Title sent first time and on change; else 0-length block.
pub struct IcyInjector {
    until: usize,
    meta: watch::Receiver<String>,
    first: bool,
}

impl IcyInjector {
    pub fn new(meta: watch::Receiver<String>) -> Self {
        Self {
            until: METAINT,
            meta,
            first: true,
        }
    }

    pub fn process(&mut self, mut chunk: Bytes) -> Vec<Bytes> {
        let mut out = Vec::new();
        while !chunk.is_empty() {
            let take = self.until.min(chunk.len());
            out.push(chunk.split_to(take));
            self.until -= take;
            if self.until == 0 {
                self.until = METAINT;
                let changed = self.meta.has_changed().unwrap_or(false);
                if self.first || changed {
                    self.first = false;
                    out.push(meta_block(&self.meta.borrow_and_update()));
                } else {
                    out.push(Bytes::from_static(&[0]));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_on_boundary() {
        let (tx, rx) = watch::channel("A - B".to_string());
        let mut inj = IcyInjector::new(rx);
        let out: Vec<u8> = inj.process(Bytes::from(vec![7u8; METAINT + 10])).concat();
        let blk = meta_block("A - B");
        assert_eq!(&out[METAINT..METAINT + blk.len()], &blk[..]);
        assert_eq!(out.len(), METAINT + blk.len() + 10);
        // unchanged -> zero byte; changed -> new title
        let out = inj.process(Bytes::from(vec![7u8; METAINT])).concat();
        assert_eq!(out[METAINT - 10], 0);
        tx.send("C".into()).unwrap();
        let out = inj.process(Bytes::from(vec![7u8; METAINT])).concat();
        let c = meta_block("C");
        assert_eq!(&out[METAINT - 10..METAINT - 10 + c.len()], &c[..]);
        assert_eq!(blk.len() % 16, 1);
    }
}
