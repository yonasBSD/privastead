//! SPDX-License-Identifier: GPL-3.0-or-later

use crate::frame::SAVE_IMAGES;
use crate::logic::intent::Intent;
use crate::logic::pipeline::RunId;
use crossbeam_channel::{Sender, TrySendError, bounded, select, tick};
use serde::Serialize;
use std::io::BufWriter;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    thread,
};

/// Represents a structured telemetry message logged during pipeline operation.
/// Encodes metadata about performance, state transitions, detection outcomes, and events.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TelemetryPacket<'a> {
    // Logs stats for a specific pipeline stage execution
    Stage {
        //implemented
        run_id: RunId,
        stage: &'a str,
        calls: u64,
        last_latency_ms: u32,
        faults: u64,
        dropped_frames: u64,
        ts: u128,
    },
    // Snapshot of system state at each pipeline tick
    TickStats {
        ts: u128,
        run_id: RunId,
        activity: &'static str,
        health: &'static str,
        event_queue_len: usize,
        standby_has_frame: bool,
        active_has_frame: bool,
        max_event_queue_len: usize,
    },
    // Details of motion detection points/clusters
    MotionMetrics {
        run_id: RunId,
        w_b: f32,
        total_points: u32,
        clustered_points: u32,
        threshold: u32,
        ts: u128,
    },
    // Frame dropped due to failure or resource constraints
    DroppedFrame {
        run_id: RunId,
        ts: u128,
        reason: &'a str, // heat / load / critical / etc
    },
    // Inference bypassed due to system load or config
    InferenceSkipped {
        run_id: RunId,
        ts: u128,
        reason: &'a str,
    },
    // CPU/RAM/temp snapshot
    Health {
        run_id: RunId,
        cpu_pct: f32,
        ram_pct: f32,
        temp_c: f32,
        ts: u128,
    },
    // ML model change triggered by health events
    ModelSwitch {
        ts: u128,
        run_id: RunId,
        from: &'a str,
        to: &'a str,
        reason: &'a str,
        health: &'a str,
    },
    // Duration spent in a specific FSM state
    StateDuration {
        run_id: RunId,
        ts: u128,
        state: &'a str,
        fsm: &'a str,
        duration_ms: u128,
    },
    DetectionsSummary {
        ts: u128,
        run_id: &'a str,
        label_stats: &'a [(
            /*label:*/ i32,
            /*count:*/ usize,
            /*avg:*/ f32,
            /*max:*/ f32,
        )],
    },
    // Activity or health FSM state change
    FSMTransition {
        run_id: RunId,
        from: &'a str,
        to: &'a str,
        reason: &'a str,
    },
    // Detection result for a single frame
    Detection {
        run_id: RunId,
        frame_rel: &'a str,
        detections: usize,
        latency_ms: u32,
        ts: u128,
    },
    // Total duration for stage (optional additional stat)
    StageDuration {
        ts: u128,
        run_id: RunId,
        stage_name: &'a str,
        stage_kind: &'a str,
        duration_ms: u128,
    },
    // Captures each dispatched intent during pipeline flow
    IntentTriggered {
        run_id: RunId,
        intent: Intent,
        ts: u128,
    },
}

impl TelemetryPacket<'_> {
    fn run_key(&self) -> Option<&str> {
        match self {
            TelemetryPacket::Stage { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::TickStats { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::MotionMetrics { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::DroppedFrame { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::InferenceSkipped { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::Health { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::ModelSwitch { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::StateDuration { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::DetectionsSummary { run_id, .. } => Some(*run_id),
            TelemetryPacket::FSMTransition { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::Detection { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::StageDuration { run_id, .. } => Some(run_id.0.as_str()),
            TelemetryPacket::IntentTriggered { run_id, .. } => Some(run_id.0.as_str()),
        }
    }
}

/*
pub enum FrameExport {
    InitialFrame,
    AdaptiveGrayscale,
    CLAHE,
    Blur,
    BgSubtractNoised,
    BgSubtractDenoised,
    BgSubtractBackend,
    MLResults,
}*/

/// Represents a telemetry logging session for a single pipeline run.
/// Manages writing structured logs to a file in the run directory.
pub struct TelemetryRun {
    pub run_id: String,
    activated: bool,
    save_all: bool,
    tx: Option<Sender<TelemetryMsg>>,
    handle: Option<JoinHandle<()>>,
    dropped: Arc<AtomicU64>,
    gate: Mutex<RunGate>,
}

enum TelemetryMsg {
    Line(String),
    Flush,
    Shutdown,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum GateState {
    Pending,
    Approved,
    Rejected,
}

struct RunGate {
    run_key: Option<String>,
    state: GateState,
    pending: Vec<String>,
}

impl Default for RunGate {
    fn default() -> Self {
        Self {
            run_key: None,
            state: GateState::Pending,
            pending: Vec::new(),
        }
    }
}

impl TelemetryRun {
    /// Create a run directory, spawn writer thread if activated.
    /// Batches up to BATCH_MAX lines or FLUSH_EVERY duration, whichever comes first.
    pub fn new(activated: bool, save_all: bool) -> Result<Self, anyhow::Error> {
        let run_id = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();
        SAVE_IMAGES.store(activated, Ordering::Relaxed);

        if !activated {
            return Ok(Self {
                run_id,
                activated: false,
                save_all,
                tx: None,
                handle: None,
                dropped: Arc::new(AtomicU64::new(0)),
                gate: Mutex::new(RunGate::default()),
            });
        }

        let base = Path::new("output").join("runs").join(&run_id);
        std::fs::create_dir_all(base.join("frames"))?;

        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(base.join("telemetry.log"))?;

        let (tx, rx) = bounded::<TelemetryMsg>(8192); // bounded -> backpressure instead of unbounded RAM
        let dropped = Arc::new(AtomicU64::new(0));

        // Writer worker
        let handle = thread::Builder::new()
            .name("telemetry-writer".to_string())
            .spawn(move || {
                const BATCH_MAX: usize = 512; // max lines per write burst
                const FLUSH_EVERY: Duration = Duration::from_millis(500); // periodic flush
                const IDLE_EXIT: Option<Duration> = None; // keep thread alive entire run

                let mut writer = BufWriter::new(file);
                let mut buf: Vec<String> = Vec::with_capacity(BATCH_MAX);
                let ticker = tick(FLUSH_EVERY);

                // Helper to write & flush current buffer
                let flush_buf =
                    |writer: &mut BufWriter<File>, buf: &mut Vec<String>, force: bool| {
                        if buf.is_empty() && !force {
                            return;
                        }
                        for line in buf.drain(..) {
                            // Each line already JSON; add newline and write
                            let _ = writer.write_all(line.as_bytes());
                            let _ = writer.write_all(b"\n");
                        }
                        let _ = writer.flush();
                    };

                // Main loop
                loop {
                    select! {
                        recv(rx) -> msg => {
                            match msg {
                                Ok(TelemetryMsg::Line(line)) => {
                                    buf.push(line);
                                    if buf.len() >= BATCH_MAX {
                                        flush_buf(&mut writer, &mut buf, false);
                                    }
                                }
                                Ok(TelemetryMsg::Flush) => {
                                    flush_buf(&mut writer, &mut buf, true);
                                }
                                Ok(TelemetryMsg::Shutdown) | Err(_) => {
                                    // Drain any remaining and exit
                                    flush_buf(&mut writer, &mut buf, true);
                                    break;
                                }
                            }
                        }
                        recv(ticker) -> _ => {
                            // Periodic flush to ensure nothing lingers if no more writes arrive
                            flush_buf(&mut writer, &mut buf, false);
                        }
                        default(IDLE_EXIT.unwrap_or(Duration::from_millis(0))) => {
                            // Not used: we keep thread until Shutdown.
                        }
                    }
                }
            })?;

        Ok(Self {
            run_id,
            activated: true,
            save_all,
            tx: Some(tx),
            handle: Some(handle),
            dropped,
            gate: Mutex::new(RunGate::default()),
        })
    }

    /// Enqueue a packet; returns quickly. If channel is full, it will try briefly then drop.
    pub fn write(&self, pkt: &TelemetryPacket) -> Result<(), anyhow::Error> {
        if !self.activated {
            return Ok(());
        }

        // Serialize outside worker to minimize critical section in writer
        let line = serde_json::to_string(pkt)?;

        if self.save_all {
            return self.try_send_line(line);
        }

        if let Some(run_key) = pkt.run_key() {
            let mut gate = self.gate.lock().unwrap();
            if gate.run_key.as_deref() != Some(run_key) {
                gate.run_key = Some(run_key.to_string());
                gate.state = GateState::Pending;
                gate.pending.clear();
            }

            match gate.state {
                GateState::Approved => {
                    drop(gate);
                    return self.try_send_line(line);
                }
                GateState::Rejected => return Ok(()),
                GateState::Pending => {
                    gate.pending.push(line);
                    return Ok(());
                }
            }
        }

        self.try_send_line(line)
    }

    /// Allow telemetry for a run and flush any buffered packets.
    pub fn approve_run(&self, run_id: &RunId) {
        if !self.activated || self.save_all {
            return;
        }

        let run_key = run_id.0.as_str();
        let pending = {
            let mut gate = self.gate.lock().unwrap();
            if gate.run_key.as_deref() != Some(run_key) {
                gate.run_key = Some(run_key.to_string());
                gate.pending.clear();
            }
            gate.state = GateState::Approved;
            std::mem::take(&mut gate.pending)
        };

        for line in pending {
            let _ = self.try_send_line(line);
        }
    }

    /// Reject telemetry for a run and discard any buffered packets.
    pub fn reject_run(&self, run_id: &RunId) {
        if !self.activated || self.save_all {
            return;
        }

        let run_key = run_id.0.as_str();
        let mut gate = self.gate.lock().unwrap();
        if gate.run_key.as_deref() != Some(run_key) {
            gate.run_key = Some(run_key.to_string());
        }
        gate.state = GateState::Rejected;
        gate.pending.clear();
    }

    fn try_send_line(&self, line: String) -> Result<(), anyhow::Error> {
        let Some(tx) = &self.tx else {
            return Ok(());
        };

        // Non-blocking fast path
        match tx.try_send(TelemetryMsg::Line(line)) {
            Ok(_) => Ok(()),
            Err(TrySendError::Full(line)) => {
                // Backoff briefly to avoid sustained stalls
                let (_tmp_tx, tmp_rx) = bounded::<()>(0);
                // Wait up to ~50ms for capacity
                let waited = select! {
                    recv(tmp_rx) -> _ => false, // never happens
                    default(Duration::from_millis(50)) => true
                };
                if waited {
                    // Try once more
                    if tx
                        .try_send(TelemetryMsg::Line(match line {
                            TelemetryMsg::Line(s) => s,
                            _ => unreachable!(),
                        }))
                        .is_ok()
                    {
                        return Ok(());
                    }
                }
                // Drop if still full; count it
                self.dropped.fetch_add(1, Ordering::Relaxed);
                // Optional: eprintln!("telemetry drop (buffer full)");
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => Ok(()), // shutting down
        }
    }

    /// Ask the worker to flush soon (non-blocking).
    pub fn request_flush(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(TelemetryMsg::Flush);
        }
    }

    /// Number of dropped packets due to full buffer (for diagnostics).
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for TelemetryRun {
    fn drop(&mut self) {
        if !self.activated {
            return;
        }
        if let Some(tx) = self.tx.take() {
            // Best-effort shutdown signal
            let _ = tx.send(TelemetryMsg::Shutdown);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join(); // ensure final flush completed
        }
    }
}
