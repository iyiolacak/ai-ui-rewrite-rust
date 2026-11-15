// Pixel-snapped canvas layout for the thread demo.
//
// Why: the demo draws into a WebGL canvas whose backing store is a whole number of device pixels.
// When the canvas box is fractional in device pixels (a centred container, `vh` heights, a 1.25x or
// 1.5x display), Firefox composites it with bilinear resampling, which softens every glyph; Chrome
// snaps the box and hides this. This helper lays the canvas out on whole device pixels, so backing
// store and box match 1:1 and no resampling happens.
//
// Usage (before init(), canvas must keep id="canvas" for Slint):
//   import { pixelSnap } from "./pixel-snap.js";
//   pixelSnap(document.getElementById("canvas"));
// The canvas parent must have a definite size (it is the area the canvas fills). The helper makes
// it `position: relative` if it is static. The canvas takes the parent's padding box.
//
// It works through CSS custom properties and one injected rule with `!important`, because winit
// rewrites the canvas's inline `width`/`height` on every resize.

const STYLE_ID = "aui-pixel-snap-style";
const RULE =
  "canvas[data-aui-snap]{position:absolute!important;display:block!important;" +
  "left:var(--aui-x,0px)!important;top:var(--aui-y,0px)!important;" +
  "width:var(--aui-w,100%)!important;height:var(--aui-h,100%)!important;" +
  "max-width:none!important;max-height:none!important;margin:0!important}";

// Snap [start, start + size) (CSS px, in page coordinates) inward to whole device pixels.
// Returns the offset from `start` and the length, both in CSS px.
export function snapSpan(start, size, dpr) {
  const eps = 1e-6;
  const a = Math.ceil(start * dpr - eps);
  const b = Math.floor((start + size) * dpr + eps);
  const len = Math.max(1, b - a);
  return { offset: (a - start * dpr) / dpr, length: len / dpr, devicePixels: len };
}

export function pixelSnap(canvas) {
  const box = canvas.parentElement;
  if (!box) throw new Error("pixelSnap: canvas has no parent element");
  if (!document.getElementById(STYLE_ID)) {
    const s = document.createElement("style");
    s.id = STYLE_ID;
    s.textContent = RULE;
    document.head.append(s);
  }
  if (getComputedStyle(box).position === "static") box.style.position = "relative";
  canvas.setAttribute("data-aui-snap", "");

  let mql = null;
  let pending = 0;

  function layout() {
    pending = 0;
    const dpr = window.devicePixelRatio || 1;
    const cs = getComputedStyle(box);
    const r = box.getBoundingClientRect();
    const bl = parseFloat(cs.borderLeftWidth) || 0;
    const bt = parseFloat(cs.borderTopWidth) || 0;
    const w = r.width - bl - (parseFloat(cs.borderRightWidth) || 0);
    const h = r.height - bt - (parseFloat(cs.borderBottomWidth) || 0);
    // Page coordinates, with the scroll position taken at device-pixel granularity, so scrolling
    // does not change the answer.
    const sx = Math.round(window.scrollX * dpr) / dpr;
    const sy = Math.round(window.scrollY * dpr) / dpr;
    const x = snapSpan(r.left + bl + sx, w, dpr);
    const y = snapSpan(r.top + bt + sy, h, dpr);
    const st = canvas.style;
    st.setProperty("--aui-x", `${x.offset}px`);
    st.setProperty("--aui-y", `${y.offset}px`);
    st.setProperty("--aui-w", `${x.length}px`);
    st.setProperty("--aui-h", `${y.length}px`);
  }
  function schedule() {
    if (!pending) pending = requestAnimationFrame(layout);
  }
  function watchDpr() {
    if (mql) mql.removeEventListener("change", onDpr);
    mql = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    mql.addEventListener("change", onDpr);
  }
  function onDpr() {
    watchDpr();
    schedule();
  }

  const ro = new ResizeObserver(schedule);
  ro.observe(box);
  window.addEventListener("resize", schedule);
  watchDpr();
  layout();

  return function dispose() {
    ro.disconnect();
    window.removeEventListener("resize", schedule);
    if (mql) mql.removeEventListener("change", onDpr);
    if (pending) cancelAnimationFrame(pending);
    canvas.removeAttribute("data-aui-snap");
    for (const p of ["--aui-x", "--aui-y", "--aui-w", "--aui-h"]) canvas.style.removeProperty(p);
  };
}
