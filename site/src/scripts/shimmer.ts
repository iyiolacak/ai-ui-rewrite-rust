// A wave of fading digits across the "10,000" in the hero, now and then.
// The first wave comes once the headline has been on screen for the time a reader needs
// to read it (about 230 words a minute). After that, a wave every 12 to 22 seconds, and not
// every time. Reduced motion stops the animation in CSS; the classes are still toggled.
const WPM = 230;
const headline = document.getElementById('hero');
let timer: number | undefined;

// Three passes: 3 x 1.6 s, plus the last digit's 0.45 s delay, with a little slack.
const WAVE_MS = 3 * 1600 + 5 * 90 + 300;

function wave(): void {
  document.querySelectorAll<HTMLElement>('.num-shimmer').forEach((el) => {
    el.classList.remove('is-wave');
    void el.offsetWidth; // restart the animation if it is running
    el.classList.add('is-wave');
    window.setTimeout(() => el.classList.remove('is-wave'), WAVE_MS);
  });
}

function scheduleNext(): void {
  window.clearTimeout(timer);
  timer = window.setTimeout(() => {
    if (!document.hidden && Math.random() < 0.6) wave();
    scheduleNext();
  }, 12000 + Math.random() * 10000);
}

if (headline) {
  const words = (headline.textContent ?? '').trim().split(/\s+/).length;
  const readMs = Math.max(2500, (words / WPM) * 60000 + 800);
  const io = new IntersectionObserver(
    (entries) => {
      if (!entries.some((e) => e.isIntersecting)) return;
      io.disconnect();
      window.setTimeout(() => {
        wave();
        scheduleNext();
      }, readMs);
    },
    { threshold: 0.6 },
  );
  io.observe(headline);
}
