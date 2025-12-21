<script lang="ts">
  import { onDestroy, tick } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { listenProvisionEvents, type ProvisionEvent } from "$lib/api";

  type StepState = "pending" | "running" | "ok" | "error";

  const stepMap: Record<string, { key: string; title: string }[]> = {
    server: [
      { key: "ssh_connect", title: "Connect via SSH" },
      { key: "detect", title: "Detect install state" },
      { key: "secrets", title: "Upload secrets" },
      { key: "remote", title: "Run remote installer" }
    ],
    image: [
      { key: "validate", title: "Validate inputs" },
      { key: "docker_check", title: "Check Docker" },
      { key: "docker_build", title: "Build image builder" },
      { key: "credentials", title: "Generate pairing credentials" },
      { key: "config", title: "Prepare build config" },
      { key: "docker_run", title: "Build image" },
      { key: "verify", title: "Verify outputs" }
    ]
  };

  let runId = "";
  let mode = "server";
  let steps: { key: string; title: string }[] = [];
  let stepStatus: Record<string, StepState> = {};
  let logs: { level: string; step?: string; line: string; time: string }[] = [];
  let doneOk: boolean | null = null;
  let currentTitle = "";
  let unlisten: (() => void) | null = null;
  let lastRunId = "";
  let logsEl: HTMLDivElement | null = null;
  let stickToBottom = true;
  let pendingScroll = false;
  let updaterWarning: string | null = null;

  $: {
    runId = $page.url.searchParams.get("runId") ?? "";
    mode = $page.url.searchParams.get("mode") ?? "server";
  }
  $: steps = stepMap[mode] ?? stepMap.server;

  $: if (runId && runId !== lastRunId) {
    lastRunId = runId;
    resetState();
    startListening();
  }

  function resetState() {
    stepStatus = {};
    for (const s of steps) stepStatus[s.key] = "pending";
    if (steps.some((s) => s.key === "validate")) {
      stepStatus = { ...stepStatus, validate: "ok" };
    }
    logs = [];
    doneOk = null;
    currentTitle = "";
    stickToBottom = true;
    updaterWarning = null;
  }

  async function startListening() {
    if (unlisten) unlisten();
    unlisten = await listenProvisionEvents((evt) => handleEvent(evt));
  }

  function handleEvent(evt: ProvisionEvent) {
    if (evt.run_id !== runId) return;

    if (evt.type === "step_start") {
      stepStatus = { ...stepStatus, [evt.step]: "running" };
      currentTitle = evt.title;
      return;
    }

    if (evt.type === "step_ok") {
      stepStatus = { ...stepStatus, [evt.step]: "ok" };
      return;
    }

    if (evt.type === "step_error") {
      stepStatus = { ...stepStatus, [evt.step]: "error" };
      logs = [
        ...logs,
        { level: "error", step: evt.step, line: evt.message, time: new Date().toLocaleTimeString() }
      ];
      queueScrollToBottom();
      return;
    }

    if (evt.type === "log") {
      logs = [
        ...logs,
        { level: evt.level, step: evt.step, line: evt.line, time: new Date().toLocaleTimeString() }
      ];
      if (
        evt.level === "warn" &&
        evt.step === "updater" &&
        !updaterWarning &&
        evt.line.includes("secluso-updater not found")
      ) {
        updaterWarning = "secluso-updater was not found, so auto updates were not set up.";
      }
      queueScrollToBottom();
      return;
    }

    if (evt.type === "done") {
      doneOk = evt.ok;
    }
  }

  onDestroy(() => {
    if (unlisten) unlisten();
  });

  function handleLogScroll() {
    if (!logsEl) return;
    const threshold = 24;
    const distance = logsEl.scrollHeight - logsEl.scrollTop - logsEl.clientHeight;
    stickToBottom = distance <= threshold;
  }

  function queueScrollToBottom() {
    if (!logsEl || !stickToBottom || pendingScroll) return;
    pendingScroll = true;
    tick().then(() => {
      pendingScroll = false;
      if (!logsEl || !stickToBottom) return;
      logsEl.scrollTop = logsEl.scrollHeight;
    });
  }

  $: completedSteps = steps.filter((s) => stepStatus[s.key] === "ok").length;
  $: totalSteps = steps.length;
  $: progress = totalSteps ? Math.round((completedSteps / totalSteps) * 100) : 0;
</script>

<main class="wrap">
  {#if doneOk !== null}
    <div class="overlay" role="status" aria-live="polite">
      <div class="modal {doneOk ? 'ok' : 'error'}">
        <div class="modal-title">{doneOk ? "All done" : "Run failed"}</div>
        <div class="modal-body">
          {doneOk ? "Everything finished successfully." : "Something went wrong. Check the logs for details."}
        </div>
        <button class="modal-btn" on:click={() => (doneOk = null)}>Dismiss</button>
      </div>
    </div>
  {/if}
  {#if updaterWarning}
    <div class="overlay" role="status" aria-live="polite">
      <div class="modal warn">
        <div class="modal-title">Updater not set up</div>
        <div class="modal-body">{updaterWarning}</div>
        <button class="modal-btn" on:click={() => (updaterWarning = null)}>Dismiss</button>
      </div>
    </div>
  {/if}
  <header class="topbar">
    <button class="back" on:click={() => goto("/")}>← Back</button>
    <h1>{mode === "image" ? "Image Build Status" : "Server Provision Status"}</h1>
    <div class="spacer"></div>
  </header>

  {#if !runId}
    <section class="card empty">
      <div class="empty-icon">⏳</div>
      <h2>Missing run ID</h2>
      <p class="muted">Start a provisioning or build task first, then return here with a run ID.</p>
    </section>
  {:else}
    <section class="card hero">
      <div class="hero-inner">
        <div>
          <div class="label">Current step</div>
          <div class="value">{currentTitle || "Waiting for events…"}</div>
          <div class="meta">
            <span>{completedSteps}/{totalSteps} steps complete</span>
            <span>Run ID: {runId}</span>
          </div>
        </div>
        <div class="status">
          <div class="pill {doneOk === true ? 'ok' : doneOk === false ? 'error' : 'running'}">
            {doneOk === true ? "Completed" : doneOk === false ? "Failed" : "Running"}
          </div>
          <div class="progress">
            <div class="bar" style={`width:${doneOk ? 100 : progress}%;`}></div>
          </div>
        </div>
      </div>
    </section>

    <div class="grid">
      <section class="card">
        <div class="cardhead">
          <h2>Steps</h2>
          <span class="count">{completedSteps}/{totalSteps}</span>
        </div>
        <div class="steps">
          {#each steps as s}
            <div class="step {stepStatus[s.key]}">
              <span class="dot"></span>
              <span class="title">{s.title}</span>
              <span class="state">{stepStatus[s.key]}</span>
            </div>
          {/each}
        </div>
      </section>

      <section class="card logs-card">
        <div class="cardhead">
          <h2>Logs</h2>
          <span class="count">{logs.length}</span>
        </div>
        {#if logs.length === 0}
          <p class="muted">No log output yet.</p>
        {:else}
          <div class="logs" bind:this={logsEl} on:scroll={handleLogScroll}>
            {#each logs as log}
              <div class="line {log.level}">
                <span class="time">{log.time}</span>
                <span class="tag">{log.step ?? "general"}</span>
                <span class="msg">{log.line}</span>
              </div>
            {/each}
          </div>
        {/if}
      </section>
    </div>
  {/if}
</main>

<style>
  .wrap {
    max-width: 980px;
    margin: 0 auto;
    padding: 20px 20px 60px;
    min-height: 100vh;
    background: radial-gradient(1200px 600px at 10% -10%, #1a2440 0%, #0b1020 55%, #080b14 100%);
    color: #e5e7eb;
  }
  .topbar { display: grid; grid-template-columns: 120px 1fr 120px; align-items: center; gap: 12px; margin: 8px 0 18px; }
  .topbar h1 { text-align: center; margin: 0; font-size: 1.6rem; color: #f8fafc; }
  .spacer { width: 100%; }

  .back {
    appearance: none;
    border: 1px solid #1f2937;
    background: #111827;
    color: #e5e7eb;
    border-radius: 999px;
    padding: 8px 14px;
    font-weight: 600;
    cursor: pointer;
    box-shadow: 0 8px 20px rgba(0, 0, 0, 0.4);
    transition: transform 0.12s ease, box-shadow 0.12s ease, border-color 0.12s ease;
  }
  .back:hover { border-color: #334155; box-shadow: 0 10px 22px rgba(0, 0, 0, 0.5); transform: translateY(-1px); }
  .back:active { transform: translateY(0); box-shadow: 0 8px 20px rgba(0, 0, 0, 0.4); }
  .back:focus-visible { outline: 2px solid #93c5fd; outline-offset: 2px; }

  .card { background: #0f172a; border: 1px solid #1f2937; border-radius: 16px; padding: 16px; margin-bottom: 14px; box-shadow: 0 10px 30px rgba(0, 0, 0, 0.35); }
  .card h2 { margin: 0 0 10px 0; font-size: 1.15rem; color: #f8fafc; }
  .muted { color: #94a3b8; margin: 0; }

  .hero { border: none; padding: 18px; background: linear-gradient(135deg, #0f172a, #111827); }
  .hero-inner { display: grid; grid-template-columns: 1.2fr 0.8fr; gap: 18px; align-items: center; }
  .label { font-size: 0.85rem; color: #94a3b8; }
  .value { font-size: 1.15rem; font-weight: 700; color: #f8fafc; }
  .status { display: grid; gap: 10px; }
  .pill { padding: 8px 14px; border-radius: 999px; font-size: 0.9rem; font-weight: 600; background: #1f2937; color: #e5e7eb; justify-self: end; }
  .pill.running { background: #1e293b; color: #93c5fd; }
  .pill.ok { background: #0f2e25; color: #34d399; }
  .pill.error { background: #3b1b1b; color: #fca5a5; }

  .progress { height: 9px; border-radius: 999px; background: #1f2937; overflow: hidden; }
  .progress .bar { height: 100%; background: linear-gradient(90deg, #3b82f6, #22c55e); transition: width 0.2s ease; }
  .meta { display: flex; justify-content: space-between; color: #94a3b8; font-size: 0.85rem; margin-top: 6px; }

  .grid { display: grid; grid-template-columns: 1fr 1.2fr; gap: 14px; }
  .cardhead { display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px; }
  .count { font-size: 0.85rem; color: #94a3b8; }

  .steps { display: grid; gap: 8px; }
  .step { display: grid; grid-template-columns: 14px 1fr auto; gap: 10px; align-items: center; padding: 10px 12px; border-radius: 12px; border: 1px solid #1f2937; background: #0b1224; }
  .step .dot { width: 10px; height: 10px; border-radius: 50%; background: #d1d5db; }
  .step.pending .dot { background: #d1d5db; }
  .step.running .dot { background: #3b82f6; }
  .step.ok .dot { background: #22c55e; }
  .step.error .dot { background: #ef4444; }
  .step .title { font-weight: 500; color: #e2e8f0; }
  .step .state { font-size: 0.8rem; text-transform: uppercase; color: #94a3b8; }

  .logs-card { display: grid; grid-template-rows: auto 1fr; }
  .logs { display: grid; gap: 6px; max-height: 360px; overflow: auto; border: 1px solid #1f2937; border-radius: 12px; padding: 12px; background: #070b16; color: #e5e7eb; }
  .line { display: grid; grid-template-columns: 70px 90px 1fr; gap: 8px; font-size: 0.88rem; }
  .line.info .msg { color: #e5e7eb; }
  .line.warn .msg { color: #fbbf24; }
  .line.error .msg { color: #fca5a5; }
  .time { color: #94a3b8; font-variant-numeric: tabular-nums; }
  .tag { color: #a5b4fc; }
  .msg { white-space: pre-wrap; }

  .empty { text-align: center; padding: 32px 20px; }
  .empty-icon { font-size: 2rem; margin-bottom: 10px; }

  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(2, 6, 23, 0.55);
    display: grid;
    place-items: center;
    z-index: 30;
  }
  .modal {
    width: min(420px, 90vw);
    background: #0f172a;
    border: 1px solid #1f2937;
    border-radius: 16px;
    padding: 18px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
  }
  .modal.ok { border-color: #0f2e25; }
  .modal.error { border-color: #3b1b1b; }
  .modal.warn { border-color: #3a2f00; }
  .modal-title { font-size: 1.1rem; font-weight: 700; margin-bottom: 6px; color: #f8fafc; }
  .modal-body { color: #cbd5f5; margin-bottom: 14px; }
  .modal-btn {
    appearance: none;
    border: 1px solid #334155;
    background: #111827;
    color: #e5e7eb;
    border-radius: 999px;
    padding: 8px 14px;
    font-weight: 600;
    cursor: pointer;
  }
  .modal-btn:hover { border-color: #475569; }

  @media (max-width: 820px) {
    .hero-inner { grid-template-columns: 1fr; }
    .grid { grid-template-columns: 1fr; }
    .pill { justify-self: start; }
    .meta { flex-direction: column; gap: 4px; }
    .line { grid-template-columns: 60px 70px 1fr; }
  }
</style>
