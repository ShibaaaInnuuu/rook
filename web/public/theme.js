// Runs before first paint so a remembered theme never flashes. A file, not an inline script: the site's CSP allows scripts from 'self' only.
try { var t = localStorage.getItem("etch-theme"); if (t === "light" || t === "dark") document.documentElement.dataset.theme = t; } catch (e) { /* no storage: the system's preference decides */ }
