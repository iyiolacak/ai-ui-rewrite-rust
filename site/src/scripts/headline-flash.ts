// Minimalist assistant-ui tooltip: appears automatically 2s after the hero is
// visible, and on hover (CSS sibling) and focus. One pill, no border.
const link = document.querySelector<HTMLElement>('.hl-link');
const tip = document.querySelector<HTMLElement>('.hl-tip');
const reduced = window.matchMedia('(prefers-reduced-motion: reduce)');

if (link && tip) {
  let shown = false;
  const io = new IntersectionObserver((entries) => {
    if (!entries.some((e) => e.isIntersecting) || shown) return;
    shown = true;
    io.disconnect();
    // After 2s on screen, show once so the explanation has always been seen.
    setTimeout(() => {
      tip.classList.add('is-auto');
      setTimeout(() => tip.classList.remove('is-auto'), 3500);
    }, reduced.matches ? 0 : 2000);
  }, { threshold: .5 });
  io.observe(link);
}
