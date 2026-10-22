// AudioWorklet: hands what reaches the cable bus to the main thread in ~10 ms stereo blocks
// (interleaved float32), on their way to the microphone filter. Its own output stays silent.
const FRAMES = 512;

class PcmTap extends AudioWorkletProcessor {
  constructor() {
    super();
    this.block = new Float32Array(FRAMES * 2);
    this.n = 0;
  }

  process([input]) {
    const left = input[0];
    const right = input[1] ?? left;
    const length = left?.length ?? 128; // no input connected: silence
    for (let i = 0; i < length; i++) {
      this.block[this.n++] = left ? left[i] : 0;
      this.block[this.n++] = right ? right[i] : 0;
    }
    if (this.n >= this.block.length) {
      this.port.postMessage(this.block, [this.block.buffer]);
      this.block = new Float32Array(FRAMES * 2);
      this.n = 0;
    }
    return true;
  }
}

registerProcessor('pcm-tap', PcmTap);
