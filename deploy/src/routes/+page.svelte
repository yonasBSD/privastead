<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  import { onMount } from "svelte";
  import { openExternalUrl } from "$lib/api";

  const STORAGE_KEY = "secluso-dev-settings";
  const FIRST_TIME_KEY = "secluso-first-time";
  let devModeOn = false;
  let firstTimeOn = false;

  onMount(() => {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return;
    try {
      const parsed = JSON.parse(raw) as { enabled?: boolean };
      devModeOn = !!parsed.enabled;
    } catch {
      devModeOn = false;
    }
  });

  onMount(() => {
    const raw = localStorage.getItem(FIRST_TIME_KEY);
    if (raw === null) return;
    firstTimeOn = raw === "true";
  });

  function toggleFirstTime() {
    firstTimeOn = !firstTimeOn;
    localStorage.setItem(FIRST_TIME_KEY, String(firstTimeOn));
  }

  function setHelpRef() {
    try {
      sessionStorage.setItem("secluso-help-ref", window.location.pathname);
    } catch {
      // best effort only
    }
  }

  function isInteractiveTarget(target: EventTarget | null): boolean {
    return target instanceof Element && !!target.closest("a, button, input, label, textarea, select");
  }

  function onToggleCardClick(event: MouseEvent) {
    if (isInteractiveTarget(event.target)) return;
    toggleFirstTime();
  }

  function onToggleKey(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === " " || event.key === "Spacebar") {
      event.preventDefault();
      toggleFirstTime();
    }
  }

  async function openExternal(url: string) {
    try {
      await openExternalUrl(url);
    } catch {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  }

</script>

<main class="container">
  <div class="titlebar">
    <div class="spacer"></div>
    <h1>Secluso Deploy v1.0.0</h1>
    <div class="actions">
      <span class="dev-dot {devModeOn ? 'on' : 'off'}" title={devModeOn ? "Developer mode on" : "Developer mode off"}></span>
      <a class="settings-btn" href="/settings">Settings</a>
    </div>
  </div>
  <p class="subtitle">Get your encrypted camera system online in two easy steps.</p>
  <section class="welcome">
    <div class="welcome-card">
      <div class="welcome-copy">
        <h2>Welcome to Secluso</h2>
        <p class="muted">Achieve true privacy with an easy, non-compromising setup. Follow the steps below to get everything online.</p>
      </div>
      <div class="store-links">
        <a class="store-btn" href="https://apps.apple.com/app/id0000000000" on:click|preventDefault={() => openExternal("https://apps.apple.com/app/id0000000000")}>
          <span class="store-icon">A</span>
          <span>App Store (placeholder)</span>
        </a>
        <a class="store-btn" href="https://play.google.com/store/apps/details?id=com.secluso.mobile" on:click|preventDefault={() => openExternal("https://play.google.com/store/apps/details?id=com.secluso.mobile")}>
          <span class="store-icon">G</span>
          <span>Google Play (placeholder)</span>
        </a>
      </div>
    </div>
  </section>

  {#if firstTimeOn}
    <section class="card toggle-card" role="button" tabindex="0" aria-pressed={firstTimeOn} on:click={onToggleCardClick} on:keydown={onToggleKey}>
      <div class="cardhead">
        <h3>First time?</h3>
        <label class="toggle">
          <input type="checkbox" checked={firstTimeOn} on:change={toggleFirstTime} />
          <span>Show step-by-step guidance</span>
        </label>
      </div>
      <p class="muted">No scripts or command line steps needed.</p>
      <ol class="quick-steps">
        <li>Install the Secluso app from your app store.</li>
        <li>Build the Raspberry Pi image and save the camera QR code.</li>
        <li>Set up your server and save the server QR code.</li>
        <li>Open the app and scan the server QR code, then the camera QR code.</li>
      </ol>
      <p class="muted">Need a server? A low cost option is Ionos VPS for around $2 per month. Just copy the login details from your provider and the app handles the setup. We are not affiliated with Ionos.</p>
      <div class="help-links">
        <a class="help-link" href="/hardware-help" on:click={setHelpRef}>Recommended hardware guide</a>
        <a class="help-link" href="/ionos-help" on:click={setHelpRef}>Ionos VPS setup guide</a>
      </div>
    </section>
  {:else}
    <section class="card toggle-card" role="button" tabindex="0" aria-pressed={firstTimeOn} on:click={onToggleCardClick} on:keydown={onToggleKey}>
      <div class="cardhead">
        <h3>First time?</h3>
        <label class="toggle">
          <input type="checkbox" checked={firstTimeOn} on:change={toggleFirstTime} />
          <span>Show step-by-step guidance</span>
        </label>
      </div>
      <p class="muted">Turn on the toggle to see the step-by-step guide.</p>
    </section>
  {/if}

  <!-- step a -->
  <section class="step">
    <div class="stephead">
      <h2 class="steptitle">
        <span class="stepkicker">Step A</span>
        <span class="stepname">Raspberry Pi</span>
      </h2>
      <p class="muted">Flash a fresh image <em>or</em> to an SD card to prepare your Pi.</p>
    </div>

    <div class="choices">
      <a class="card" href="/image">
        <h3>Build Raspberry&nbsp;Pi Image</h3>
        <p>
          Generate a Pi OS image with the camera secret QR code, packages, and the auto-updater.
          Outputs a flash-ready <code>.img</code> for Raspberry Pi Imager and the camera secret QR code.
        </p>
        <ul class="details">
          <li>No device required during setup</li>
          <li>Deterministic config & pinned packages</li>
          <li>Ideal for new SD cards</li>
        </ul>
        <span class="card-cta">Start Image Builder &rarr;</span>
      </a>
    </div>
  </section>

  <!-- step b -->
  <section class="step">
    <div class="stephead">
      <h2 class="steptitle">
        <span class="stepkicker">Step B</span>
        <span class="stepname">Server</span>
      </h2>
      <p class="muted">The Secluso server is provisioned via SSH. This sets it up fully to work with your Raspberry Pi camera.</p>
    </div>

    <div class="choices single">
      <a class="card" href="/server-ssh">
        <h3>Provision Server (SSH)</h3>
        <p>
          Install the Secluso server as a binary or Docker image, setup services, install necessary packages, enable the auto-updater,
          and optionally harden packages/services.
        </p>
        <ul class="details">
          <li>Linux server with SSH access</li>
          <li>Binary or Docker (your choice)</li>
          <li>Auto-updater & hardening toggles</li>
        </ul>
        <span class="card-cta">Connect to Server &rarr;</span>
      </a>
    </div>
  </section>

  <p class="footnote">
    <strong>Tip:</strong> New device? Start with <em>Build Raspberry Pi Image</em>.
    Already running? Use <em>Provision Existing Pi</em>.
  </p>
</main>

<style>
:root {
  font-family: Inter, Avenir, Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 24px;
  color: #0f0f0f;
  background-color: #f6f6f6;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

.container {
  margin: 0 auto;
  padding: 7vh 24px 10vh;
  max-width: 1040px;
  display: flex;
  flex-direction: column;
  gap: 18px;
}

/* headings */
.titlebar {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: 12px;
}
.titlebar .spacer { width: 100%; }
h1 { text-align: center; margin: 0 0 4px 0; font-size: 2rem; }
.actions { display: flex; justify-content: flex-end; }
.settings-btn {
  border: 1px solid #d8d8d8;
  background: #ffffff;
  color: #111;
  padding: 8px 12px;
  border-radius: 10px;
  cursor: pointer;
  text-decoration: none;
}
.settings-btn:hover { border-color: #c4c4c4; }
.dev-dot {
  width: 10px;
  height: 10px;
  border-radius: 999px;
  border: 1px solid #d1d5db;
  background: #ef4444;
  display: inline-block;
}
.dev-dot.on { background: #22c55e; border-color: #16a34a; }
.dev-dot.off { background: #ef4444; border-color: #dc2626; }

.subtitle {
  text-align: center;
  margin: 0 0 18px 0;
  color: #333;
}

.welcome { margin-bottom: 18px; }
.welcome-card {
  display: flex;
  gap: 18px;
  align-items: center;
  justify-content: space-between;
  border-radius: 16px;
  padding: 18px;
  background: linear-gradient(135deg, #f7f8fb, #ffffff);
  border: 1px solid #e6e6e6;
  box-shadow: 0 2px 10px rgba(0,0,0,0.06);
}
.welcome-copy h2 { margin: 0 0 8px; font-size: 1.25rem; }
.store-links { display: flex; gap: 12px; flex-wrap: wrap; }
.store-btn {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  border-radius: 12px;
  border: 1px solid #dfe3ea;
  background: #fff;
  text-decoration: none;
  color: #1f2937;
  font-weight: 600;
  box-shadow: 0 2px 8px rgba(0,0,0,0.06);
}
.store-btn:hover { border-color: #c9d3ea; }
.store-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 8px;
  background: #396cd8;
  color: #fff;
  font-size: 0.85rem;
}

.step { margin-top: 8px; }

.stephead {
  display: grid;
  grid-template-columns: 1fr;
  gap: 6px;
  margin-bottom: 10px;
}

.steptitle {
  display: flex;
  align-items: baseline;
  gap: 10px;
  margin: 0;
}

.stepkicker {
  font-weight: 700;
  font-size: 0.9rem;
  letter-spacing: .06em;
  text-transform: uppercase;
  color: #4a63b8;
}

.stepname {
  font-size: 1.3rem;
  font-weight: 700;
  color: #0f0f0f;
}

.muted { color: #666; margin: 0; }

/* cards and layout */
.choices {
  display: grid;
  grid-template-columns: repeat( auto-fit, minmax(300px, 1fr) );
  gap: 18px;
}
.choices.single { grid-template-columns: 1fr; }

.card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  border-radius: 14px;
  padding: 18px;
  background: #ffffff;
  color: inherit;
  text-decoration: none;
  border: 1px solid #e6e6e6;
  box-shadow: 0 2px 10px rgba(0,0,0,0.06);
  transition: transform .12s ease, box-shadow .12s ease, border-color .12s ease;
}
.card:hover { transform: translateY(-1px); box-shadow: 0 6px 20px rgba(0,0,0,0.10); border-color: #d8d8d8; }

.card:active { transform: translateY(0); }

.card h3 { margin: 0; font-size: 1.1rem; }
.card p { margin: 0; color: #444; }
.cardhead { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.toggle-card { cursor: pointer; }

.quick-steps {
  margin: 6px 0 0;
  padding-left: 20px;
  color: #555;
}
.quick-steps li { margin: 4px 0; }

.help-links { display: flex; flex-wrap: wrap; gap: 12px; margin-top: 8px; }
.help-link { font-size: 0.95rem; color: #396cd8; text-decoration: none; }
.help-link:hover { text-decoration: underline; }

.details { margin: 4px 0 0; padding-left: 18px; color: #555; }
.details li { margin: 2px 0; }

.card-cta { margin-top: auto; font-weight: 600; color: #396cd8; }

/* footer */
.footnote {
  margin: 12px auto 0;
  text-align: center;
  color: #666;
  max-width: 860px;
  font-size: 0.98rem;
}


/* dark mode */
@media (prefers-color-scheme: dark) {
  :root { color: #f6f6f6; background-color: #2f2f2f; }
  .stepkicker { color: #7aa7ff; }
  .stepname { color: #f6f6f6; }
  .card { background: #111; border-color: #2a2a2a; box-shadow: 0 2px 10px rgba(0,0,0,.35); }
  .card p, .details, .subtitle, .muted, .footnote { color: #d3d3d3; }
  a:hover { color: #24c8db; }
  .settings-btn { background: #111; color: #f6f6f6; border-color: #2a2a2a; }
  .dev-dot { border-color: #1f2937; }
  .dev-dot.on { background: #22c55e; border-color: #16a34a; }
  .dev-dot.off { background: #ef4444; border-color: #dc2626; }
  .welcome-card { background: linear-gradient(135deg, #10131a, #14171f); border-color: #2a2a2a; }
  .store-btn { background: #111; border-color: #2a2a2a; color: #f6f6f6; }
  .store-icon { background: #7aa7ff; color: #0b1020; }
}

.toggle {
  display: inline-flex;
  gap: 8px;
  align-items: center;
  padding: 8px 10px;
  border: 1px solid #e6e6e6;
  border-radius: 10px;
  background: #fff;
  font-size: 0.9rem;
  color: #111;
}
.toggle input { transform: translateY(1px); }

@media (prefers-color-scheme: dark) {
  .toggle { background: #111; border-color: #2a2a2a; color: #f6f6f6; }
}

@media (max-width: 720px) {
  .welcome-card { flex-direction: column; align-items: flex-start; }
}
</style>
