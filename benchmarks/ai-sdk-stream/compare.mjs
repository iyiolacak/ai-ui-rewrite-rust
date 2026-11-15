#!/usr/bin/env node
/** Native CPU comparison only. No React, Slint, network, model or UI is measured. */
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { availableParallelism, cpus, loadavg, platform, arch } from 'node:os';
import { basename, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { isDeepStrictEqual } from 'node:util';
import { pathToFileURL } from 'node:url';
import { replay as upstream } from './ai-sdk-oracle.mjs';

function argumentsOf(args) {
  const result = {};
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    if (!['--binary', '--out'].includes(key) || !args[index + 1] || result[key]) {
      throw new Error('Usage: node compare.mjs --binary <release replay executable> [--out <report.json>]');
    }
    result[key] = args[index + 1];
  }
  if (!result['--binary']) throw new Error('--binary is required');
  return result;
}

const sse = chunk => `data: ${JSON.stringify(chunk)}\n\n`;
function textWire(count) {
  return sse({ type: 'text-start', id: 't' }) +
    sse({ type: 'text-delta', id: 't', delta: 'abcdefghijklmnopqrstuvwx' }).repeat(count) +
    sse({ type: 'text-end', id: 't' }) + 'data: [DONE]\n\n';
}

function workloads() {
  const toolInput = { items: Array.from({ length: 1024 }, (_, index) => ({ index, label: `item-${index}` })) };
  const raw = JSON.stringify(toolInput);
  let tool = sse({ type: 'tool-input-start', toolCallId: 'c', toolName: 'search' });
  for (let offset = 0; offset < raw.length; offset += 64) {
    tool += sse({ type: 'tool-input-delta', toolCallId: 'c', inputTextDelta: raw.slice(offset, offset + 64) });
  }
  tool += sse({ type: 'tool-input-available', toolCallId: 'c', toolName: 'search', input: toolInput });
  tool += sse({ type: 'tool-output-available', toolCallId: 'c', output: { found: 1024 } });
  tool += 'data: [DONE]\n\n';
  return [
    { name: 'text-10000x24', description: 'One message, 10,000 text deltas of 24 bytes.', deltas: 10_000,
      request: { raw_utf8: textWire(10_000), chunk_bytes: 4096 } },
    { name: 'tool-json-64-byte-fragments', description: 'A 1,024-object tool input, 64-character argument fragments, then complete input and output.',
      deltas: Math.ceil(raw.length / 64), request: { raw_utf8: tool, chunk_bytes: 4096 } },
    { name: 'message-with-1000-existing-parts', description: '1,000 existing text parts in ONE message, then 1,000 deltas; not conversation-history rendering.',
      deltas: 1000, request: { raw_utf8: textWire(1000), chunk_bytes: 4096,
        initialMessage: { id: 'm', role: 'assistant', parts: Array.from({ length: 1000 }, (_, index) => ({
          type: 'text', state: 'done', text: `retained-${index}-` + 'x'.repeat(48),
        })) } } },
  ];
}

export function rustClient(binary, options = {}) {
  const child = spawn(resolve(binary), options.args ?? [], { stdio: ['pipe', 'pipe', 'ignore'] });
  const lines = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  let failure;
  // Resolve, never reject, so a spawn failure cannot become an unhandled promise
  // while another runtime's warmup is executing.
  const exited = new Promise(done => {
    child.once('error', error => { failure = error; done({ code: null }); });
    child.once('exit', code => done({ code }));
  });
  child.stdin.on('error', error => { failure = error; });
  async function bounded(operation, label) {
    let timer;
    try {
      return await Promise.race([operation, new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`Timed out waiting for Rust ${label}`)), options.timeoutMs ?? 30_000);
      })]);
    } finally {
      clearTimeout(timer);
    }
  }
  return {
    async replay(request) {
      if (failure) throw new Error('Rust replay process is unavailable');
      await bounded(new Promise((done, reject) => {
        child.stdin.write(JSON.stringify(request) + '\n', error => {
          if (error) reject(new Error('Rust replay write failed'));
          else done();
        });
      }), 'request write');
      const line = await bounded(lines.next(), 'response');
      if (line.done || failure) throw new Error('Rust replay ended before returning a result');
      return JSON.parse(line.value);
    },
    async close() {
      child.stdin.end();
      const { code } = await bounded(exited, 'shutdown');
      if (code !== 0) throw new Error('Rust replay exited unsuccessfully');
    },
    stop() { child.kill(); },
  };
}

export function assertEquivalent(reference, result, workload, mode) {
  for (const key of ['foldedMessage', 'sidecars', 'emissions']) {
    if (!isDeepStrictEqual(reference[key], result[key])) {
      throw new Error(`${workload}/${mode}: ${key} mismatch; timings rejected`);
    }
  }
  if (!Number.isFinite(result.elapsed_ms) || result.elapsed_ms <= 0) throw new Error('Invalid timing');
}

function distribution(values) {
  const sorted = [...values].sort((a, b) => a - b);
  return { samples_ms: values, min_ms: sorted[0], median_ms: sorted[Math.floor(sorted.length / 2)], max_ms: sorted.at(-1) };
}

export function trialOrder(trial, modes) {
  return modes.map((_, index) => modes[(index + trial) % modes.length]);
}

async function main(argv) {
  const args = argumentsOf(argv);
  const rust = rustClient(args['--binary']);
  try {
    const rustVersion = spawnSync('rustc', ['--version'], { encoding: 'utf8' });
    if (rustVersion.status !== 0) throw new Error('rustc must be on PATH for toolchain recording');
    const cpuQuota = await readFile('/sys/fs/cgroup/cpu.max', 'utf8').then(value => value.trim()).catch(() => null);
    const report = {
      scope: 'Native in-memory SSE + JSON/schema + fold + sidecars; no React, Slint, UI, networking or model generation.',
      versions: { ai: '7.0.166', provider_utils: '5.0.76', node: process.version, rust: rustVersion.stdout.trim() },
      required_build: 'cargo build -p ai-ui-sdk-stream --example replay --release --locked; opt-level=3, lto=true, codegen-units=1, panic=abort',
      binary: { name: basename(args['--binary']), sha256: createHash('sha256').update(await readFile(args['--binary'])).digest('hex'),
        provenance: 'Caller-supplied executable; build flags are required procedure, not inferred from the binary. rust version is the compiler on PATH.' },
      environment: { platform: platform(), arch: arch(), cpu_model: cpus()[0]?.model ?? 'unknown',
        logical_cpus: cpus().length, available_parallelism: availableParallelism(), cgroup_cpu_max: cpuQuota,
        load_1m_start: loadavg()[0] },
      trials_per_mode: 9, warmups_per_mode: 2,
      timing_boundary: 'Internal replay clocks exclude process startup, JSONL request parsing, prepared network chunks, initial copy and output serialization. Rust includes initial state validation and indexing; Node includes upstream snapshot production.',
      comparison: 'Rust snapshot materializes each publication. Rust patch emits ordered borrowed updates and materializes once at EOF; its speedup includes changed consumer demand. Every measured final fold, sidecar and publication count is compared with upstream.',
      warning: 'Synthetic microbenchmarks on a shared host. No general 100x application claim; no allocation/RSS, UI latency or cross-platform runtime measurement.',
      workloads: [],
    };
    for (const workload of workloads()) {
      const reference = await upstream(workload.request);
      if (reference.sidecars.errors.length) throw new Error(`${workload.name}: upstream errors`);
      const run = async mode => {
        const result = mode === 'node-snapshot' ? await upstream(workload.request)
          : await rust.replay({ ...workload.request, mode: mode === 'rust-snapshot' ? 'snapshot' : 'patch' });
        assertEquivalent(reference, result, workload.name, mode);
        return result;
      };
      const modes = ['node-snapshot', 'rust-snapshot', 'rust-patch'];
      for (let warmup = 0; warmup < 2; warmup += 1) for (const mode of modes) await run(mode);
      const timings = Object.fromEntries(modes.map(mode => [mode, []]));
      const trials = [];
      const observations = {};
      for (let trial = 0; trial < 9; trial += 1) {
        // Balanced rotation: each mode occupies every position three times.
        const order = trialOrder(trial, modes);
        for (const mode of order) {
          const load = loadavg()[0];
          const result = await run(mode);
          timings[mode].push(result.elapsed_ms);
          trials.push({ trial, mode, elapsed_ms: result.elapsed_ms, load_1m: load });
          observations[mode] = { emissions: result.emissions,
            snapshots_materialized: result.snapshots_materialized ?? result.emissions };
        }
      }
      const summary = Object.fromEntries(modes.map(mode => [mode, distribution(timings[mode])]));
      const nodeMedian = summary['node-snapshot'].median_ms;
      report.workloads.push({ name: workload.name, description: workload.description,
        deltas: workload.deltas, wire_bytes: Buffer.byteLength(workload.request.raw_utf8),
        network_chunk_bytes: 4096, network_reads: Math.ceil(Buffer.byteLength(workload.request.raw_utf8) / 4096),
        initial_parts: workload.request.initialMessage?.parts.length ?? 0,
        equivalence: 'pass: foldedMessage, sidecars, emissions, every warmup and trial',
        observations, summary, speedup_from_node_medians: {
          rust_snapshot: nodeMedian / summary['rust-snapshot'].median_ms,
          rust_patch: nodeMedian / summary['rust-patch'].median_ms,
        }, trials });
    }
    report.environment.load_1m_end = loadavg()[0];
    await rust.close();
    const output = JSON.stringify(report, null, 2) + '\n';
    if (args['--out']) await writeFile(args['--out'], output, { flag: 'wx' });
    process.stdout.write(output);
  } catch (error) {
    rust.stop();
    // Synthetic workloads only; never dump requests or message content on failure.
    process.stderr.write(`Comparison failed: ${error.message}\n`);
    process.exitCode = 1;
  }
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await main(process.argv.slice(2));
}
