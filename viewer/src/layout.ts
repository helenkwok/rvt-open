// Page chrome only: the icon rail's panel toggles, the Export menu and the
// phone bottom sheet. Everything about files, the scene and exports stays in
// main.ts; this module never touches them, it only shows and hides panels.

const app = document.getElementById('app')!;
const toggles: Array<[HTMLButtonElement, 'left' | 'right']> = [
  [document.getElementById('toggle-left') as HTMLButtonElement, 'left'],
  [document.getElementById('toggle-right') as HTMLButtonElement, 'right'],
];
const phone = window.matchMedia('(max-width: 760px)');

function setPanel(side: 'left' | 'right', open: boolean): void {
  app.dataset[side] = open ? 'open' : 'closed';
  for (const [btn, s] of toggles) {
    if (s === side) btn.setAttribute('aria-pressed', String(open));
  }
}

// On a phone the panels share one bottom sheet, so at most one is open, and
// both start closed so the model and the empty state have the screen.
function applyBreakpoint(): void {
  if (phone.matches) {
    setPanel('left', false);
    setPanel('right', false);
  } else {
    setPanel('left', true);
    setPanel('right', true);
  }
}
applyBreakpoint();
phone.addEventListener('change', applyBreakpoint);

for (const [btn, side] of toggles) {
  btn.addEventListener('click', () => {
    const open = app.dataset[side] !== 'open';
    if (phone.matches && open) setPanel(side === 'left' ? 'right' : 'left', false);
    setPanel(side, open);
  });
}

document.getElementById('open-file')!.addEventListener('click', () => {
  (document.getElementById('file-input') as HTMLInputElement).click();
});

// ---------- Export menu ----------
// A disclosure: it closes after an export starts, on Escape, on a click
// outside it, or from its own button.
const exportToggle = document.getElementById('export-toggle') as HTMLButtonElement;
const exportMenu = document.getElementById('export-menu')!;

const menuOpen = (): boolean => exportToggle.getAttribute('aria-expanded') === 'true';

function setExportMenu(open: boolean, focusToggle = false): void {
  exportMenu.hidden = !open;
  exportToggle.setAttribute('aria-expanded', String(open));
  if (open) {
    const first = exportMenu.querySelector<HTMLElement>('select, button:not(:disabled)');
    first?.focus();
  } else if (focusToggle) {
    exportToggle.focus();
  }
}
exportToggle.addEventListener('click', () => setExportMenu(!menuOpen()));
document.addEventListener('keydown', (ev) => {
  if (ev.key === 'Escape' && menuOpen()) {
    // Escape also clears the selection in main.ts; closing the menu first
    // keeps that from happening behind an open menu.
    ev.stopImmediatePropagation();
    setExportMenu(false, true);
  }
}, true);
document.addEventListener('pointerdown', (ev) => {
  if (!menuOpen()) return;
  const target = ev.target as Node;
  if (!exportMenu.contains(target) && !exportToggle.contains(target)) setExportMenu(false);
});
for (const btn of exportMenu.querySelectorAll<HTMLButtonElement>('.export-btn')) {
  btn.addEventListener('click', () => setExportMenu(false, true));
}
