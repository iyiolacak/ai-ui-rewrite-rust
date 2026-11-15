// The "24×" in the headline rattles left and right between "opens" and "faster", striking them like walls;
// each strike carries along the line and the words return firmly to their places. Horizontal only.
// The physics is in motion/crashLine.ts. This file only measures and paints.
//
// The headline stays ordinary HTML text: the words are inline-block spans, motion is a transform
// (no layout, nothing in the accessibility tree or the indexed text changes), and at rest every
// inline style is removed. Reduced motion skips it entirely; it also stops at once if the preference
// changes, the tab is hidden or the layout resizes. It runs once, after the headline is on screen,
// and replays when a pointer that can hover moves onto the number.
import { CrashLine, type BodyLayout } from './motion/crashLine.ts';

const root = document.querySelector<HTMLElement>('[data-crash]');
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
const canHover = window.matchMedia('(hover: hover) and (pointer: fine)');
/** Time to land on the headline before the crash, ms. */
const START_DELAY = 700;

if (root && !reducedMotion.matches) {
  const words = Array.from(root.querySelectorAll<HTMLElement>('[data-crash-word]'));
  const sourceElement = root.querySelector<HTMLElement>('[data-crash-source]');
  const source = sourceElement ? words.indexOf(sourceElement) : -1;
  let line: CrashLine | undefined;
  let frame: number | undefined;
  let last = 0;
  let direction = 1;

  const finish = (): void => {
    if (frame !== undefined) cancelAnimationFrame(frame);
    frame = undefined;
    line = undefined;
    for (const w of words) {
      w.style.removeProperty('transform');
      w.style.removeProperty('will-change');
    }
    delete root.dataset['motion'];
  };

  // Rest layout in em, rows by their top edge. Only called at rest, so there are no transforms to undo.
  const measure = (): BodyLayout[] => {
    const em = Number.parseFloat(getComputedStyle(root).fontSize) || 16;
    const rects = words.map((w) => w.getBoundingClientRect());
    const tops: number[] = [];
    return rects.map((r) => {
      let row = tops.findIndex((t) => Math.abs(t - r.top) < em * 0.5);
      if (row < 0) row = tops.push(r.top) - 1;
      return { left: r.left / em, width: r.width / em, row };
    });
  };

  const paint = (): void => {
    if (!line) return;
    for (const [i, x] of line.frame().entries()) {
      words[i]!.style.transform = `translate3d(${x.toFixed(4)}em, 0, 0)`;
    }
  };

  const tick = (time: number): void => {
    frame = undefined;
    if (!line) return;
    line.advance(last ? (time - last) / 1000 : 0);
    last = time;
    paint();
    if (line.settled) finish();
    else frame = requestAnimationFrame(tick);
  };

  const crash = (): void => {
    if (source < 0 || reducedMotion.matches || document.hidden) return;
    if (!line) {
      line = new CrashLine(measure(), source);
      for (const w of words) w.style.willChange = 'transform';
      root.dataset['motion'] = 'running';
    }
    line.launch(line.settings.launchSpeed * direction);
    direction = -direction;
    if (frame === undefined) {
      last = 0;
      frame = requestAnimationFrame(tick);
    }
  };

  reducedMotion.addEventListener('change', finish);
  document.addEventListener('visibilitychange', () => {
    if (document.hidden) finish();
  });

  void document.fonts.ready.then(() => {
    let width = root.getBoundingClientRect().width;
    new ResizeObserver(() => {
      const next = root.getBoundingClientRect().width;
      if (Math.abs(next - width) < 0.5) return;
      width = next;
      finish();
    }).observe(root);
    // A new font face changes every width; whatever was measured is stale.
    document.fonts.addEventListener('loadingdone', finish);

    let timer: number | undefined;
    const io = new IntersectionObserver(
      (entries) => {
        window.clearTimeout(timer);
        if (!entries.some((e) => e.isIntersecting)) return;
        timer = window.setTimeout(() => {
          io.disconnect();
          crash();
        }, START_DELAY);
      },
      { threshold: 0.6 },
    );
    io.observe(root);

    // Replays only once the previous crash has mostly died down, so a jittery pointer cannot pump it.
    sourceElement?.addEventListener('pointerenter', () => {
      if (!canHover.matches) return;
      if (line && line.time < 0.9) return;
      io.disconnect();
      window.clearTimeout(timer);
      crash();
    });
  });
}
