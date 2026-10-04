// Small global UI state: tooltip and the workspace-wide repo filter.

export const tip = $state({ show: false, x: 0, y: 0, title: '', lines: [] as string[] });

export function showTip(e: MouseEvent, title: string, lines: string[] = []) {
  tip.show = true;
  tip.x = e.clientX;
  tip.y = e.clientY;
  tip.title = title;
  tip.lines = lines;
}

export function hideTip() {
  tip.show = false;
}

function load(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

export const filters = $state({ repo: load('csi.repo', '') });

export function setRepo(r: string) {
  filters.repo = r;
  try {
    localStorage.setItem('csi.repo', r);
  } catch {
    /* storage unavailable */
  }
}
