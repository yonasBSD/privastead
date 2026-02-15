// SPDX-License-Identifier: GPL-3.0-or-later
// Tauri doesn't have a Node.js server to do proper SSR
// so we will use adapter-static to prerender the app (SSG)
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

// SvelteKit's default version hash is build-time dependent. Pin it so static
// asset names are reproducible across identical source/toolchain builds.
const deterministicVersion =
  process.env.SOURCE_DATE_EPOCH ?? process.env.npm_package_version ?? "0";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  kit: {
    adapter: adapter(),
    version: {
      name: deterministicVersion,
    },
  },
};

export default config;
