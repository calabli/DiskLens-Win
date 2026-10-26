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

/** 距今多久，例如“刚刚”“5 分钟前”“昨天 14:05”“10月3日 09:30”。 */
export function formatAgo(ms: number, now = Date.now()): string {
  const minutes = Math.floor((now - ms) / 60_000);
  if (minutes < 1) return "刚刚";
  if (minutes < 60) return `${minutes} 分钟前`;
  if (minutes < 6 * 60) return `${Math.floor(minutes / 60)} 小时前`;
  const date = new Date(ms);
  const time = date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const days = Math.ceil((today.getTime() - ms) / 86_400_000);
  if (days <= 0) return `今天 ${time}`;
  if (days === 1) return `昨天 ${time}`;
  return `${date.getMonth() + 1}月${date.getDate()}日 ${time}`;
}

/** 百分比，保留一位小数。 */
export function formatPercent(part: number, whole: number): string {
  return whole > 0 ? `${((part / whole) * 100).toFixed(1)}%` : "0.0%";
}
