// The first screen under the hero IS the live demo. As you scroll it travels down and grows, on a curve, until it
// sits centred as the main element; scroll on and it moves away like any other block.
//
// Nothing is forced: no modal, no scroll lock, no stand-in image. The demo's frame is a sticky block in the page (CSS)
// laid out once at its final size, so the canvas never resizes (pixel-snap.js sizes the canvas from the frame's
// rect, and a resize would make Slint re-layout every frame). Until it is docked the frame is only moved and
// scaled with a transform, from where the strip's first screen sits (cached per layout) to its own slot. While it is
// small it ignores the pointer, because a scaled canvas would mis-map the mouse; clicking it scrolls to the demo.
//
// Once docked, the demo autoplays once — from the top to the very bottom —
// then hands the thread to the pointer for good. The accent ring is the only
// takeover cue while it flies; the ordinal appears only after the thread
// settles, so it reads as part of the demo, not the landing.
//
// The fly is a one-time intro. Each position is an EVENT, not a sticky value: the page writes
// `__aui.scrollTo` and dispatches `aui:scroll`, and the demo applies it once. The last step is an exact 1,
// which the demo turns into a real scroll-to-bottom. Any user input over the canvas (wheel, pointer down,
// scroll keys) cancels the intro. After it finishes or is cancelled the page never sends again, so you can
// scroll the thread anywhere by hand and it stays where you leave it. Undocking doesn't yank it back.
//
// Perf: the flight is transform-only (compositor thread). Geometry is cached
// per layout, so scrolling costs no layout or style recalc.
import { gsap } from 'gsap';
import { ScrollTrigger } from 'gsap/ScrollTrigger';

gsap.registerPlugin(ScrollTrigger);

const screens = document.querySelector<HTMLElement>('.screens');
const screen = document.querySelector<HTMLElement>('.screen');
const run = document.querySelector<HTMLElement>('.demo-run');
const sticky = document.querySelector<HTMLElement>('.demo-sticky');
const slot = document.querySelector<HTMLElement>('.demo-slot');
const frame = document.querySelector<HTMLElement>('.demo-frame');
const hint = document.querySelector<HTMLElement>('.stage-hint');

const wide = matchMedia('(min-width: 900px) and (min-height: 560px)');
const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const curve = gsap.parseEase('power2.inOut');

const raw = { p: 0 }; // from the scroll position
const smooth = { p: 0 }; // follows `raw` a little behind, so a mouse-wheel notch is not a jump
let docked = false;
let flying = false;
let started = false; // the flight begins once the canvas has been laid out at its full size
let settle = 0; // frames to stay untransformed after a resize, so pixel-snap.js measures the real box
let autoplayId = 0; // rAF id while the intro fly is running
let autoplayLast = 0;
let autoplayFrom = 0;
let autoplayTo = 1;
let autoplayT0 = 0;
let autoPlaying = false;
let autoplayFramesAtBottom = 0; // consecutive frames at f=1
// The fly is a one-time intro. Once it finishes or is cancelled, the page
// never writes `scrollFraction` again — the thread is the pointer's.
let introDone = false;
let cancelIntro = false;
const canvas = document.getElementById('canvas');
const ordinal = document.getElementById('demo-ordinal') as HTMLElement | null;

// Cached flight geometry: viewport-space delta from slot to source screen and
// the source-to-slot width ratio. Both ends scroll together, so the delta is
// constant for a given layout — no per-frame reads.
let geom: { dx: number; dy: number; k0: number } | null = null;
let lastT = -1;

function measureGeom(): void {
  if (!screen || !slot) return;
  try {
    const s = screen.getBoundingClientRect();
    const to = slot.getBoundingClientRect();
    if (!s.width || !to.width) return;
    // The slot lives in a sticky box. The flight only runs while it is NOT stuck (it docks when the run's top
    // reaches the viewport top), but this can be called at any scroll position (a ScrollTrigger refresh, a
    // resize). When stuck, or pushed down at the end of the run, the slot sits up to 50vh lower than its
    // in-flow place, and caching that would fly the frame onto the hero text on the way back up. Unstuck, the
    // sticky box's top equals the run's top, so subtract the difference to always measure the in-flow slot.
    const stuck = sticky && run ? sticky.getBoundingClientRect().top - run.getBoundingClientRect().top : 0;
    geom = { dx: s.left - to.left, dy: s.top - (to.top - stuck), k0: s.width / to.width };
  } catch {
    /* measure again on the next refresh */
  }
}

function aui(): { scrollTo?: number; settled?: number; introDone?: number } | undefined {
  return (window as unknown as { __aui?: { scrollTo?: number; settled?: number; introDone?: number } }).__aui;
}

function readSettled(): boolean {
  return aui()?.settled === 1;
}

function setDocked(on: boolean): void {
  if (on === docked) return;
  docked = on;
  hint?.classList.toggle('is-on', on);
  document.body.classList.toggle('demo-docked', on);
  frame?.classList.toggle('is-docked', on);
  if (on) {
    ordinal?.classList.remove('is-docked');
    if (!introDone) startAutoplay();
  } else {
    stopAutoplay();
    ordinal?.classList.remove('is-docked');
  }
}

function pushScrub(f: number): void {
  const w = aui();
  if (!w) return;
  w.scrollTo = f;
  window.dispatchEvent(new Event('aui:scroll'));
}

function stopAutoplay(): void {
  if (autoplayId) cancelAnimationFrame(autoplayId);
  autoplayId = 0;
  autoPlaying = false;
  autoplayFramesAtBottom = 0;
}

// The counter overlay (ordinal.ts) waits for this: it stays hidden only while the intro auto-flies.
function markIntroOver(): void {
  const w = aui();
  if (w) w.introDone = 1;
}

function finishIntro(): void {
  introDone = true;
  markIntroOver();
  stopAutoplay();
  // Nothing to hand back: the page sends no more positions.
}

// Ease-out cubic: fast at first, settling into the bottom.
function easeOutCubic(x: number): number {
  return 1 - Math.pow(1 - x, 3);
}

function startAutoplay(): void {
  stopAutoplay();
  cancelIntro = false;
  // Fly from the top (0) to the very bottom (1). These are the ONLY positions
  // the page ever sends; after it finishes or is cancelled, the thread is yours.
  autoplayFrom = 0;
  autoplayTo = 1;
  autoplayT0 = performance.now();
  autoplayLast = 0;
  autoplayFramesAtBottom = 0;

  autoPlaying = true;
  const DURATION = 1400;
  const step = (t: number) => {
    if (!autoPlaying || cancelIntro) {
      // A real wheel gesture over the canvas during the fly cancels it.
      if (cancelIntro) finishIntro();
      return;
    }
    const x = Math.min(1, (t - autoplayT0) / DURATION);
    const f = autoplayFrom + (autoplayTo - autoplayFrom) * easeOutCubic(x);
    if (f !== autoplayLast) {
      autoplayLast = f; // the final step is exactly 1 and is sent once
      pushScrub(f);
    }
    if (x >= 1) {
      // Wait for web.rs to ACK that a frame was painted at 1.
      autoplayFramesAtBottom++;
      if (readSettled() || autoplayFramesAtBottom > 90) {
        finishIntro();
        return;
      }
      autoplayId = requestAnimationFrame(step);
      return;
    }
    autoplayId = requestAnimationFrame(step);
  };
  autoplayId = requestAnimationFrame(step);
}

function land(): void {
  if (!flying && lastT === 1) return;
  flying = false;
  lastT = 1;
  frame!.classList.remove('is-flying');
  for (const p of ['transform', 'border-radius', 'mask-image', '-webkit-mask-image']) frame!.style.removeProperty(p);
}

/** Transform-only flight: cached geometry, no layout/style reads, no repaint triggers. */
function render(): void {
  if (!started) {
    // pixel-snap.js measures the frame's rect once, when the demo starts. A scaled frame would make it size the canvas
    // to the scaled box, so wait until that has happened (it marks the canvas), or until the first frame is drawn
    // (it did not load; CSS sizes the canvas). If the demo never starts there is nothing to move, and the screenshot stays.
    const aui = window as unknown as { __aui?: { firstFrame?: number } };
    if (!canvas?.hasAttribute('data-aui-snap') && !aui.__aui?.firstFrame) return;
    started = true;
    // pixel-snap.js measures again on the next frames (ResizeObserver's first callback, then a requestAnimationFrame); let them see the real box.
    settle = 8;
    const img = screen!.querySelector('img');
    if (img) img.style.opacity = '0';
  }
  if (settle > 0) {
    settle--;
    land();
    return;
  }
  const t = curve(Math.min(1, Math.max(0, smooth.p)));
  if (t >= 0.9995) {
    land();
    setDocked(true);
    return;
  }
  setDocked(false);
  if (!flying) {
    flying = true;
    lastT = -1; // force one paint on take-off
    frame!.classList.add('is-flying');
  }
  if (!geom) measureGeom();
  if (!geom) return;
  // Skip the DOM write when converged: the ticker keeps running but costs ~nothing.
  if (Math.abs(t - lastT) < 0.00015) return;
  lastT = t;
  const inv = 1 - t;
  const k = geom.k0 + (1 - geom.k0) * t;
  frame!.style.transform = `translate3d(${(geom.dx * inv).toFixed(2)}px, ${(geom.dy * inv).toFixed(2)}px, 0) scale(${k.toFixed(5)})`;
}

function enable(): void {
  const strip = screen!;
  // The live demo takes this screen's place: keep the box (it is the link target and the strip's rhythm), drop the picture.
  strip.style.background = 'transparent';
  document.body.classList.add('demo-travels');
  measureGeom();

  // 0 when the strip's top is 85% down the viewport, 1 when the demo's pin starts (the frame is then centred).
  ScrollTrigger.create({
    trigger: screens,
    start: 'top 85%',
    endTrigger: run,
    end: 'top top',
    onUpdate: (self) => {
      raw.p = self.progress;
    },
    onRefresh: (self) => {
      raw.p = self.progress;
      smooth.p = self.progress;
      // Layout changed: re-cache the endpoints and repaint once.
      geom = null;
      lastT = -1;
      measureGeom();
    },
  });

  // The scrub trigger is gone: it made page scroll seem to scroll the thread
  // and made it hard to reach the composer. Now the demo autoplays once on
  // dock — from mid-thread to the newest — then the pointer owns it.
  gsap.ticker.add(() => {
    // Ease toward the scroll-derived value; the gap closes in ~0.25 s, so it still feels attached to the page.
    // Early-out: when converged, render() skips its DOM write too.
    const d = raw.p - smooth.p;
    if (Math.abs(d) < 0.0004) {
      if (smooth.p !== raw.p) {
        smooth.p = raw.p;
        render();
      } else if (flying || !docked) {
        render();
      }
      return;
    }
    smooth.p += d * 0.18;
    if (Math.abs(raw.p - smooth.p) < 0.0004) smooth.p = raw.p;
    render();
  });

  // A resize makes pixel-snap.js measure the box again (next frame); let it see the box untransformed.
  window.addEventListener('resize', () => {
    settle = 4;
    geom = null;
    lastT = -1;
    land();
    // Re-measure after the untransformed frames so the next take-off is exact.
    window.setTimeout(measureGeom, 120);
  });

  // Wheel: the canvas gets the event. During the intro, a wheel over the
  // canvas cancels it (the user wants to scroll by hand). After the intro,
  // there is no lock at all — the page and the thread both scroll normally.
  let lastWheel = 0;
  let owner: 'demo' | 'page' = 'page';
  window.addEventListener('wheel', (e) => {
    const now = performance.now();
    if (now - lastWheel > 150) owner = docked && frame!.contains(e.target as Node) ? 'demo' : 'page';
    lastWheel = now;
    if (owner === 'demo') {
      hint?.classList.remove('is-on');
      if (autoPlaying) {
        cancelIntro = true;
        markIntroOver();
      }
    }
  }, { passive: true, capture: true });
  // A scrollbar drag, touch or click on the frame, or a scroll key, also means "I'm driving".
  const cancelOnInput = () => {
    hint?.classList.remove('is-on');
    if (autoPlaying) {
      cancelIntro = true;
      markIntroOver();
    }
  };
  window.addEventListener('pointerdown', (e) => {
    if (docked && frame!.contains(e.target as Node)) cancelOnInput();
  }, { passive: true, capture: true });
  const scrollKeys = new Set(['PageUp', 'PageDown', 'Home', 'End', 'ArrowUp', 'ArrowDown', ' ']);
  window.addEventListener('keydown', (e) => {
    if (scrollKeys.has(e.key) && docked && frame!.contains(e.target as Node)) cancelOnInput();
  }, { passive: true, capture: true });
}

// Small screens and reduced motion keep the plain layout: the demo sits in the page and the screenshot links to it.
if (screens && screen && run && slot && frame && wide.matches && !reduced.matches) enable();
