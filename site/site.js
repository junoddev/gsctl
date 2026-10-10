// Install tabs (arrow keys move between them) and copy buttons.
const tabs = [...document.querySelectorAll('[role="tab"]')];
function select(tab) {
  for (const t of tabs) {
    const on = t === tab;
    t.setAttribute('aria-selected', on);
    t.tabIndex = on ? 0 : -1;
    document.getElementById(t.getAttribute('aria-controls')).hidden = !on;
  }
}
tabs.forEach((tab, i) => {
  tab.addEventListener('click', () => select(tab));
  tab.addEventListener('keydown', e => {
    const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
    if (!step) return;
    const next = tabs[(i + step + tabs.length) % tabs.length];
    select(next);
    next.focus();
  });
});

for (const button of document.querySelectorAll('.copy')) {
  button.addEventListener('click', async () => {
    const text = button.parentElement.querySelector('code').innerText;
    try {
      await navigator.clipboard.writeText(text);
      button.textContent = 'copied';
    } catch {
      button.textContent = 'select + copy';
    }
    button.classList.add('done');
    setTimeout(() => { button.textContent = 'copy'; button.classList.remove('done'); }, 1500);
  });
}
