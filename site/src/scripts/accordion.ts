// Single-open accordions: one open item per group. The panels animate their height with
// grid-template-rows, so the easing is the CSS curve, not a measured height. Opening uses a
// fast-out, slow-in curve; closing uses an ease-in, so the panel tucks away quickly.
// Each trigger is a real button with aria-expanded, so keyboard and screen readers work too.
document.querySelectorAll<HTMLElement>('[data-accordion]').forEach((group) => {
  const items = Array.from(group.querySelectorAll<HTMLElement>('[data-acc-item]'));

  const setOpen = (item: HTMLElement, open: boolean) => {
    item.classList.toggle('is-open', open);
    item.querySelector('.acc-trigger')?.setAttribute('aria-expanded', String(open));
  };

  items.forEach((item) => {
    const trigger = item.querySelector<HTMLButtonElement>('.acc-trigger');
    trigger?.addEventListener('click', () => {
      const willOpen = !item.classList.contains('is-open');
      items.forEach((other) => setOpen(other, false));
      setOpen(item, willOpen);
    });
  });
});
