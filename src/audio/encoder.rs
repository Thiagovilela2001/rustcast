use bytes::Bytes;
use mp3lame_encoder::{
    Bitrate, Builder, Encoder, InterleavedPcm, Quality, max_required_buffer_size,
};

pub struct Mp3(Encoder);

impl Mp3 {
    pub fn new(kbps: u32) -> Self {
        let brate = match kbps {
            ..=64 => Bitrate::Kbps64,
            65..=96 => Bitrate::Kbps96,
            97..=128 => Bitrate::Kbps128,
            129..=192 => Bitrate::Kbps192,
            193..=256 => Bitrate::Kbps256,
            _ => Bitrate::Kbps320,
        };
        let mut b = Builder::new().expect("lame builder");
        b.set_num_channels(2).unwrap();
        b.set_sample_rate(super::deck::RATE).unwrap();
        b.set_brate(brate).unwrap();
        b.set_quality(Quality::Good).unwrap();
        Self(b.build().expect("lame init"))
    }

    pub fn encode(&mut self, pcm: &[f32]) -> Bytes {
        let mut out = Vec::with_capacity(max_required_buffer_size(pcm.len() / 2));
        let _ = self.0.encode_to_vec(InterleavedPcm(pcm), &mut out);
        out.into()
    }
}
