// Runs before `vite build`. Vite's compiler (Rolldown) ships one native module per OS and CPU,
// installed as an optional package. When the right one is missing, Rolldown's own error blames
// npm and package-lock.json; this explains the real cause and the fix instead.
const where = `${process.platform}-${process.arch}`;
try {
  await import('vite');
} catch (e) {
  const missing = [];
  for (let c = e; c; c = c.cause) {
    const m = /Cannot find module '(@rolldown\/binding-[^']+)'/.exec(c.message);
    if (m) missing.push(m[1]);
  }
  if (!/native binding/i.test(e.message)) throw e;
  console.error(`
csi: the UI build tool's native module for ${where} isn't installed.
${missing.length ? `(looked for ${missing.filter((m) => !m.includes('wasm')).join(', ')})\n` : ''}
This happens when ui/node_modules was installed
  - with optional packages turned off (pnpm config get optional  → should not be false),
  - by a different Node, e.g. an Intel (x64) one vs. an Apple Silicon (arm64) one, or
  - by an install that was interrupted.

Fix: reinstall with the same Node you build with (currently ${process.version}, ${process.arch}):
  rm -rf ui/node_modules
  pnpm -C ui install
  pnpm -C ui build
`);
  process.exit(1);
}
