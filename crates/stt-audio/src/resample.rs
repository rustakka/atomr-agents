//! Sample-rate conversion via `rubato`.

use atomr_agents_stt_core::{PcmBuffer, SttError};
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

/// Resample a mono PCM buffer to the target sample rate. Returns
/// the input unchanged if the rates already match.
pub fn resample_mono(pcm: &PcmBuffer, target_sr: u32) -> Result<PcmBuffer, SttError> {
    if pcm.channels != 1 {
        return Err(SttError::decode(
            "resample_mono requires a mono input; mix down first via decode::to_mono",
        ));
    }
    if pcm.sample_rate == target_sr {
        return Ok(pcm.clone());
    }
    let ratio = target_sr as f64 / pcm.sample_rate as f64;
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        oversampling_factor: 256,
        interpolation: SincInterpolationType::Linear,
        window: WindowFunction::Blackman2,
    };
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, pcm.samples.len(), 1)
        .map_err(|e| SttError::decode(format!("rubato init: {e}")))?;
    let waves_in = vec![pcm.samples.clone()];
    let waves_out = resampler
        .process(&waves_in, None)
        .map_err(|e| SttError::decode(format!("rubato process: {e}")))?;
    Ok(PcmBuffer::new(
        waves_out.into_iter().next().unwrap_or_default(),
        target_sr,
        1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_when_rates_match() {
        let pcm = PcmBuffer::new(vec![0.1, -0.1, 0.5, -0.5], 16_000, 1);
        let out = resample_mono(&pcm, 16_000).unwrap();
        assert_eq!(out.samples, pcm.samples);
        assert_eq!(out.sample_rate, 16_000);
    }
}
