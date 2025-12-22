<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";

  type DevSettings = {
    enabled: boolean;
    wifiSsid: string;
    wifiPsk: string;
    wifiCountry: string;
    binariesSource: "main" | "custom";
    binariesRepo: string;
    key1Name: string;
    key1User: string;
    key2Name: string;
    key2User: string;
    githubToken: string;
    showDockerHelp: boolean;
  };

  const STORAGE_KEY = "secluso-dev-settings";
  const defaultSettings: DevSettings = {
    enabled: false,
    wifiSsid: "",
    wifiPsk: "",
    wifiCountry: "",
    binariesSource: "main",
    binariesRepo: "",
    key1Name: "",
    key1User: "",
    key2Name: "",
    key2User: "",
    githubToken: "",
    showDockerHelp: false
  };

  let devSettings: DevSettings = { ...defaultSettings };
  let saved = false;

  onMount(() => {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return;
    try {
      const parsed = JSON.parse(raw) as Partial<DevSettings>;
      devSettings = { ...defaultSettings, ...parsed };
    } catch {
      devSettings = { ...defaultSettings };
    }
  });

  function saveSettings() {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(devSettings));
    saved = true;
    setTimeout(() => (saved = false), 1500);
  }
</script>

<main class="wrap">
  <header class="topbar">
    <button class="back" on:click={() => goto("/")}>← Back</button>
    <h1>Settings</h1>
    <div class="spacer"></div>
  </header>

  <section class="card">
    <div class="section">
      <label class="toggle">
        <input type="checkbox" bind:checked={devSettings.enabled} />
        <span>Developer mode</span>
      </label>
      <p class="help">Extra options for testing and staging.</p>
    </div>

    {#if devSettings.enabled}
      <div class="section block">
        <div class="block-title">UI Testing <span class="badge">Optional</span></div>
        <label class="toggle">
          <input type="checkbox" bind:checked={devSettings.showDockerHelp} />
          <span>Force Docker help panel</span>
        </label>
        <p class="help">Shows the Docker install panel even when Docker is installed.</p>
      </div>

      <div class="section block">
        <div class="block-title">Wi-Fi <span class="badge">Optional</span></div>
        <label class="field">
          <span>Wi-Fi SSID</span>
          <input placeholder="dev-wifi" bind:value={devSettings.wifiSsid} />
        </label>
        <p class="help">Used only for the first boot.</p>
        <label class="field">
          <span>Wi-Fi password</span>
          <input type="password" placeholder="password" bind:value={devSettings.wifiPsk} />
        </label>
        <p class="help">Must match the SSID above.</p>
        <label class="field">
          <span>Wi-Fi country</span>
          <input placeholder="US" bind:value={devSettings.wifiCountry} />
        </label>
        <p class="help">Two-letter country code.</p>
      </div>

      <div class="section block">
        <div class="block-title">Binaries <span class="badge">Optional</span></div>
        <label class="radio-row">
          <input type="radio" name="binaries" value="main" bind:group={devSettings.binariesSource} />
          <span>Use main release binaries</span>
        </label>
        <label class="radio-row">
          <input type="radio" name="binaries" value="custom" bind:group={devSettings.binariesSource} />
          <span>Use another repo</span>
        </label>
        {#if devSettings.binariesSource === "custom"}
          <label class="field">
            <span>Repo</span>
            <input placeholder="secluso/secluso or https://github.com/you/repo" bind:value={devSettings.binariesRepo} />
          </label>
          <p class="help">Use owner/repo or a full GitHub URL.</p>
          <label class="field">
            <span>Key 1 name</span>
            <input placeholder="release-key-1" bind:value={devSettings.key1Name} />
          </label>
          <label class="field">
            <span>Key 1 GitHub username</span>
            <input placeholder="username1" bind:value={devSettings.key1User} />
          </label>
          <label class="field">
            <span>Key 2 name</span>
            <input placeholder="release-key-2" bind:value={devSettings.key2Name} />
          </label>
          <label class="field">
            <span>Key 2 GitHub username</span>
            <input placeholder="username2" bind:value={devSettings.key2User} />
          </label>
          <p class="help">Both key names and GitHub users are required for custom repos.</p>
        {/if}
      </div>

      <div class="section block">
        <div class="block-title">GitHub Token <span class="badge">Optional</span></div>
        <label class="field">
          <span>Token</span>
          <input type="password" placeholder="ghp_..." bind:value={devSettings.githubToken} />
        </label>
        <p class="help">Used for GitHub API requests to avoid rate limits.</p>
      </div>
    {/if}
  </section>

  <div class="actions">
    <button class="primary" on:click={saveSettings}>Save</button>
    {#if saved}<span class="saved">Saved</span>{/if}
  </div>
</main>

<style>
  .wrap { max-width: 980px; margin: 0 auto; padding: 20px 20px 60px; }
  .topbar { display: grid; grid-template-columns: 120px 1fr 120px; align-items: center; gap: 12px; margin: 8px 0 18px; }
  .topbar h1 { text-align: center; margin: 0; font-size: 1.6rem; }
  .spacer { width: 100%; }
  .back { justify-self: start; }

  .card { background: #fff; border: 1px solid #e7e7e7; border-radius: 14px; padding: 16px; margin-bottom: 14px; box-shadow: 0 6px 22px rgba(0,0,0,0.06); }
  .section { display: grid; gap: 6px; margin-bottom: 12px; }
  .section.block { padding: 10px; border-radius: 12px; border: 1px solid #e6e6e6; background: #f9f9f9; }
  .block-title { font-weight: 700; margin-bottom: 2px; color: #1f2937; }
  .badge { display: inline-block; margin-left: 6px; padding: 2px 6px; border-radius: 999px; font-size: 0.7rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.02em; background: #eef2ff; color: #334155; border: 1px solid #dbe3ff; }
  .toggle { display: inline-flex; gap: 8px; align-items: center; }
  .radio-row { display: flex; gap: 8px; align-items: center; }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .field span { font-weight: 600; color: #333; }
  .field input { padding: 10px 12px; border-radius: 10px; border: 1px solid #ddd; background: #fff; }
  .help { margin: 0; color: #777; font-size: 0.92rem; }

  .actions { margin-top: 12px; display: flex; gap: 12px; align-items: center; }
  button { border: 1px solid #d7d7d7; background: #fff; color: #111; padding: 10px 14px; border-radius: 10px; cursor: pointer; }
  button.primary { background: #396cd8; color: #fff; border-color: #396cd8; }
  .saved { color: #16a34a; font-weight: 600; }

  @media (prefers-color-scheme: dark) {
    .card { background: #111; border-color: #2a2a2a; box-shadow: 0 2px 10px rgba(0,0,0,.35); }
    .help { color: #c7c7c7; }
    .field input { background: #111; border-color: #2a2a2a; color: #f6f6f6; }
    .section.block { border-color: #2a2a2a; background: #0b0b0b; }
    .block-title { color: #e2e8f0; }
    button { border-color: #2a2a2a; color: #f6f6f6; background: #111; }
    button.primary { background: #396cd8; border-color: #396cd8; color: #fff; }
    .badge { background: #1c2333; color: #cbd5f5; border-color: #2b3550; }
  }
</style>
