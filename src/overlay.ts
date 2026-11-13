import { listen } from "@tauri-apps/api/event";
import "./theme.css";
import "./overlay.css";

type OverlayState = "recording" | "transcribing" | "error";
interface StatePayload {
  state: OverlayState;
  message?: string;
}

const BAR_COUNT = 24;
const MIN_HEIGHT = 2;
const MAX_HEIGHT = 28;
/** Bar height is sqrt(rms) * K. Normal speech (rms 0.05–0.09) fills about 60–80%. */
const K = 75;

/** Bar height in px for one RMS level. */
export function barHeight(rms: number): number {
  const height = Math.sqrt(Math.max(0, rms)) * K;
  return Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, height));
}

const pill = document.querySelector<HTMLElement>(".pill")!;
const barsEl = document.querySelector<HTMLElement>("#bars")!;
const errorEl = document.querySelector<HTMLElement>("#error")!;

const bars: HTMLElement[] = [];
for (let i = 0; i < BAR_COUNT; i++) {
  const bar = document.createElement("div");
  bar.className = "bar";
  barsEl.append(bar);
  bars.push(bar);
}

/** The last 24 levels, oldest first. The newest is drawn on the right. */
let levels: number[] = new Array(BAR_COUNT).fill(0);

function draw(): void {
  bars.forEach((bar, i) => {
    bar.style.height = `${barHeight(levels[i])}px`;
  });
}

function setState({ state, message }: StatePayload): void {
  pill.dataset.state = state;
  errorEl.textContent = state === "error" ? (message ?? "Something went wrong") : "";
  if (state === "recording") {
    levels = new Array(BAR_COUNT).fill(0);
    draw();
  }
}

void listen<StatePayload>("overlay-state", (event) => setState(event.payload));
void listen<{ rms: number }>("level", (event) => {
  levels.push(event.payload.rms);
  levels.shift();
  draw();
});

draw();
