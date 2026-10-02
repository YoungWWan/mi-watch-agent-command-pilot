(() => {
  if (location.protocol !== "https:") return false;
  const host = location.hostname;
  if (!(host === "xiaomi.com" || host.endsWith(".xiaomi.com") ||
        host === "mi.com" || host.endsWith(".mi.com"))) return false;
  if (location.port && location.port !== "443") return false;
  return (document.body?.innerText || "").trim().toLowerCase() === "ok";
})()
