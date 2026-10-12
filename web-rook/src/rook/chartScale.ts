import type { AutoscaleInfo } from "lightweight-charts";

/** The autoscale range with room below the lowest value for the "launched" pin, never below zero. The markers' own autoscale is off
 *  (it adds its margin below the range, which is how the axis once read −5.00 ETH), so the room is made here. */
export function floorAtZero(original: () => AutoscaleInfo | null): AutoscaleInfo | null {
  const r = original();
  if (!r?.priceRange) return r;
  const { minValue, maxValue } = r.priceRange, room = (maxValue - minValue || Math.abs(maxValue) || 1) * 0.15;
  return { priceRange: { minValue: Math.max(0, minValue - room), maxValue }, margins: r.margins };
}
