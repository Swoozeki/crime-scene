// Color by job (see the data-viz method): heat = one orange ramp, light → dark; age and
// knowledge loss = one blue ramp; people = fixed categorical order + "Other"; health = status
// colors, always paired with a label.
import * as d3 from 'd3';

const dark = () =>
  document.documentElement.dataset.theme === 'dark' ||
  (document.documentElement.dataset.theme !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);

// Orange single-hue ramp (light mode starts near the surface; dark mode is stepped separately).
const HEAT_LIGHT = ['#f6e7de', '#f3c6ad', '#ee9f78', '#e57947', '#d9541c', '#b23f10', '#86300b'];
const HEAT_DARK = ['#3a2a22', '#5e3420', '#86401f', '#b04f1f', '#d9541c', '#ec7a45', '#f7a77c'];
const BLUE_LIGHT = ['#cde2fb', '#9ec5f4', '#6da7ec', '#3987e5', '#256abf', '#184f95', '#0d366b'];
const BLUE_DARK = ['#1d2a3a', '#183f6d', '#1c5cab', '#2a78d6', '#3987e5', '#6da7ec', '#9ec5f4'];

const ramp = (stops: string[]) => d3.scaleLinear<string>().domain(stops.map((_, i) => i / (stops.length - 1))).range(stops).interpolate(d3.interpolateLab).clamp(true);

export function heat(x: number): string {
  return ramp(dark() ? HEAT_DARK : HEAT_LIGHT)(Math.max(0, Math.min(1, x)));
}

export function blue(x: number): string {
  return ramp(dark() ? BLUE_DARK : BLUE_LIGHT)(Math.max(0, Math.min(1, x)));
}

// Validated categorical order (reference palette), light and dark steps.
const CAT_LIGHT = ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300', '#4a3aa7'];
const CAT_DARK = ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#008300', '#9085e9'];
export const OTHER_LIGHT = '#b9b7ae';
export const OTHER_DARK = '#5a5955';

/** People colors: the top 7 (by knowledge) keep a fixed slot; everyone else is "Other". */
export function personScale(topNames: string[]) {
  const slots = new Map(topNames.slice(0, 7).map((n, i) => [n, i]));
  return (name: string | null | undefined) => {
    const i = name ? slots.get(name) : undefined;
    const d = dark();
    return i === undefined ? (d ? OTHER_DARK : OTHER_LIGHT) : (d ? CAT_DARK : CAT_LIGHT)[i];
  };
}

export type Status = 'good' | 'warning' | 'serious' | 'critical';
export const STATUS: Record<Status, string> = { good: '#0ca30c', warning: '#fab219', serious: '#ec835a', critical: '#d03b3b' };

export function healthStatus(h: number): Status {
  if (h >= 8) return 'good';
  if (h >= 6) return 'warning';
  if (h >= 4) return 'serious';
  return 'critical';
}

export const healthLabel: Record<Status, string> = { good: 'healthy', warning: 'watch', serious: 'unhealthy', critical: 'alarming' };

/** People ranked by lines of code they are main developer of, from the hotspot tree.
 *  Every people-colored view uses this one order so a person keeps their color everywhere. */
export function topPeople(tree: any): string[] {
  const m = new Map<string, number>();
  const walk = (n: any) => {
    if (n.children?.length) n.children.forEach(walk);
    else if (n.main_dev) m.set(n.main_dev, (m.get(n.main_dev) ?? 0) + (n.loc ?? 0));
  };
  walk(tree);
  return [...m.entries()].sort((a, b) => b[1] - a[1]).map(([n]) => n);
}
