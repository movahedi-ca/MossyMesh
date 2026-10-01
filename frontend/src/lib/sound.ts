/**
 * Lightweight WebAudio sound effects for chess events (#23).
 *
 * No audio assets are shipped; every effect is synthesized with an
 * oscillator so the offline PWA stays asset-free. The AudioContext is
 * created lazily inside user-gesture call paths (square clicks), so
 * autoplay policies are satisfied. Calls are safe before any gesture:
 * they simply no-op if the context cannot start.
 */

let ctx: AudioContext | null = null;

function audio(): AudioContext | null {
  if (typeof window === "undefined") return null;
  const AC =
    window.AudioContext ??
    (window as unknown as { webkitAudioContext?: typeof AudioContext })
      .webkitAudioContext;
  if (!AC) return null;
  if (!ctx) ctx = new AC();
  if (ctx.state === "suspended") void ctx.resume();
  return ctx;
}

function beep(
  frequency: number,
  delaySeconds: number,
  duration = 0.12,
  type: OscillatorType = "sine",
): void {
  const ac = audio();
  if (!ac) return;
  const start = ac.currentTime + delaySeconds;
  const osc = ac.createOscillator();
  const gain = ac.createGain();
  osc.type = type;
  osc.frequency.value = frequency;
  gain.gain.setValueAtTime(0.0001, start);
  gain.gain.exponentialRampToValueAtTime(0.22, start + 0.015);
  gain.gain.exponentialRampToValueAtTime(0.0001, start + duration);
  osc.connect(gain);
  gain.connect(ac.destination);
  osc.start(start);
  osc.stop(start + duration + 0.05);
}

/** Two soft ticks, played after a legal move lands. */
export function playMoveSound(): void {
  beep(660, 0);
  beep(880, 0.07);
}

/** Short rising triad, played on checkmate. */
export function playCheckmateSound(): void {
  beep(523.25, 0);
  beep(659.25, 0.12);
  beep(783.99, 0.24, 0.28);
}
