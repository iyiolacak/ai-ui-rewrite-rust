// The scroll counter over the live demo. The WebAssembly app reports the message at the top
// edge of its viewport in `window.__aui.firstVisible` (0-based; see examples/thread/src/web.rs)
// and fires `aui:index` when it changes. Scrolling the 10,000-message thread jumps the number
// by hundreds at a time, which is the point: the list moves that far without a frame stall.
// The numeral is an ordinal, not an animation of the scroll offset, so it changes in the same
// turn as the scroll, not a frame later.
//
// It is shown live during the intro fly too (docked, intro not yet over): the app reports every
// first-visible change, programmatic moves included, so the number runs up while the thread
// flies. The intro ends when WASM sets `__aui.settled` (it landed at the bottom) or the page sets
// `__aui.introDone` (also on cancel by a gesture); the counter then fades ~1.5 s later.
const overlay = document.getElementById('demo-ordinal');
const valueEl = document.getElementById('demo-ordinal-value');
const totalEl = document.getElementById('demo-ordinal-total');
const frame = document.querySelector<HTMLElement>('.demo-frame');
const format = new Intl.NumberFormat('en-US');

let shown = -1;
let hideTimer = 0;
let gesturedAt = -Infinity;
// A held pointer on the demo (the scrollbar thumb, a touch, a drag) keeps the counter up
// until it is released, however slowly the list is being moved.
let holding = false;
let settledOnce = false; // the intro has ended; the landing fade timer has been started

// On wide screens stage.ts flies the demo into place (`demo-travels`) and the counter waits
// until it has docked. Small screens and reduced motion keep the demo in the page without
// that flight, and there it is always in place: the counter must not wait for a dock that
// never comes, or scrolling 10,000 messages would look like a slow scroll through twenty.
const docked = () =>
  !document.body.classList.contains('demo-travels') || document.body.classList.contains('demo-docked');

type Metrics = { firstVisible?: number; messages?: number; userScrolling?: number; settled?: number; introDone?: number };

function metrics(): Metrics | undefined {
  return (window as unknown as { __aui?: Metrics }).__aui;
}

// The visible metric is one-based for people; the app counts from zero.
function render(index: number): void {
  if (valueEl) valueEl.textContent = format.format(index + 1);
}

function introOver(): boolean {
  const aui = metrics();
  return typeof aui?.settled === 'number' || aui?.introDone === 1;
}

function wake(): void {
  if (!overlay || !docked()) return;
  overlay.classList.add('is-visible');
  window.clearTimeout(hideTimer);
  // While the pointer is down, the gesture is not over: no hide timer at all.
  if (holding) return;
  // Otherwise stay up while the gesture keeps moving the list, and leave shortly after it settles.
  hideTimer = window.setTimeout(() => overlay.classList.remove('is-visible'), 1500);
}

function release(): void {
  if (!holding) return;
  holding = false;
  if (!overlay?.classList.contains('is-visible')) return;
  window.clearTimeout(hideTimer);
  hideTimer = window.setTimeout(() => overlay.classList.remove('is-visible'), 900);
}

function gesture(): void {
  gesturedAt = performance.now();
  wake();
}

function read(): void {
  const aui = metrics();
  if (introOver() && !settledOnce) {
    settledOnce = true;
    // The fly has landed (or a gesture took over): keep the numeral up a moment, then fade.
    wake();
  }
  if (typeof aui?.firstVisible !== 'number') return;
  if (typeof aui.messages === 'number' && totalEl) {
    const total = `of ${format.format(aui.messages)} messages`;
    if (totalEl.textContent !== total) totalEl.textContent = total;
  }
  if (aui.firstVisible !== shown) {
    shown = aui.firstVisible;
    render(shown);
    // Every way of moving the thread counts: wheel, keys, a touch flick and dragging the
    // scrollbar inside the canvas. The app marks those as user scrolling; a thread switch
    // or a new turn is not one, and should not summon the counter.
    // The intro fly is programmatic, so it counts too while it runs.
    const flying = !introOver() && document.body.classList.contains('demo-docked');
    if (flying || aui.userScrolling === 1 || performance.now() - gesturedAt < 2200) wake();
  }
  if (!docked()) overlay?.classList.remove('is-visible');
}

// A gesture is the start of the story, so show at wheel time rather than waiting for the
// ordinal to change. Only gestures over the demo count; the page's own wheel is not ours.
window.addEventListener(
  'wheel',
  (event) => {
    const target = event.target;
    if (target instanceof Node && frame?.contains(target)) gesture();
  },
  { passive: true },
);

// A held pointer on the demo (the scrollbar thumb, a touch, a drag) keeps the counter up
// until it is released, however slowly the list is being moved. This only extends an
// already-open counter; pointer-down alone must not summon it, or every click would.
window.addEventListener(
  'pointerdown',
  (event) => {
    const target = event.target;
    if (target instanceof Node && frame?.contains(target)) {
      holding = true;
      window.clearTimeout(hideTimer);
    }
  },
  { passive: true },
);
window.addEventListener('pointerup', release, { passive: true });
window.addEventListener('pointercancel', release, { passive: true });
// A pointer released outside the window never reports back; a blur ends the gesture.
window.addEventListener('blur', release);

const scrollKeys = new Set(['PageDown', 'PageUp', 'Home', 'End', 'ArrowDown', 'ArrowUp', ' ']);
window.addEventListener(
  'keydown',
  (event) => {
    // Keys in the thread's canvas scroll it; keys in the composer's editor do not.
    if (event.target instanceof HTMLCanvasElement && scrollKeys.has(event.key)) gesture();
  },
  true,
);

// A scrollbar drag begins inside the canvas and is handled by the app, so the ordinal change
// is the only signal the page gets; the app's `userScrolling` metric tells it that was a person.
window.addEventListener('aui:index', read);
// The app can boot before this script runs, so read what it has already published, and keep
// a slow fallback for a missed event (a thread switch, or a metric set during a model import).
read();
window.setInterval(read, 400);
