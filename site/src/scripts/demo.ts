// Loads the WebAssembly demo as soon as the page has loaded, so there is nothing to click.
// The hero paints first; the download starts after the window load event.
const statusEl = document.getElementById('demo-status');
const subEl = document.getElementById('demo-status-sub');
const cover = document.getElementById('demo-cover');
const base = new URL('wasm/', document.baseURI);
let started = false;

function hasWebGL(): boolean {
  try {
    const c = document.createElement('canvas');
    return !!(c.getContext('webgl2') || c.getContext('webgl'));
  } catch {
    return false;
  }
}

// winit sizes the canvas from ResizeObserver's devicePixelContentBoxSize but takes its scale factor
// from devicePixelRatio. Under emulated device metrics (DevTools device mode, Playwright's
// deviceScaleFactor) the first stays at CSS pixels while the second is e.g. 2.6, so Slint computes a
// logical window 2.6x too small and draws everything 2.6x too large, with compact mode wrongly
// reading a 145 px window. Real displays report both consistently and are left alone: only a
// size that disagrees with contentBox * devicePixelRatio by over 15% is replaced by that product.
function reconcileDevicePixelSize(): void {
  try {
    const proto = ResizeObserverEntry.prototype;
    const desc = Object.getOwnPropertyDescriptor(proto, 'devicePixelContentBoxSize');
    if (!desc?.get) return;
    const getter = desc.get;
    Object.defineProperty(proto, 'devicePixelContentBoxSize', {
      configurable: true,
      enumerable: desc.enumerable,
      get(this: ResizeObserverEntry) {
        const real = getter.call(this) as ReadonlyArray<ResizeObserverSize>;
        const css = this.contentBoxSize?.[0];
        const dpr = window.devicePixelRatio || 1;
        if (this.target.id !== 'canvas' || !css || !real?.[0] || css.inlineSize < 1) return real;
        if (Math.abs(real[0].inlineSize / (css.inlineSize * dpr) - 1) <= 0.15) return real;
        return [{ inlineSize: Math.round(css.inlineSize * dpr), blockSize: Math.round(css.blockSize * dpr) }];
      },
    });
  } catch {
    /* browsers without devicePixelContentBoxSize (Safari) take winit's CSS-pixel fallback, which is consistent */
  }
}

function fail(message: string): void {
  started = false;
  if (statusEl) statusEl.textContent = message;
  if (subEl) subEl.textContent = 'The page above stays readable — the demo loads in the background.';
}

async function load(): Promise<void> {
  if (started) return;
  if (!hasWebGL()) {
    fail('This browser has no WebGL, so the demo cannot run here.');
    return;
  }
  started = true;
  if (statusEl) statusEl.textContent = 'Loading the demo…';
  if (subEl) subEl.textContent = 'Downloading the WebAssembly app — please wait a moment';
  const t0 = performance.now();
  const poll = window.setInterval(() => {
    const s = Math.round((performance.now() - t0) / 1000);
    if (s > 0 && subEl) subEl.textContent = `Still loading… ${s}s — the demo is worth the wait`;
  }, 500);
  (window as unknown as { __AUI_ARGS: string }).__AUI_ARGS = 'history=10000&threads=40';
  reconcileDevicePixelSize();
  try {
    // Whole-device-pixel canvas layout: keeps text sharp in Firefox. Optional, so a missing file is not fatal.
    try {
      const snap = await import(/* @vite-ignore */ new URL('pixel-snap.js', base).href);
      snap.pixelSnap(document.getElementById('canvas'));
    } catch {
      /* fall back to the stylesheet sizing */
    }
    const mod = await import(/* @vite-ignore */ new URL('ai_ui_example_thread.js', base).href);
    await mod.default();
  } catch (e) {
    // winit may unwind through run() as control flow; anything else is a failure.
    if (!/control flow/.test(String(e))) {
      window.clearInterval(poll);
      fail(`Could not start the demo: ${e}`);
      return;
    }
  }
  const aui = window as unknown as { __aui?: { firstFrame?: number } };
  // No focus() call: focusing the canvas would scroll the page to the demo.
  const wait = () => {
    if (aui.__aui?.firstFrame) {
      window.clearInterval(poll);
      cover?.classList.add('is-live');
      setTimeout(() => cover?.setAttribute('hidden', ''), 450);
    } else {
      setTimeout(wait, 50);
    }
  };
  wait();
}

if (document.readyState === 'complete') void load();
else window.addEventListener('load', () => void load(), { once: true });
