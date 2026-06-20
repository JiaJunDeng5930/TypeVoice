use crate::doubao_asr;

pub(crate) fn pcm_peak_abs(pcm: &[u8]) -> i32 {
    pcm.chunks_exact(2)
        .map(|bytes| i32::from(i16::from_le_bytes([bytes[0], bytes[1]])).abs())
        .max()
        .unwrap_or(0)
}

pub(crate) fn pcm_bytes_for_ms(ms: u64) -> usize {
    let bytes_per_second = doubao_asr::PCM_SAMPLE_RATE as u64
        * u64::from(doubao_asr::PCM_CHANNELS)
        * u64::from(doubao_asr::PCM_BITS / 8);
    ((bytes_per_second * ms) / 1000) as usize
}
