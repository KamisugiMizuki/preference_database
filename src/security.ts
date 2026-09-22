// 只允许显式的绝对 HTTP(S) URL；保留原文，避免改写查询参数或签名。
export function httpUrl(value: string): string | null {
  if (!/^https?:\/\/[^/\\]/i.test(value) || /[\u0000-\u0020\u007f\\]/.test(value)) {
    return null;
  }
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") && url.hostname
      ? value
      : null;
  } catch {
    return null;
  }
}
