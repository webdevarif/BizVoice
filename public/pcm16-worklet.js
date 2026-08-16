// Downsamples the mic to the 16 kHz mono PCM16 the realtime STT socket wants,
// on the audio thread, and posts it in small chunks for streaming.
//
// Why not just ask getUserMedia for 16 kHz: on Windows the sampleRate
// constraint is an ideal hint that is not honoured, and Chromium runs one rate
// per AudioContext — which the waveform analyser and vad-web already share. So
// the device rate is whatever it is (typically 44.1 or 48 kHz) and the
// conversion happens here.
//
// Why not reuse vad-web's frames: its resampler is a box average, which leaves
// audible aliasing in the top band — exactly where Bangla sibilants and
// aspirates live. Fine for a speech/no-speech decision, wrong to hand a
// transcription model. This uses a windowed-sinc low-pass instead.

const TARGET_RATE = 16000;
const CHUNK_SAMPLES = 512; // 32 ms at 16 kHz — 1024 bytes per message
const TAPS = 63; // odd, so the filter has a whole-sample group delay
const CUTOFF = 7600; // just under Nyquist, leaving room for the transition band

/** Windowed-sinc low-pass, normalised to unity DC gain. */
function makeFilter(sampleRate) {
  const fc = CUTOFF / sampleRate; // cycles/sample
  const mid = (TAPS - 1) / 2;
  const h = new Float32Array(TAPS);
  let sum = 0;
  for (let i = 0; i < TAPS; i++) {
    const n = i - mid;
    // sinc, with the removable singularity at n = 0 filled in
    const s = n === 0 ? 2 * fc : Math.sin(2 * Math.PI * fc * n) / (Math.PI * n);
    // Blackman window — ~ -74 dB sidelobes, enough that aliasing is inaudible
    const w =
      0.42 -
      0.5 * Math.cos((2 * Math.PI * i) / (TAPS - 1)) +
      0.08 * Math.cos((4 * Math.PI * i) / (TAPS - 1));
    h[i] = s * w;
    sum += h[i];
  }
  for (let i = 0; i < TAPS; i++) h[i] /= sum;
  return h;
}

class Pcm16Processor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.filter = makeFilter(sampleRate);
    // Ring of the last TAPS input samples, so filtering is continuous across
    // the 128-sample render quanta rather than restarting every block.
    this.history = new Float32Array(TAPS);
    // Fractional read position into the incoming stream. The ratio is rarely
    // an integer (44100/16000 = 2.75625), so this must not be rounded.
    this.pos = 0;
    this.ratio = sampleRate / TARGET_RATE;
    this.out = new Int16Array(CHUNK_SAMPLES);
    this.outLen = 0;
    this.stopped = false;
    this.port.onmessage = (e) => {
      if (e.data === 'stop') this.stopped = true;
    };
  }

  /** Filtered value at an integer input index, reading through the history ring. */
  filterAt(input, idx) {
    let acc = 0;
    for (let t = 0; t < TAPS; t++) {
      const i = idx - t;
      const v = i >= 0 ? input[i] : this.history[TAPS + i];
      acc += v * this.filter[t];
    }
    return acc;
  }

  process(inputs) {
    if (this.stopped) return false;
    const ch = inputs[0] && inputs[0][0];
    if (!ch) return true; // mic not delivering yet; keep the node alive

    // Walk output positions across this block, low-passing then linearly
    // interpolating between neighbouring filtered samples.
    //
    // Stop one sample short of the block end: interpolation needs `i + 1`, and
    // there is no lookahead into the next block. Clamping to the last sample
    // instead would collapse b onto a and silently discard the fraction — at
    // 44.1 kHz (ratio 2.75625) that lands on ~36% of render quanta, i.e. an
    // error impulse 125x/sec in exactly the top band this filter exists to
    // protect. Leaving the sample for the next call costs one sample of delay
    // and nothing else: `pos` then starts in [-1, 0), and filterAt already
    // reads negative indices out of the history ring.
    while (this.pos < ch.length - 1) {
      const i = Math.floor(this.pos);
      const frac = this.pos - i;
      const a = this.filterAt(ch, i);
      const b = this.filterAt(ch, i + 1);
      const v = a + (b - a) * frac;

      // Clamp before scaling: values slightly outside [-1,1] are legal float
      // audio and would wrap to the opposite sign as int16.
      const c = v > 1 ? 1 : v < -1 ? -1 : v;
      this.out[this.outLen++] = c < 0 ? c * 0x8000 : c * 0x7fff;

      if (this.outLen === CHUNK_SAMPLES) {
        const buf = this.out.buffer.slice(0);
        this.port.postMessage(buf, [buf]); // transfer, no copy
        this.outLen = 0;
      }
      this.pos += this.ratio;
    }
    this.pos -= ch.length; // carry the fraction into the next block

    // Keep the tail of this block as history for the next one.
    if (ch.length >= TAPS) {
      this.history.set(ch.subarray(ch.length - TAPS));
    } else {
      this.history.copyWithin(0, ch.length);
      this.history.set(ch, TAPS - ch.length);
    }
    return true;
  }
}

registerProcessor('pcm16', Pcm16Processor);
