const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** 把字节数格式化为 1024 进制的可读大小，例如 1.5 GB。 */
export function formatSize(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  return unit === 0 ? `${value} B` : `${value.toFixed(1)} ${UNITS[unit]}`;
}

/** 百分比，保留一位小数。 */
export function formatPercent(part: number, whole: number): string {
  return whole > 0 ? `${((part / whole) * 100).toFixed(1)}%` : "0.0%";
}
